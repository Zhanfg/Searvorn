use crate::{
    error::{ErrorKind, Result, SearvornError},
    sha256::sha256_reader_range,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApkSigningScheme {
    V2,
    V3,
    V31,
}

impl ApkSigningScheme {
    const fn block_id(self) -> u32 {
        match self {
            Self::V2 => V2_BLOCK_ID,
            Self::V3 => V3_BLOCK_ID,
            Self::V31 => V31_BLOCK_ID,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApkSigningCertificate {
    pub scheme: ApkSigningScheme,
    pub signer_index: u32,
    pub certificate_index: u32,
    pub der_offset: u64,
    pub der_len: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug)]
struct SigningBlockBounds {
    block_offset: u64,
    total_size: u64,
    pairs_start: u64,
    pairs_end: u64,
}

pub fn inspect_signing_block<R>(
    reader: &mut R,
    central_directory_offset: u64,
) -> Result<ApkSigningBlockSummary>
where
    R: RandomRead + ?Sized,
{
    let Some(bounds) = locate_signing_block(reader, central_directory_offset)? else {
        return Ok(ApkSigningBlockSummary::default());
    };

    let mut cursor = bounds.pairs_start;
    let mut summary = ApkSigningBlockSummary {
        present: true,
        block_offset: bounds.block_offset,
        block_size: bounds.total_size,
        ..ApkSigningBlockSummary::default()
    };

    while cursor < bounds.pairs_end {
        let pair = read_pair_header(reader, cursor, bounds.pairs_end)?;
        summary.pair_count = summary.pair_count.saturating_add(1);

        match pair.id {
            V2_BLOCK_ID => summary.has_v2 = true,
            V3_BLOCK_ID => summary.has_v3 = true,
            V31_BLOCK_ID => summary.has_v31 = true,
            _ => {}
        }

        cursor = pair.next;
    }

    if cursor != bounds.pairs_end {
        return Err(SearvornError::new(
            ErrorKind::InvalidInput,
            "apk_signing.block",
        ));
    }

    Ok(summary)
}

pub fn signing_certificates<R>(
    reader: &mut R,
    central_directory_offset: u64,
) -> Result<Vec<ApkSigningCertificate>>
where
    R: RandomRead + ?Sized,
{
    let Some(bounds) = locate_signing_block(reader, central_directory_offset)? else {
        return Ok(Vec::new());
    };

    let mut certificates = Vec::new();

    for scheme in [
        ApkSigningScheme::V31,
        ApkSigningScheme::V3,
        ApkSigningScheme::V2,
    ] {
        if let Some((value_offset, value_len)) =
            find_pair_value(reader, bounds, scheme.block_id())?
        {
            parse_scheme_certificates(
                reader,
                scheme,
                value_offset,
                value_len,
                &mut certificates,
            )?;
        }
    }

    Ok(certificates)
}

fn parse_scheme_certificates<R>(
    reader: &mut R,
    scheme: ApkSigningScheme,
    value_offset: u64,
    value_len: u64,
    output: &mut Vec<ApkSigningCertificate>,
) -> Result<()>
where
    R: RandomRead + ?Sized,
{
    let value_end = checked_end(value_offset, value_len, "apk_signing.scheme")?;
    let (signers_offset, signers_len) = read_length_prefixed(reader, value_offset, value_end)?;
    let signers_end = checked_end(signers_offset, signers_len, "apk_signing.signers")?;
    let mut signer_cursor = signers_offset;
    let mut signer_index = 0u32;

    while signer_cursor < signers_end {
        let (signer_offset, signer_len) =
            read_length_prefixed(reader, signer_cursor, signers_end)?;
        let signer_end = checked_end(signer_offset, signer_len, "apk_signing.signer")?;

        let (signed_data_offset, signed_data_len) =
            read_length_prefixed(reader, signer_offset, signer_end)?;
        let signed_data_end =
            checked_end(signed_data_offset, signed_data_len, "apk_signing.signed_data")?;

        let (_, digests_len) =
            read_length_prefixed(reader, signed_data_offset, signed_data_end)?;
        let certificates_field = signed_data_offset
            .checked_add(4)
            .and_then(|offset| offset.checked_add(digests_len))
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "apk_signing.certificates"))?;

        let (certificates_offset, certificates_len) =
            read_length_prefixed(reader, certificates_field, signed_data_end)?;
        let certificates_end = checked_end(
            certificates_offset,
            certificates_len,
            "apk_signing.certificates",
        )?;
        let mut certificate_cursor = certificates_offset;
        let mut certificate_index = 0u32;

        while certificate_cursor < certificates_end {
            let (der_offset, der_len) =
                read_length_prefixed(reader, certificate_cursor, certificates_end)?;
            let der_end = checked_end(der_offset, der_len, "apk_signing.certificate")?;
            let fingerprint = sha256_reader_range(reader, der_offset, der_len)?;

            output.push(ApkSigningCertificate {
                scheme,
                signer_index,
                certificate_index,
                der_offset,
                der_len,
                sha256: fingerprint,
            });

            certificate_index = certificate_index.saturating_add(1);
            certificate_cursor = der_end;
        }

        if certificate_cursor != certificates_end {
            return Err(SearvornError::new(
                ErrorKind::InvalidInput,
                "apk_signing.certificates",
            ));
        }

        signer_index = signer_index.saturating_add(1);
        signer_cursor = signer_end;
    }

    if signer_cursor != signers_end {
        return Err(SearvornError::new(
            ErrorKind::InvalidInput,
            "apk_signing.signers",
        ));
    }

    Ok(())
}

fn locate_signing_block<R>(
    reader: &mut R,
    central_directory_offset: u64,
) -> Result<Option<SigningBlockBounds>>
where
    R: RandomRead + ?Sized,
{
    if central_directory_offset < FOOTER_LEN {
        return Ok(None);
    }

    let footer_offset = central_directory_offset - FOOTER_LEN;
    let mut footer = [0u8; FOOTER_LEN as usize];
    read_exact_at(reader, footer_offset, &mut footer)?;

    if footer[8..24] != MAGIC {
        return Ok(None);
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

    Ok(Some(SigningBlockBounds {
        block_offset,
        total_size,
        pairs_start: block_offset + LEADING_SIZE_LEN,
        pairs_end: footer_offset,
    }))
}

#[derive(Clone, Copy, Debug)]
struct PairHeader {
    id: u32,
    value_offset: u64,
    value_len: u64,
    next: u64,
}

fn read_pair_header<R>(reader: &mut R, cursor: u64, pairs_end: u64) -> Result<PairHeader>
where
    R: RandomRead + ?Sized,
{
    let remaining = pairs_end.saturating_sub(cursor);
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

    Ok(PairHeader {
        id,
        value_offset: cursor + 12,
        value_len: pair_len - 4,
        next,
    })
}

fn find_pair_value<R>(
    reader: &mut R,
    bounds: SigningBlockBounds,
    id: u32,
) -> Result<Option<(u64, u64)>>
where
    R: RandomRead + ?Sized,
{
    let mut cursor = bounds.pairs_start;

    while cursor < bounds.pairs_end {
        let pair = read_pair_header(reader, cursor, bounds.pairs_end)?;
        if pair.id == id {
            return Ok(Some((pair.value_offset, pair.value_len)));
        }
        cursor = pair.next;
    }

    Ok(None)
}

fn read_length_prefixed<R>(reader: &mut R, field_offset: u64, limit: u64) -> Result<(u64, u64)>
where
    R: RandomRead + ?Sized,
{
    let prefix_end = field_offset
        .checked_add(4)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "apk_signing.length"))?;
    if prefix_end > limit {
        return Err(SearvornError::with_detail(
            ErrorKind::InvalidInput,
            "apk_signing.length",
            "length prefix exceeds container",
        ));
    }

    let mut prefix = [0u8; 4];
    read_exact_at(reader, field_offset, &mut prefix)?;
    let len = u64::from(u32::from_le_bytes(prefix));
    let value_offset = prefix_end;
    let value_end = checked_end(value_offset, len, "apk_signing.length")?;

    if value_end > limit {
        return Err(SearvornError::with_detail(
            ErrorKind::InvalidInput,
            "apk_signing.length",
            "length-prefixed value exceeds container",
        ));
    }

    Ok((value_offset, len))
}

