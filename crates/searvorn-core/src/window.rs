use crate::{
    error::{ErrorKind, Result, SearvornError},
    vfs::RandomRead,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowRead {
    pub offset: u64,
    pub read: usize,
    pub total_len: u64,
    pub eof: bool,
}

pub fn read_window_into<R>(
    reader: &mut R,
    offset: u64,
    buffer: &mut [u8],
) -> Result<WindowRead>
where
    R: RandomRead + ?Sized,
{
    let total_len = reader.len()?;

    if offset > total_len {
        return Err(SearvornError::new(
            ErrorKind::InvalidInput,
            "window.read",
        ));
    }

    let available = total_len - offset;
    let requested = buffer.len().min(available.min(usize::MAX as u64) as usize);
    let mut filled = 0usize;

    while filled < requested {
        let count = reader.read_at(offset + filled as u64, &mut buffer[filled..requested])?;
        if count == 0 {
            break;
        }

        filled += count;
    }

    Ok(WindowRead {
        offset,
        read: filled,
        total_len,
        eof: offset + filled as u64 >= total_len,
    })
}

pub fn read_window<R>(reader: &mut R, offset: u64, max_len: usize) -> Result<Vec<u8>>
where
    R: RandomRead + ?Sized,
{
    let total_len = reader.len()?;

    if offset > total_len {
        return Err(SearvornError::new(
            ErrorKind::InvalidInput,
            "window.read",
        ));
    }

    let remaining = total_len - offset;
    let size = max_len.min(remaining.min(usize::MAX as u64) as usize);
    let mut buffer = vec![0u8; size];
    let outcome = read_window_into(reader, offset, &mut buffer)?;
    buffer.truncate(outcome.read);
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::{read_window, read_window_into};
    use crate::{
        error::Result,
        vfs::RandomRead,
        ErrorKind,
    };

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

    #[test]
    fn fills_window_across_partial_reads() {
        let mut reader = PartialReader {
            bytes: b"0123456789".to_vec(),
            max_read: 2,
        };
        let mut buffer = [0u8; 5];

        let outcome = read_window_into(&mut reader, 3, &mut buffer).expect("window");

        assert_eq!(&buffer, b"34567");
        assert_eq!(outcome.offset, 3);
        assert_eq!(outcome.read, 5);
        assert_eq!(outcome.total_len, 10);
        assert!(!outcome.eof);
    }

    #[test]
    fn clips_window_at_end_of_file() {
        let mut reader = PartialReader {
            bytes: b"abcdefgh".to_vec(),
            max_read: 3,
        };
        let bytes = read_window(&mut reader, 6, 32).expect("window");

        assert_eq!(bytes, b"gh");
    }

    #[test]
    fn empty_window_at_eof_is_valid() {
        let mut reader = PartialReader {
            bytes: b"abc".to_vec(),
            max_read: 1,
        };
        let mut buffer = [0u8; 8];

        let outcome = read_window_into(&mut reader, 3, &mut buffer).expect("window");

        assert_eq!(outcome.read, 0);
        assert!(outcome.eof);
    }

    #[test]
    fn offset_after_eof_is_rejected() {
        let mut reader = PartialReader {
            bytes: b"abc".to_vec(),
            max_read: 1,
        };

        let error = read_window(&mut reader, 4, 8).expect_err("offset must fail");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}
