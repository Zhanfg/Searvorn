use crate::{
    error::{ErrorKind, Result, SearvornError},
    vfs::RandomRead,
};

pub const DEFAULT_TEXT_PROBE_BYTES: usize = 4096;
pub const MAX_TEXT_PROBE_BYTES: usize = 16 * 1024;
pub const MAX_UTF8_WINDOW_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16LeBom,
    Utf16BeBom,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextProbe {
    pub encoding: TextEncoding,
    pub sampled_bytes: usize,
    pub contains_nul: bool,
}

impl TextProbe {
    pub const fn likely_binary(self) -> bool {
        matches!(self.encoding, TextEncoding::Unknown) || self.contains_nul
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Utf8Window {
    pub requested_offset: u64,
    pub byte_offset: u64,
    pub total_len: u64,
    pub bytes: Vec<u8>,
    pub eof: bool,
}

impl Utf8Window {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes).expect("Utf8Window invariant")
    }
}

pub fn probe_text<R>(reader: &mut R, requested_sample: usize) -> Result<TextProbe>
where
    R: RandomRead + ?Sized,
{
    let total_len = reader.len()?;
    let sample_len = requested_sample
        .min(MAX_TEXT_PROBE_BYTES)
        .min(total_len.min(usize::MAX as u64) as usize);
    let mut sample = vec![0u8; sample_len];
    read_fully_available(reader, 0, &mut sample)?;

    let contains_nul = sample.contains(&0);
    let encoding = if sample.starts_with(&[0xef, 0xbb, 0xbf]) {
        TextEncoding::Utf8Bom
    } else if sample.starts_with(&[0xff, 0xfe]) {
        TextEncoding::Utf16LeBom
    } else if sample.starts_with(&[0xfe, 0xff]) {
        TextEncoding::Utf16BeBom
    } else if std::str::from_utf8(&sample).is_ok() && !contains_nul {
        TextEncoding::Utf8
    } else {
        TextEncoding::Unknown
    };

    Ok(TextProbe {
        encoding,
        sampled_bytes: sample.len(),
        contains_nul,
    })
}

pub fn probe_text_default<R>(reader: &mut R) -> Result<TextProbe>
where
    R: RandomRead + ?Sized,
{
    probe_text(reader, DEFAULT_TEXT_PROBE_BYTES)
}

pub fn read_utf8_window<R>(
    reader: &mut R,
    requested_offset: u64,
    max_bytes: usize,
) -> Result<Utf8Window>
where
    R: RandomRead + ?Sized,
{
    if max_bytes > MAX_UTF8_WINDOW_BYTES {
        return Err(SearvornError::new(ErrorKind::InvalidInput, "text.window"));
    }

    let total_len = reader.len()?;
    if requested_offset > total_len {
        return Err(SearvornError::new(ErrorKind::InvalidInput, "text.window"));
    }

    if requested_offset == total_len || max_bytes == 0 {
        return Ok(Utf8Window {
            requested_offset,
            byte_offset: requested_offset,
            total_len,
            bytes: Vec::new(),
            eof: requested_offset == total_len,
        });
    }

    let probe_start = requested_offset.saturating_sub(3);
    let desired_end = requested_offset
        .saturating_add(max_bytes as u64)
        .saturating_add(3)
        .min(total_len);
    let raw_len = usize::try_from(desired_end - probe_start)
        .map_err(|_| SearvornError::new(ErrorKind::Unsupported, "text.window"))?;
    let mut raw = vec![0u8; raw_len];
    read_fully_available(reader, probe_start, &mut raw)?;

    let mut start = usize::try_from(requested_offset - probe_start)
        .map_err(|_| SearvornError::new(ErrorKind::Unsupported, "text.window"))?;

    while start > 0 && start < raw.len() && is_utf8_continuation(raw[start]) {
        start -= 1;
    }

    let mut end = start.saturating_add(max_bytes).min(raw.len());
    while end > start && end < raw.len() && is_utf8_continuation(raw[end]) {
        end -= 1;
    }

    let bytes = raw[start..end].to_vec();
    std::str::from_utf8(&bytes).map_err(|error| {
        SearvornError::with_detail(
            ErrorKind::InvalidInput,
            "text.window",
            format!("invalid UTF-8 near byte {}", error.valid_up_to()),
        )
    })?;

    let byte_offset = probe_start + start as u64;
    Ok(Utf8Window {
        requested_offset,
        byte_offset,
        total_len,
        eof: byte_offset + bytes.len() as u64 >= total_len,
        bytes,
    })
}

