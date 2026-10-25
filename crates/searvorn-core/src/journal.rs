use crate::error::{ErrorKind, Result, SearvornError};

const MAGIC: [u8; 4] = *b"SVJ1";
const RECORD_SIZE: usize = 32;
const CHECKSUM_END: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum JournalKind {
    Begin = 1,
    Prepared = 2,
    Committed = 3,
    Aborted = 4,
}

impl JournalKind {
    fn from_byte(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::Begin),
            2 => Ok(Self::Prepared),
            3 => Ok(Self::Committed),
            4 => Ok(Self::Aborted),
            _ => Err(SearvornError::new(
                ErrorKind::InvalidInput,
                "journal.decode",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TransactionId(u128);

impl TransactionId {
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u128 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalRecord {
    pub transaction: TransactionId,
    pub kind: JournalKind,
}

impl JournalRecord {
    pub const fn new(transaction: TransactionId, kind: JournalKind) -> Self {
        Self { transaction, kind }
    }

    pub fn encode(self) -> [u8; RECORD_SIZE] {
        let mut encoded = [0u8; RECORD_SIZE];
        encoded[..4].copy_from_slice(&MAGIC);
        encoded[4] = 1;
        encoded[5] = self.kind as u8;
        encoded[8..24].copy_from_slice(&self.transaction.get().to_le_bytes());

        let checksum = crc32(&encoded[..CHECKSUM_END]);
        encoded[24..28].copy_from_slice(&checksum.to_le_bytes());
        encoded[28..].copy_from_slice(&MAGIC);
        encoded
    }

    pub fn decode(encoded: &[u8]) -> Result<Self> {
        if encoded.len() != RECORD_SIZE
            || encoded[..4] != MAGIC
            || encoded[28..] != MAGIC
            || encoded[4] != 1
        {
            return Err(SearvornError::new(
                ErrorKind::InvalidInput,
                "journal.decode",
            ));
        }

        let expected = u32::from_le_bytes(
            encoded[24..28]
                .try_into()
                .expect("checksum field has fixed width"),
        );
        if crc32(&encoded[..CHECKSUM_END]) != expected {
            return Err(SearvornError::with_detail(
                ErrorKind::InvalidInput,
                "journal.decode",
                "checksum mismatch",
            ));
        }

        let transaction = u128::from_le_bytes(
            encoded[8..24]
                .try_into()
                .expect("transaction field has fixed width"),
        );

        Ok(Self {
            transaction: TransactionId::new(transaction),
            kind: JournalKind::from_byte(encoded[5])?,
        })
    }
}

pub fn scan_records<F>(bytes: &[u8], mut visitor: F) -> Result<usize>
where
    F: FnMut(JournalRecord) -> Result<()>,
{
    let complete = bytes.len() / RECORD_SIZE;
    for index in 0..complete {
        let start = index * RECORD_SIZE;
        let record = JournalRecord::decode(&bytes[start..start + RECORD_SIZE])?;
        visitor(record)?;
    }

    Ok(complete)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionState {
    Begun,
    Prepared,
    Committed,
    Aborted,
}

impl From<JournalKind> for TransactionState {
    fn from(kind: JournalKind) -> Self {
        match kind {
            JournalKind::Begin => Self::Begun,
            JournalKind::Prepared => Self::Prepared,
            JournalKind::Committed => Self::Committed,
            JournalKind::Aborted => Self::Aborted,
        }
    }
}

pub fn reduce_states(
    bytes: &[u8],
) -> Result<std::collections::BTreeMap<TransactionId, TransactionState>> {
    let mut states = std::collections::BTreeMap::new();

    scan_records(bytes, |record| {
        states.insert(record.transaction, record.kind.into());
        Ok(())
    })?;

    Ok(states)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;

    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }

    !crc
}

#[cfg(test)]
mod tests {
    use super::{scan_records, JournalKind, JournalRecord, TransactionId};
    use crate::ErrorKind;

    #[test]
    fn record_round_trip() {
        let record = JournalRecord::new(
            TransactionId::new(0x1122_3344_5566_7788_99aa_bbcc_ddee_ff00),
            JournalKind::Prepared,
        );

        assert_eq!(
            JournalRecord::decode(&record.encode()).expect("decode"),
            record
        );
    }

    #[test]
    fn checksum_rejects_corruption() {
        let mut encoded = JournalRecord::new(TransactionId::new(7), JournalKind::Begin).encode();
        encoded[12] ^= 0x80;

        let error = JournalRecord::decode(&encoded).expect_err("corruption must fail");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn reducer_keeps_last_complete_state() {
        let transaction = TransactionId::new(42);
        let mut bytes = Vec::new();

        for kind in [
            JournalKind::Begin,
            JournalKind::Prepared,
            JournalKind::Committed,
        ] {
            bytes.extend_from_slice(&JournalRecord::new(transaction, kind).encode());
        }

        let states = super::reduce_states(&bytes).expect("reduce");
        assert_eq!(
            states.get(&transaction),
            Some(&super::TransactionState::Committed)
        );
    }

    #[test]
    fn scan_ignores_torn_trailing_record() {
        let first = JournalRecord::new(TransactionId::new(1), JournalKind::Begin);
        let second = JournalRecord::new(TransactionId::new(1), JournalKind::Committed);
        let mut bytes = Vec::from(first.encode());
        bytes.extend_from_slice(&second.encode()[..11]);

        let mut seen = Vec::new();
        let count = scan_records(&bytes, |record| {
            seen.push(record);
            Ok(())
        })
        .expect("scan");

        assert_eq!(count, 1);
        assert_eq!(seen, vec![first]);
    }
}
