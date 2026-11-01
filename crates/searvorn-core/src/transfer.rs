use crate::{
    error::{ErrorKind, Result, SearvornError},
    task::CancellationFlag,
    vfs::{RandomRead, RandomWrite},
};

pub const DEFAULT_COPY_CHUNK: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CopyOptions {
    pub chunk_size: usize,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_COPY_CHUNK,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CopyProgress {
    pub copied: u64,
    pub total_hint: u64,
}

pub fn copy_stream<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    options: CopyOptions,
    cancellation: Option<&CancellationFlag>,
    mut progress: F,
) -> Result<u64>
where
    R: RandomRead + ?Sized,
    W: RandomWrite + ?Sized,
    F: FnMut(CopyProgress),
{
    if options.chunk_size == 0 {
        return Err(SearvornError::new(ErrorKind::InvalidInput, "transfer.copy"));
    }

    let total_hint = reader.len()?;
    let mut buffer = vec![0u8; options.chunk_size];
    let mut offset = 0u64;

    loop {
        check_cancelled(cancellation)?;

        let read = reader.read_at(offset, &mut buffer)?;
        if read == 0 {
            break;
        }

        let mut written = 0usize;
        while written < read {
            check_cancelled(cancellation)?;

            let count = writer.write_at(offset + written as u64, &buffer[written..read])?;
            if count == 0 {
                return Err(SearvornError::with_detail(
                    ErrorKind::Io,
                    "transfer.copy",
                    "writer made no progress",
                ));
            }
            written += count;
        }

        offset += read as u64;
        progress(CopyProgress {
            copied: offset,
            total_hint,
        });
    }

    writer.set_len(offset)?;
    writer.flush()?;
    Ok(offset)
}

fn check_cancelled(cancellation: Option<&CancellationFlag>) -> Result<()> {
    if cancellation.is_some_and(CancellationFlag::is_cancelled) {
        Err(SearvornError::new(ErrorKind::Interrupted, "transfer.copy"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{copy_stream, CopyOptions};
    use crate::{
        error::{ErrorKind, Result},
        task::CancellationFlag,
        vfs::{RandomRead, RandomWrite},
    };

    struct MemoryReader {
        data: Vec<u8>,
        max_read: usize,
    }

    impl RandomRead for MemoryReader {
        fn len(&self) -> Result<u64> {
            Ok(self.data.len() as u64)
        }

        fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
            let offset = offset as usize;
            if offset >= self.data.len() {
                return Ok(0);
            }

            let count = self
                .max_read
                .min(buffer.len())
                .min(self.data.len() - offset);
            buffer[..count].copy_from_slice(&self.data[offset..offset + count]);
            Ok(count)
        }
    }

    struct MemoryWriter {
        data: Vec<u8>,
        max_write: usize,
    }

    impl RandomWrite for MemoryWriter {
        fn len(&self) -> Result<u64> {
            Ok(self.data.len() as u64)
        }

        fn write_at(&mut self, offset: u64, buffer: &[u8]) -> Result<usize> {
            let count = self.max_write.min(buffer.len());
            let offset = offset as usize;
            let end = offset + count;

            if self.data.len() < end {
                self.data.resize(end, 0);
            }
            self.data[offset..end].copy_from_slice(&buffer[..count]);
            Ok(count)
        }

        fn set_len(&mut self, len: u64) -> Result<()> {
            self.data.resize(len as usize, 0);
            Ok(())
        }

        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn handles_partial_reads_and_writes() {
        let input = b"0123456789abcdef".to_vec();
        let mut reader = MemoryReader {
            data: input.clone(),
            max_read: 3,
        };
        let mut writer = MemoryWriter {
            data: Vec::new(),
            max_write: 2,
        };
        let mut last_progress = 0;

        let copied = copy_stream(
            &mut reader,
            &mut writer,
            CopyOptions { chunk_size: 5 },
            None,
            |progress| last_progress = progress.copied,
        )
        .expect("copy");

        assert_eq!(copied, input.len() as u64);
        assert_eq!(last_progress, input.len() as u64);
        assert_eq!(writer.data, input);
    }

    #[test]
    fn cancellation_stops_copy() {
        let cancellation = CancellationFlag::new();
        let mut reader = MemoryReader {
            data: vec![7; 32],
            max_read: 4,
        };
        let mut writer = MemoryWriter {
            data: Vec::new(),
            max_write: 4,
        };

        let error = copy_stream(
            &mut reader,
            &mut writer,
            CopyOptions { chunk_size: 4 },
            Some(&cancellation),
            |progress| {
                if progress.copied >= 8 {
                    cancellation.cancel();
                }
            },
        )
        .expect_err("copy should be cancelled");

        assert_eq!(error.kind(), ErrorKind::Interrupted);
        assert_eq!(writer.data.len(), 8);
    }

    #[test]
    fn rejects_empty_buffer() {
        let mut reader = MemoryReader {
            data: vec![1],
            max_read: 1,
        };
        let mut writer = MemoryWriter {
            data: Vec::new(),
            max_write: 1,
        };

        let error = copy_stream(
            &mut reader,
            &mut writer,
            CopyOptions { chunk_size: 0 },
            None,
            |_| {},
        )
        .expect_err("zero chunk must fail");

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}
