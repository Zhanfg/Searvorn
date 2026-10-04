use std::ops::Range;

use crate::{
    error::{ErrorKind, Result, SearvornError},
    vfs::RandomRead,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PieceSource {
    Original,
    Added,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Piece {
    source: PieceSource,
    offset: u64,
    len: u64,
}

#[derive(Debug)]
pub struct EditBuffer {
    original_len: u64,
    len: u64,
    pieces: Vec<Piece>,
    added: Vec<u8>,
}

impl EditBuffer {
    pub fn new(original_len: u64) -> Self {
        let mut pieces = Vec::new();
        if original_len != 0 {
            pieces.push(Piece {
                source: PieceSource::Original,
                offset: 0,
                len: original_len,
            });
        }

        Self {
            original_len,
            len: original_len,
            pieces,
            added: Vec::new(),
        }
    }

    pub const fn len(&self) -> u64 {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn original_len(&self) -> u64 {
        self.original_len
    }

    pub fn inserted_bytes(&self) -> usize {
        self.added.len()
    }

    pub fn piece_count(&self) -> usize {
        self.pieces.len()
    }

    pub fn insert(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        if offset > self.len {
            return Err(SearvornError::new(ErrorKind::InvalidInput, "edit.insert"));
        }

        if bytes.is_empty() {
            return Ok(());
        }

        let add_offset = self.added.len() as u64;
        self.added.extend_from_slice(bytes);
        let new_piece = Piece {
            source: PieceSource::Added,
            offset: add_offset,
            len: bytes.len() as u64,
        };

        let index = self.split_at(offset)?;
        self.pieces.insert(index, new_piece);
        self.len = self
            .len
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "edit.insert"))?;
        self.coalesce();
        Ok(())
    }

    pub fn delete(&mut self, range: Range<u64>) -> Result<()> {
        self.validate_range(&range)?;

        if range.is_empty() {
            return Ok(());
        }

        let start = self.split_at(range.start)?;
        let end = self.split_at(range.end)?;
        self.pieces.drain(start..end);
        self.len -= range.end - range.start;
        self.coalesce();
        Ok(())
    }

    pub fn replace(&mut self, range: Range<u64>, bytes: &[u8]) -> Result<()> {
        let start = range.start;
        self.delete(range)?;
        self.insert(start, bytes)
    }

    pub fn read_into<R>(&self, original: &mut R, offset: u64, buffer: &mut [u8]) -> Result<usize>
    where
        R: RandomRead + ?Sized,
    {
        if offset > self.len {
            return Err(SearvornError::new(ErrorKind::InvalidInput, "edit.read"));
        }

        let wanted = buffer
            .len()
            .min((self.len - offset).min(usize::MAX as u64) as usize);
        if wanted == 0 {
            return Ok(0);
        }

        let mut logical = 0u64;
        let mut written = 0usize;

        for piece in &self.pieces {
            let piece_end = logical + piece.len;
            if piece_end <= offset {
                logical = piece_end;
                continue;
            }

            if logical >= offset + wanted as u64 {
                break;
            }

            let start_in_piece = offset.saturating_sub(logical);
            let available = piece.len - start_in_piece;
            let count = available.min((wanted - written) as u64) as usize;
            let source_offset = piece.offset + start_in_piece;

            match piece.source {
                PieceSource::Original => {
                    read_exact_at(original, source_offset, &mut buffer[written..written + count])?;
                }
                PieceSource::Added => {
                    let start = usize::try_from(source_offset)
                        .map_err(|_| SearvornError::new(ErrorKind::Unsupported, "edit.read"))?;
                    let end = start
                        .checked_add(count)
                        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "edit.read"))?;
                    let added = self
                        .added
                        .get(start..end)
                        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "edit.read"))?;
                    buffer[written..written + count].copy_from_slice(added);
                }
            }

            written += count;
            if written == wanted {
                break;
            }

            logical = piece_end;
        }

        Ok(written)
    }

    fn validate_range(&self, range: &Range<u64>) -> Result<()> {
        if range.start > range.end || range.end > self.len {
            Err(SearvornError::new(
                ErrorKind::InvalidInput,
                "edit.range",
            ))
        } else {
            Ok(())
        }
    }

    fn split_at(&mut self, logical_offset: u64) -> Result<usize> {
        if logical_offset > self.len {
            return Err(SearvornError::new(ErrorKind::InvalidInput, "edit.split"));
        }

        if logical_offset == self.len {
            return Ok(self.pieces.len());
        }

        let mut logical = 0u64;

        for index in 0..self.pieces.len() {
            let piece = self.pieces[index];
            let end = logical + piece.len;

            if logical_offset == logical {
                return Ok(index);
            }

            if logical_offset < end {
                let left_len = logical_offset - logical;
                let right_len = piece.len - left_len;

                self.pieces[index].len = left_len;
                self.pieces.insert(
                    index + 1,
                    Piece {
                        source: piece.source,
                        offset: piece.offset + left_len,
                        len: right_len,
                    },
                );
                return Ok(index + 1);
            }

            logical = end;
        }

        Ok(self.pieces.len())
    }

    fn coalesce(&mut self) {
        let mut write = 0usize;

        for read in 0..self.pieces.len() {
            let piece = self.pieces[read];
            if piece.len == 0 {
                continue;
            }

            if write != 0 {
                let previous = self.pieces[write - 1];
                if previous.source == piece.source
                    && previous.offset + previous.len == piece.offset
                {
                    self.pieces[write - 1].len += piece.len;
                    continue;
                }
            }

            self.pieces[write] = piece;
            write += 1;
        }

        self.pieces.truncate(write);
    }
}

