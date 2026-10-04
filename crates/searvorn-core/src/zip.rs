use crate::{
    error::{ErrorKind, Result, SearvornError},
    vfs::RandomRead,
    window::read_window,
};

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const EOCD_MIN_LEN: usize = 22;
const MAX_COMMENT_LEN: usize = u16::MAX as usize;
const CENTRAL_FIXED_LEN: usize = 46;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZipSummary {
    pub entries: u64,
    pub central_directory_offset: u64,
    pub central_directory_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZipEntry {
    pub name: Vec<u8>,
    pub flags: u16,
    pub compression_method: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub local_header_offset: u64,
}

impl ZipEntry {
    pub fn display_name(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }

    pub const fn is_utf8_name(&self) -> bool {
        self.flags & (1 << 11) != 0
    }

    pub const fn is_stored(&self) -> bool {
        self.compression_method == 0
    }
}

pub fn scan_zip<R, F>(reader: &mut R, mut visitor: F) -> Result<ZipSummary>
where
    R: RandomRead + ?Sized,
    F: FnMut(&ZipEntry) -> Result<()>,
{
    let total_len = reader.len()?;
    let tail_len = (EOCD_MIN_LEN + MAX_COMMENT_LEN) as u64;
    let tail_start = total_len.saturating_sub(tail_len);
    let tail = read_window(
        reader,
        tail_start,
        (total_len - tail_start)
            .try_into()
            .map_err(|_| SearvornError::new(ErrorKind::Unsupported, "zip.scan"))?,
    )?;

    let eocd_index = find_eocd(&tail)?;
    let eocd = &tail[eocd_index..];
    let disk_number = u16_at(eocd, 4)?;
    let directory_disk = u16_at(eocd, 6)?;
    let entries_on_disk = u16_at(eocd, 8)?;
    let entries = u16_at(eocd, 10)?;
    let directory_size = u32_at(eocd, 12)?;
    let directory_offset = u32_at(eocd, 16)?;

    if disk_number != 0 || directory_disk != 0 || entries_on_disk != entries {
        return Err(SearvornError::new(ErrorKind::Unsupported, "zip.multidisk"));
    }

    if entries == u16::MAX || directory_size == u32::MAX || directory_offset == u32::MAX {
        return Err(SearvornError::new(ErrorKind::Unsupported, "zip.zip64"));
    }

    let directory_offset = u64::from(directory_offset);
    let directory_size = u64::from(directory_size);
    let directory_end = directory_offset
        .checked_add(directory_size)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "zip.directory"))?;

    if directory_end > total_len {
        return Err(SearvornError::with_detail(
            ErrorKind::InvalidInput,
            "zip.directory",
            "central directory exceeds file length",
        ));
    }

    let mut cursor = directory_offset;

    for _ in 0..entries {
        let mut fixed = [0u8; CENTRAL_FIXED_LEN];
        read_exact_at(reader, cursor, &mut fixed)?;

        if u32_at(&fixed, 0)? != CENTRAL_SIGNATURE {
            return Err(SearvornError::with_detail(
                ErrorKind::InvalidInput,
                "zip.central",
                "invalid central directory signature",
            ));
        }

        let flags = u16_at(&fixed, 8)?;
        let compression_method = u16_at(&fixed, 10)?;
        let crc32 = u32_at(&fixed, 16)?;
        let compressed_size = u32_at(&fixed, 20)?;
        let uncompressed_size = u32_at(&fixed, 24)?;
        let name_len = usize::from(u16_at(&fixed, 28)?);
        let extra_len = u64::from(u16_at(&fixed, 30)?);
        let comment_len = u64::from(u16_at(&fixed, 32)?);
        let disk_start = u16_at(&fixed, 34)?;
        let local_header_offset = u32_at(&fixed, 42)?;

        if compressed_size == u32::MAX
            || uncompressed_size == u32::MAX
            || local_header_offset == u32::MAX
            || disk_start == u16::MAX
        {
            return Err(SearvornError::new(ErrorKind::Unsupported, "zip.zip64"));
        }

        if disk_start != 0 {
            return Err(SearvornError::new(ErrorKind::Unsupported, "zip.multidisk"));
        }

        let name_offset = cursor
            .checked_add(CENTRAL_FIXED_LEN as u64)
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "zip.central"))?;
        let mut name = vec![0u8; name_len];
        read_exact_at(reader, name_offset, &mut name)?;

        let record_len = (CENTRAL_FIXED_LEN as u64)
            .checked_add(name_len as u64)
            .and_then(|value| value.checked_add(extra_len))
            .and_then(|value| value.checked_add(comment_len))
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "zip.central"))?;
        cursor = cursor
            .checked_add(record_len)
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "zip.central"))?;

        if cursor > directory_end {
            return Err(SearvornError::with_detail(
                ErrorKind::InvalidInput,
                "zip.central",
                "entry exceeds central directory bounds",
            ));
        }

        visitor(&ZipEntry {
            name,
            flags,
            compression_method,
            crc32,
            compressed_size: u64::from(compressed_size),
            uncompressed_size: u64::from(uncompressed_size),
            local_header_offset: u64::from(local_header_offset),
        })?;
    }

    Ok(ZipSummary {
        entries: u64::from(entries),
        central_directory_offset: directory_offset,
        central_directory_size: directory_size,
    })
}

