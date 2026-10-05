use crate::{
    error::{ErrorKind, Result, SearvornError},
    task::CancellationFlag,
    vfs::RandomRead,
};

pub const DEFAULT_LINE_STRIDE: u32 = 256;
pub const DEFAULT_SCAN_CHUNK: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineCheckpoint {
    pub line: u64,
    pub offset: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineScanProgress {
    pub scanned_bytes: u64,
    pub scanned_to: u64,
    pub next_line: u64,
    pub complete: bool,
}

#[derive(Debug)]
pub struct SparseLineIndex {
    total_len: u64,
    stride: u32,
    scanned_to: u64,
    next_line: u64,
    complete: bool,
    checkpoints: Vec<LineCheckpoint>,
}

impl SparseLineIndex {
    pub fn new(total_len: u64, stride: u32) -> Result<Self> {
        if stride == 0 {
            return Err(SearvornError::new(
                ErrorKind::InvalidInput,
                "line_index.new",
            ));
        }

        Ok(Self {
            total_len,
            stride,
            scanned_to: 0,
            next_line: 0,
            complete: total_len == 0,
            checkpoints: vec![LineCheckpoint { line: 0, offset: 0 }],
        })
    }

    pub fn with_default_stride(total_len: u64) -> Self {
        Self::new(total_len, DEFAULT_LINE_STRIDE).expect("default stride is non-zero")
    }

    pub const fn total_len(&self) -> u64 {
        self.total_len
    }

    pub const fn scanned_to(&self) -> u64 {
        self.scanned_to
    }

    pub const fn complete(&self) -> bool {
        self.complete
    }

    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    pub fn checkpoints(&self) -> &[LineCheckpoint] {
        &self.checkpoints
    }

    pub fn advance<R>(
        &mut self,
        reader: &mut R,
        max_bytes: u64,
        cancellation: Option<&CancellationFlag>,
    ) -> Result<LineScanProgress>
    where
        R: RandomRead + ?Sized,
    {
        if reader.len()? != self.total_len {
            return Err(SearvornError::with_detail(
                ErrorKind::Conflict,
                "line_index.advance",
                "source length changed",
            ));
        }

        if self.complete || max_bytes == 0 {
            return Ok(LineScanProgress {
                scanned_bytes: 0,
                scanned_to: self.scanned_to,
                next_line: self.next_line,
                complete: self.complete,
            });
        }

        let target = self
            .scanned_to
            .saturating_add(max_bytes)
            .min(self.total_len);
        let mut scratch = vec![0u8; DEFAULT_SCAN_CHUNK];
        let start = self.scanned_to;

        while self.scanned_to < target {
            check_cancelled(cancellation)?;

            let remaining = target - self.scanned_to;
            let requested = scratch.len().min(remaining.min(usize::MAX as u64) as usize);
            let read = reader.read_at(self.scanned_to, &mut scratch[..requested])?;

            if read == 0 {
                return Err(SearvornError::with_detail(
                    ErrorKind::Io,
                    "line_index.advance",
                    "source ended before reported length",
                ));
            }

            let chunk_start = self.scanned_to;

            for (index, byte) in scratch[..read].iter().enumerate() {
                if *byte != b'\n' {
                    continue;
                }

                self.next_line = self.next_line.saturating_add(1);
                if self.next_line.is_multiple_of(u64::from(self.stride)) {
                    self.checkpoints.push(LineCheckpoint {
                        line: self.next_line,
                        offset: chunk_start + index as u64 + 1,
                    });
                }
            }

            self.scanned_to += read as u64;
        }

        self.complete = self.scanned_to == self.total_len;

        Ok(LineScanProgress {
            scanned_bytes: self.scanned_to - start,
            scanned_to: self.scanned_to,
            next_line: self.next_line,
            complete: self.complete,
        })
    }

    pub fn line_start<R>(&self, reader: &mut R, line: u64) -> Result<Option<u64>>
    where
        R: RandomRead + ?Sized,
    {
        if line == 0 {
            return Ok(Some(0));
        }

        if line > self.next_line {
            return Ok(None);
        }

        let checkpoint = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.line <= line)
            .checked_sub(1)
            .and_then(|index| self.checkpoints.get(index))
            .copied()
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "line_index.lookup"))?;

        if checkpoint.line == line {
            return Ok(Some(checkpoint.offset));
        }

        let mut current_line = checkpoint.line;
        let mut offset = checkpoint.offset;
        let mut scratch = vec![0u8; DEFAULT_SCAN_CHUNK];

        while offset < self.scanned_to {
            let remaining = self.scanned_to - offset;
            let requested = scratch.len().min(remaining.min(usize::MAX as u64) as usize);
            let read = reader.read_at(offset, &mut scratch[..requested])?;

            if read == 0 {
                return Err(SearvornError::with_detail(
                    ErrorKind::Io,
                    "line_index.lookup",
                    "source ended before indexed range",
                ));
            }

            for (index, byte) in scratch[..read].iter().enumerate() {
                if *byte != b'\n' {
                    continue;
                }

                current_line += 1;
                if current_line == line {
                    return Ok(Some(offset + index as u64 + 1));
                }
            }

            offset += read as u64;
        }

        Ok(None)
    }

    pub fn line_count_if_complete(&self) -> Option<u64> {
        if !self.complete {
            return None;
        }

        if self.total_len == 0 {
            Some(1)
        } else {
            Some(self.next_line.saturating_add(1))
        }
    }
}