fn read_exact_at<R>(reader: &mut R, offset: u64, buffer: &mut [u8]) -> Result<()>
where
    R: RandomRead + ?Sized,
{
    let mut filled = 0usize;

    while filled < buffer.len() {
        let count = reader.read_at(offset + filled as u64, &mut buffer[filled..])?;
        if count == 0 {
            return Err(SearvornError::with_detail(
                ErrorKind::Io,
                "edit.read",
                "source ended before expected length",
            ));
        }
        filled += count;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::EditBuffer;
    use crate::{error::Result, vfs::RandomRead, ErrorKind};

    struct PartialReader {
        bytes: Vec<u8>,
        max_read: usize,
    }

    impl RandomRead for PartialReader {
        fn len(&self) -> Result<u64> {
            Ok(self.bytes.len() as u64)
        }

        fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
            let offset = offset as usize;
            if offset >= self.bytes.len() {
                return Ok(0);
            }

            let count = buffer
                .len()
                .min(self.max_read)
                .min(self.bytes.len() - offset);
            buffer[..count].copy_from_slice(&self.bytes[offset..offset + count]);
            Ok(count)
        }
    }

    fn materialize(buffer: &EditBuffer, original: &[u8]) -> Vec<u8> {
        let mut reader = PartialReader {
            bytes: original.to_vec(),
            max_read: 2,
        };
        let mut out = vec![0u8; buffer.len() as usize];
        let read = buffer
            .read_into(&mut reader, 0, &mut out)
            .expect("materialize");
        out.truncate(read);
        out
    }

    #[test]
    fn empty_buffer_has_no_piece() {
        let buffer = EditBuffer::new(0);
        assert!(buffer.is_empty());
        assert_eq!(buffer.piece_count(), 0);
    }

    #[test]
    fn insert_does_not_copy_original() {
        let mut buffer = EditBuffer::new(1_000_000_000);
        buffer.insert(500_000_000, b"abc").expect("insert");

        assert_eq!(buffer.len(), 1_000_000_003);
        assert_eq!(buffer.inserted_bytes(), 3);
        assert_eq!(buffer.piece_count(), 3);
    }

    #[test]
    fn insert_delete_and_replace_materialize_correctly() {
        let original = b"abcdef";
        let mut buffer = EditBuffer::new(original.len() as u64);

        buffer.insert(3, b"XYZ").expect("insert");
        assert_eq!(materialize(&buffer, original), b"abcXYZdef");

        buffer.delete(1..5).expect("delete");
        assert_eq!(materialize(&buffer, original), b"aZdef");

        buffer.replace(1..2, b"123").expect("replace");
        assert_eq!(materialize(&buffer, original), b"a123def");
    }

    #[test]
    fn read_supports_windows_crossing_piece_boundaries() {
        let original = b"abcdefgh";
        let mut buffer = EditBuffer::new(original.len() as u64);
        buffer.insert(4, b"XYZ").expect("insert");

        let mut reader = PartialReader {
            bytes: original.to_vec(),
            max_read: 1,
        };
        let mut out = [0u8; 6];
        let read = buffer.read_into(&mut reader, 2, &mut out).expect("read");

        assert_eq!(read, 6);
        assert_eq!(&out, b"cdXYZe");
    }

    #[test]
    fn rejects_invalid_ranges() {
        let mut buffer = EditBuffer::new(3);
        let error = buffer.delete(2..4).expect_err("invalid range");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);

        let error = buffer.insert(4, b"x").expect_err("invalid offset");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}
