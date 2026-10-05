use crate::{
    error::{ErrorKind, Result, SearvornError},
    vfs::RandomRead,
};

pub struct ReadSlice<'a, R>
where
    R: RandomRead + ?Sized,
{
    source: &'a mut R,
    base: u64,
    len: u64,
}

impl<'a, R> ReadSlice<'a, R>
where
    R: RandomRead + ?Sized,
{
    pub fn new(source: &'a mut R, base: u64, len: u64) -> Result<Self> {
        let source_len = source.len()?;
        let end = base
            .checked_add(len)
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "slice.new"))?;

        if end > source_len {
            return Err(SearvornError::with_detail(
                ErrorKind::InvalidInput,
                "slice.new",
                "slice exceeds source length",
            ));
        }

        Ok(Self { source, base, len })
    }

    pub const fn base(&self) -> u64 {
        self.base
    }

    pub const fn slice_len(&self) -> u64 {
        self.len
    }
}

impl<R> RandomRead for ReadSlice<'_, R>
where
    R: RandomRead + ?Sized,
{
    fn len(&self) -> Result<u64> {
        Ok(self.len)
    }

    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if offset >= self.len || buffer.is_empty() {
            return Ok(0);
        }

        let remaining = self.len - offset;
        let requested = buffer.len().min(remaining.min(usize::MAX as u64) as usize);
        let source_offset = self
            .base
            .checked_add(offset)
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "slice.read"))?;

        self.source.read_at(source_offset, &mut buffer[..requested])
    }
}

#[cfg(test)]
mod tests {
    use super::ReadSlice;
    use crate::{error::Result, vfs::RandomRead, ErrorKind};

    struct MemoryReader(Vec<u8>);

    impl RandomRead for MemoryReader {
        fn len(&self) -> Result<u64> {
            Ok(self.0.len() as u64)
        }

        fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
            let offset = offset as usize;
            if offset >= self.0.len() {
                return Ok(0);
            }

            let count = buffer.len().min(self.0.len() - offset);
            buffer[..count].copy_from_slice(&self.0[offset..offset + count]);
            Ok(count)
        }
    }

    #[test]
    fn clamps_reads_to_slice_bounds() {
        let mut source = MemoryReader(b"0123456789".to_vec());
        let mut slice = ReadSlice::new(&mut source, 3, 4).expect("slice");
        let mut buffer = [0u8; 8];

        let read = slice.read_at(1, &mut buffer).expect("read");

        assert_eq!(read, 3);
        assert_eq!(&buffer[..read], b"456");
        assert_eq!(slice.len().expect("len"), 4);
        assert_eq!(slice.base(), 3);
    }

    #[test]
    fn rejects_slice_outside_source() {
        let mut source = MemoryReader(vec![0; 8]);
        let error = match ReadSlice::new(&mut source, 7, 2) {
            Ok(_) => panic!("slice must be rejected"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}