fn find_eocd(tail: &[u8]) -> Result<usize> {
    if tail.len() < EOCD_MIN_LEN {
        return Err(SearvornError::new(ErrorKind::InvalidInput, "zip.eocd"));
    }

    for index in (0..=tail.len() - EOCD_MIN_LEN).rev() {
        if u32::from_le_bytes([
            tail[index],
            tail[index + 1],
            tail[index + 2],
            tail[index + 3],
        ]) != EOCD_SIGNATURE
        {
            continue;
        }

        let comment_len = usize::from(u16::from_le_bytes([
            tail[index + 20],
            tail[index + 21],
        ]));

        if index + EOCD_MIN_LEN + comment_len == tail.len() {
            return Ok(index);
        }
    }

    Err(SearvornError::with_detail(
        ErrorKind::InvalidInput,
        "zip.eocd",
        "end of central directory not found",
    ))
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
                ErrorKind::InvalidInput,
                "zip.read",
                "unexpected end of file",
            ));
        }

        filled += count;
    }

    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "zip.decode"))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "zip.decode"))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

#[cfg(test)]
mod tests {
    use super::{scan_zip, ZipEntry};
    use crate::{
        error::Result,
        vfs::RandomRead,
        ErrorKind,
    };

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

    fn one_file_zip() -> Vec<u8> {
        let name = b"a.txt";
        let data = b"hello";
        let mut bytes = Vec::new();

        bytes.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&20u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(data);

        let central_offset = bytes.len() as u32;
        bytes.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&20u16.to_le_bytes());
        bytes.extend_from_slice(&20u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(name);
        let central_size = bytes.len() as u32 - central_offset;

        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&central_size.to_le_bytes());
        bytes.extend_from_slice(&central_offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        bytes
    }

    #[test]
    fn scans_central_directory_with_partial_reads() {
        let mut reader = MemoryReader {
            bytes: one_file_zip(),
            max_read: 7,
        };
        let mut entries = Vec::<ZipEntry>::new();

        let summary = scan_zip(&mut reader, |entry| {
            entries.push(entry.clone());
            Ok(())
        })
        .expect("scan");

        assert_eq!(summary.entries, 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, b"a.txt");
        assert_eq!(entries[0].compressed_size, 5);
        assert_eq!(entries[0].uncompressed_size, 5);
        assert!(entries[0].is_stored());
    }

    #[test]
    fn rejects_missing_eocd() {
        let mut reader = MemoryReader {
            bytes: vec![0; 64],
            max_read: 64,
        };

        let error = scan_zip(&mut reader, |_| Ok(())).expect_err("invalid zip");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_central_directory_outside_file() {
        let mut bytes = one_file_zip();
        let len = bytes.len();
        let eocd = len - 22;
        bytes[eocd + 16..eocd + 20].copy_from_slice(&u32::MAX.saturating_sub(1).to_le_bytes());

        let mut reader = MemoryReader {
            bytes,
            max_read: 64,
        };

        let error = scan_zip(&mut reader, |_| Ok(())).expect_err("invalid bounds");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}