fn is_utf8_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

fn read_fully_available<R>(reader: &mut R, offset: u64, buffer: &mut [u8]) -> Result<()>
where
    R: RandomRead + ?Sized,
{
    let mut filled = 0usize;

    while filled < buffer.len() {
        let read = reader.read_at(offset + filled as u64, &mut buffer[filled..])?;
        if read == 0 {
            return Err(SearvornError::with_detail(
                ErrorKind::Io,
                "text.read",
                "source ended before reported length",
            ));
        }

        filled += read;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        probe_text, read_utf8_window, TextEncoding, MAX_TEXT_PROBE_BYTES, MAX_UTF8_WINDOW_BYTES,
    };
    use crate::{error::Result, vfs::RandomRead, ErrorKind};

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

            let read = buffer
                .len()
                .min(self.max_read)
                .min(self.bytes.len() - offset);
            buffer[..read].copy_from_slice(&self.bytes[offset..offset + read]);
            Ok(read)
        }
    }

    #[test]
    fn probe_detects_utf8_and_boms() {
        let mut utf8 = MemoryReader {
            bytes: "hello 世界".as_bytes().to_vec(),
            max_read: 3,
        };
        assert_eq!(
            probe_text(&mut utf8, MAX_TEXT_PROBE_BYTES)
                .expect("probe")
                .encoding,
            TextEncoding::Utf8
        );

        let mut utf16 = MemoryReader {
            bytes: vec![0xff, 0xfe, b'a', 0],
            max_read: 2,
        };
        assert_eq!(
            probe_text(&mut utf16, 64).expect("probe").encoding,
            TextEncoding::Utf16LeBom
        );
    }

    #[test]
    fn nul_without_utf16_bom_is_treated_as_unknown() {
        let mut reader = MemoryReader {
            bytes: b"a\0b".to_vec(),
            max_read: 3,
        };
        let probe = probe_text(&mut reader, 64).expect("probe");

        assert_eq!(probe.encoding, TextEncoding::Unknown);
        assert!(probe.likely_binary());
    }

    #[test]
    fn utf8_window_moves_to_character_boundary() {
        let text = "A€B漢C";
        let mut reader = MemoryReader {
            bytes: text.as_bytes().to_vec(),
            max_read: 2,
        };

        let window = read_utf8_window(&mut reader, 2, 4).expect("window");

        assert_eq!(window.byte_offset, 1);
        assert_eq!(window.as_str(), "€");
    }

    #[test]
    fn utf8_window_reads_to_eof_without_splitting_character() {
        let text = "ab漢";
        let mut reader = MemoryReader {
            bytes: text.as_bytes().to_vec(),
            max_read: 2,
        };

        let window = read_utf8_window(&mut reader, 2, 32).expect("window");

        assert_eq!(window.as_str(), "漢");
        assert!(window.eof);
    }

    #[test]
    fn rejects_invalid_utf8_and_oversized_window() {
        let mut invalid = MemoryReader {
            bytes: vec![0xff, 0xfe, 0xfd],
            max_read: 3,
        };
        assert_eq!(
            read_utf8_window(&mut invalid, 0, 3)
                .expect_err("invalid UTF-8")
                .kind(),
            ErrorKind::InvalidInput
        );

        let mut reader = MemoryReader {
            bytes: b"x".to_vec(),
            max_read: 1,
        };
        assert_eq!(
            read_utf8_window(&mut reader, 0, MAX_UTF8_WINDOW_BYTES + 1)
                .expect_err("oversized")
                .kind(),
            ErrorKind::InvalidInput
        );
    }
}
