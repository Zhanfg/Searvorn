use crate::{
    error::{ErrorKind, Result, SearvornError},
    vfs::RandomRead,
};

const MAGIC: [u8; 16] = *b"APK Sig Block 42";
const FOOTER_LEN: u64 = 24;
const LEADING_SIZE_LEN: u64 = 8;

pub const V2_BLOCK_ID: u32 = 0x7109_871a;
pub const V3_BLOCK_ID: u32 = 0xf053_68c0;
pub const V31_BLOCK_ID: u32 = 0x1b93_ad61;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ApkSigningBlockSummary {
    pub present: bool,
    pub block_offset: u64,
    pub block_size: u64,
    pub pair_count: u32,
    pub has_v2: bool,
    pub has_v3: bool,
    pub has_v31: bool,
}

pub fn inspect_signing_block<R>(
    reader: &mut R,
    central_directory_offset: u64,
) -> Result<ApkSigningBlockSummary>
where
    R: RandomRead + ?Sized,
{
    if central_directory_offset < FOOTER_LEN {
        return Ok(ApkSigningBlockSummary::default());
    }

    let footer_offset = central_directory_offset - FOOTER_LEN;
    let mut footer = [0u8; FOOTER_LEN as usize];
    read_exact_at(reader, footer_offset, &mut footer)?;

    if footer[8..24] != MAGIC {
        return Ok(ApkSigningBlockSummary::default());
    }

    let size = u64::from_le_bytes(
        footer[..8]
            .try_into()
            .expect("APK signing footer size has fixed width"),
    );

    if size < FOOTER_LEN {
        return Err(SearvornError::with_detail(
            ErrorKind::InvalidInput,
            "apk_signing.block",
            "signing block size is too small",
        ));
    }

    let total_size = size
        .checked_add(LEADING_SIZE_LEN)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "apk_signing.block"))?;
    let block_offset = central_directory_offset
        .checked_sub(total_size)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "apk_signing.block"))?;

    let mut leading_size = [0u8; 8];
    read_exact_at(reader, block_offset, &mut leading_size)?;
    if u64::from_le_bytes(leading_size) != size {
        return Err(SearvornError::with_detail(
            ErrorKind::InvalidInput,
            "apk_signing.block",
            "leading and trailing signing block sizes differ",
        ));
    }

    let pairs_start = block_offset + LEADING_SIZE_LEN;
    let pairs_end = footer_offset;
    let mut cursor = pairs_start;
    let mut summary = ApkSigningBlockSummary {
        present: true,
        block_offset,
        block_size: total_size,
        ..ApkSigningBlockSummary::default()
    };

    while cursor < pairs_end {
        let remaining = pairs_end - cursor;
        if remaining < 12 {
            return Err(SearvornError::with_detail(
                ErrorKind::InvalidInput,
                "apk_signing.pair",
                "truncated signing block pair",
            ));
        }

        let mut header = [0u8; 12];
        read_exact_at(reader, cursor, &mut header)?;
        let pair_len =
            u64::from_le_bytes(header[..8].try_into().expect("pair size has fixed width"));

        if pair_len < 4 {
            return Err(SearvornError::with_detail(
                ErrorKind::InvalidInput,
                "apk_signing.pair",
                "pair length is smaller than ID",
            ));
        }

        let id = u32::from_le_bytes(header[8..12].try_into().expect("pair ID has fixed width"));
        let next = cursor
            .checked_add(8)
            .and_then(|offset| offset.checked_add(pair_len))
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "apk_signing.pair"))?;

        if next > pairs_end {
            return Err(SearvornError::with_detail(
                ErrorKind::InvalidInput,
                "apk_signing.pair",
                "pair exceeds signing block bounds",
            ));
        }

        summary.pair_count = summary.pair_count.saturating_add(1);
        match id {
            V2_BLOCK_ID => summary.has_v2 = true,
            V3_BLOCK_ID => summary.has_v3 = true,
            V31_BLOCK_ID => summary.has_v31 = true,
            _ => {}
        }

        cursor = next;
    }

    if cursor != pairs_end {
        return Err(SearvornError::new(
            ErrorKind::InvalidInput,
            "apk_signing.block",
        ));
    }

    Ok(summary)
}

fn read_exact_at<R>(reader: &mut R, offset: u64, buffer: &mut [u8]) -> Result<()>
where
    R: RandomRead + ?Sized,
{
    let mut filled = 0usize;

    while filled < buffer.len() {
        let read = reader.read_at(offset + filled as u64, &mut buffer[filled..])?;
        if read == 0 {
            return Err(SearvornError::with_detail(
                ErrorKind::InvalidInput,
                "apk_signing.read",
                "unexpected end of file",
            ));
        }

        filled += read;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{inspect_signing_block, V2_BLOCK_ID, V31_BLOCK_ID, V3_BLOCK_ID};
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

            let read = buffer.len().min(self.0.len() - offset);
            buffer[..read].copy_from_slice(&self.0[offset..offset + read]);
            Ok(read)
        }
    }

    fn signing_block(ids: &[u32]) -> Vec<u8> {
        let mut pairs = Vec::new();

        for id in ids {
            pairs.extend_from_slice(&4u64.to_le_bytes());
            pairs.extend_from_slice(&id.to_le_bytes());
        }

        let size = pairs.len() as u64 + 24;
        let mut block = Vec::new();
        block.extend_from_slice(&size.to_le_bytes());
        block.extend_from_slice(&pairs);
        block.extend_from_slice(&size.to_le_bytes());
        block.extend_from_slice(b"APK Sig Block 42");
        block
    }

    #[test]
    fn detects_v2_v3_and_v31_ids_without_parsing_values() {
        let bytes = signing_block(&[V2_BLOCK_ID, V3_BLOCK_ID, V31_BLOCK_ID, 0x1234_5678]);
        let central_offset = bytes.len() as u64;
        let mut reader = MemoryReader(bytes);

        let summary = inspect_signing_block(&mut reader, central_offset).expect("inspect");

        assert!(summary.present);
        assert!(summary.has_v2);
        assert!(summary.has_v3);
        assert!(summary.has_v31);
        assert_eq!(summary.pair_count, 4);
        assert_eq!(summary.block_offset, 0);
        assert_eq!(summary.block_size, central_offset);
    }

    #[test]
    fn absence_of_magic_means_no_v2_plus_signing_block() {
        let mut reader = MemoryReader(vec![0; 64]);
        let summary = inspect_signing_block(&mut reader, 64).expect("inspect");

        assert!(!summary.present);
    }

    #[test]
    fn mismatched_size_fields_are_rejected() {
        let mut bytes = signing_block(&[V2_BLOCK_ID]);
        let len = bytes.len();
        bytes[..8].copy_from_slice(&999u64.to_le_bytes());
        let mut reader = MemoryReader(bytes);

        let error = inspect_signing_block(&mut reader, len as u64).expect_err("invalid block");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn pair_crossing_footer_is_rejected() {
        let mut bytes = signing_block(&[V2_BLOCK_ID]);
        bytes[8..16].copy_from_slice(&1000u64.to_le_bytes());
        let len = bytes.len();
        let mut reader = MemoryReader(bytes);

        let error = inspect_signing_block(&mut reader, len as u64).expect_err("invalid pair");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}