fn checked_end(offset: u64, len: u64, operation: &'static str) -> Result<u64> {
    offset
        .checked_add(len)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, operation))
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
    use super::{
        inspect_signing_block, signing_certificates, ApkSigningScheme, V2_BLOCK_ID, V31_BLOCK_ID,
        V3_BLOCK_ID,
    };
    use crate::{error::Result, sha256::sha256, vfs::RandomRead, ErrorKind};

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

        wrap_pairs(pairs)
    }

    fn signing_block_with_value(id: u32, value: &[u8]) -> Vec<u8> {
        let mut pairs = Vec::new();
        pairs.extend_from_slice(&(value.len() as u64 + 4).to_le_bytes());
        pairs.extend_from_slice(&id.to_le_bytes());
        pairs.extend_from_slice(value);
        wrap_pairs(pairs)
    }

    fn wrap_pairs(pairs: Vec<u8>) -> Vec<u8> {
        let size = pairs.len() as u64 + 24;
        let mut block = Vec::new();
        block.extend_from_slice(&size.to_le_bytes());
        block.extend_from_slice(&pairs);
        block.extend_from_slice(&size.to_le_bytes());
        block.extend_from_slice(b"APK Sig Block 42");
        block
    }

    fn length_prefixed(value: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        output.extend_from_slice(&(value.len() as u32).to_le_bytes());
        output.extend_from_slice(value);
        output
    }

    fn minimal_v2_value(certificate: &[u8]) -> Vec<u8> {
        let digests = length_prefixed(&[]);
        let certificates = length_prefixed(&length_prefixed(certificate));
        let attributes = length_prefixed(&[]);

        let mut signed_data = Vec::new();
        signed_data.extend_from_slice(&digests);
        signed_data.extend_from_slice(&certificates);
        signed_data.extend_from_slice(&attributes);

        let mut signer = Vec::new();
        signer.extend_from_slice(&length_prefixed(&signed_data));
        signer.extend_from_slice(&length_prefixed(&[]));
        signer.extend_from_slice(&length_prefixed(&[]));

        length_prefixed(&length_prefixed(&signer))
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
    fn extracts_certificate_and_hashes_der_without_full_apk_copy() {
        let certificate = b"fake-der-certificate";
        let value = minimal_v2_value(certificate);
        let bytes = signing_block_with_value(V2_BLOCK_ID, &value);
        let central_offset = bytes.len() as u64;
        let mut reader = MemoryReader(bytes);

        let certificates = signing_certificates(&mut reader, central_offset).expect("certificates");

        assert_eq!(certificates.len(), 1);
        let item = certificates[0];
        assert_eq!(item.scheme, ApkSigningScheme::V2);
        assert_eq!(item.signer_index, 0);
        assert_eq!(item.certificate_index, 0);
        assert_eq!(item.der_len, certificate.len() as u64);
        assert_eq!(item.sha256, sha256(certificate));
    }

    #[test]
    fn absence_of_magic_means_no_v2_plus_signing_block() {
        let mut reader = MemoryReader(vec![0; 64]);
        let summary = inspect_signing_block(&mut reader, 64).expect("inspect");

        assert!(!summary.present);
        assert!(signing_certificates(&mut reader, 64)
            .expect("certificates")
            .is_empty());
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