fn check_cancelled(cancellation: Option<&CancellationFlag>) -> Result<()> {
    if cancellation.is_some_and(CancellationFlag::is_cancelled) {
        Err(SearvornError::new(
            ErrorKind::Interrupted,
            "line_index.advance",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SparseLineIndex;
    use crate::{error::Result, task::CancellationFlag, vfs::RandomRead, ErrorKind};

    struct MemoryReader {
        bytes: Vec<u8>,
        max_read: usize,
    }

    impl RandomRead for MemoryReader {
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
    fn indexes_incrementally_and_resolves_lines() {
        let data = b"a\nbb\nccc\ndddd\neeeee".to_vec();
        let mut reader = MemoryReader {
            bytes: data.clone(),
            max_read: 3,
        };
        let mut index = SparseLineIndex::new(data.len() as u64, 2).expect("index");

        let first = index.advance(&mut reader, 5, None).expect("first scan");
        assert_eq!(first.scanned_bytes, 5);
        assert!(!first.complete);
        assert_eq!(index.line_start(&mut reader, 0).expect("line"), Some(0));
        assert_eq!(index.line_start(&mut reader, 1).expect("line"), Some(2));
        assert_eq!(index.line_start(&mut reader, 2).expect("line"), Some(5));
        assert_eq!(index.line_start(&mut reader, 3).expect("line"), None);

        let second = index.advance(&mut reader, u64::MAX, None).expect("finish");
        assert!(second.complete);
        assert_eq!(index.line_start(&mut reader, 3).expect("line"), Some(9));
        assert_eq!(index.line_start(&mut reader, 4).expect("line"), Some(14));
        assert_eq!(index.line_count_if_complete(), Some(5));
        assert_eq!(index.checkpoints().len(), 3);
    }

    #[test]
    fn empty_file_has_one_logical_line() {
        let mut reader = MemoryReader {
            bytes: Vec::new(),
            max_read: 4,
        };
        let index = SparseLineIndex::with_default_stride(0);

        assert!(index.complete());
        assert_eq!(index.line_count_if_complete(), Some(1));
        assert_eq!(index.line_start(&mut reader, 0).expect("line"), Some(0));
    }

    #[test]
    fn source_length_change_is_detected() {
        let mut reader = MemoryReader {
            bytes: b"a\nb".to_vec(),
            max_read: 4,
        };
        let mut index = SparseLineIndex::with_default_stride(3);
        reader.bytes.push(b'c');

        let error = index
            .advance(&mut reader, 4, None)
            .expect_err("must conflict");
        assert_eq!(error.kind(), ErrorKind::Conflict);
    }

    #[test]
    fn cancellation_stops_incremental_scan() {
        let mut reader = MemoryReader {
            bytes: vec![b'x'; 200_000],
            max_read: 4096,
        };
        let mut index = SparseLineIndex::with_default_stride(200_000);
        let cancellation = CancellationFlag::new();
        cancellation.cancel();

        let error = index
            .advance(&mut reader, 100_000, Some(&cancellation))
            .expect_err("must cancel");

        assert_eq!(error.kind(), ErrorKind::Interrupted);
        assert_eq!(index.scanned_to(), 0);
    }

    #[test]
    fn rejects_zero_stride() {
        let error = SparseLineIndex::new(10, 0).expect_err("zero stride");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}
