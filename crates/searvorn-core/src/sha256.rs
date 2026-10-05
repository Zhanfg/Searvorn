use crate::{
    error::{ErrorKind, Result, SearvornError},
    vfs::RandomRead,
};

const INITIAL: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    total_len: u64,
}

impl Sha256 {
    pub const fn new() -> Self {
        Self {
            state: INITIAL,
            buffer: [0; 64],
            buffer_len: 0,
            total_len: 0,
        }
    }

    pub fn update(&mut self, mut bytes: &[u8]) {
        self.total_len = self.total_len.saturating_add(bytes.len() as u64);

        if self.buffer_len != 0 {
            let copy = bytes.len().min(64 - self.buffer_len);
            self.buffer[self.buffer_len..self.buffer_len + copy].copy_from_slice(&bytes[..copy]);
            self.buffer_len += copy;
            bytes = &bytes[copy..];

            if self.buffer_len == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffer_len = 0;
            }
        }

        while bytes.len() >= 64 {
            let block: &[u8; 64] = bytes[..64].try_into().expect("slice length checked");
            self.compress(block);
            bytes = &bytes[64..];
        }

        if !bytes.is_empty() {
            self.buffer[..bytes.len()].copy_from_slice(bytes);
            self.buffer_len = bytes.len();
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        let mut padding = [0u8; 128];
        padding[0] = 0x80;

        let remainder = self.buffer_len;
        let zero_pad = if remainder < 56 {
            56 - remainder
        } else {
            120 - remainder
        };
        let padding_len = zero_pad + 8;
        padding[zero_pad..padding_len].copy_from_slice(&bit_len.to_be_bytes());

        self.update(&padding[..padding_len]);

        debug_assert_eq!(self.buffer_len, 0);

        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];

        for (index, chunk) in block.chunks_exact(4).take(16).enumerate() {
            w[index] = u32::from_be_bytes(chunk.try_into().expect("chunk is four bytes"));
        }

        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }

        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];

        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize()
}

pub fn sha256_reader_range<R>(reader: &mut R, offset: u64, len: u64) -> Result<[u8; 32]>
where
    R: RandomRead + ?Sized,
{
    let source_len = reader.len()?;
    let end = offset
        .checked_add(len)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "sha256.range"))?;

    if end > source_len {
        return Err(SearvornError::new(ErrorKind::InvalidInput, "sha256.range"));
    }

    let mut hasher = Sha256::new();
    let mut scratch = [0u8; 64 * 1024];
    let mut cursor = offset;

    while cursor < end {
        let remaining = end - cursor;
        let wanted = scratch.len().min(remaining.min(usize::MAX as u64) as usize);
        let read = reader.read_at(cursor, &mut scratch[..wanted])?;

        if read == 0 {
            return Err(SearvornError::with_detail(
                ErrorKind::Io,
                "sha256.range",
                "source ended before reported length",
            ));
        }

        hasher.update(&scratch[..read]);
        cursor += read as u64;
    }

    Ok(hasher.finalize())
}

pub fn format_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        output.push(char::from(HEX[(byte >> 4) as usize]));
        output.push(char::from(HEX[(byte & 0x0f) as usize]));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::{format_hex, sha256, sha256_reader_range, Sha256};
    use crate::{error::Result, vfs::RandomRead};

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
    fn matches_standard_empty_and_abc_vectors() {
        assert_eq!(
            format_hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            format_hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn incremental_updates_match_single_update() {
        let mut hasher = Sha256::new();
        hasher.update(b"a");
        hasher.update(b"b");
        hasher.update(b"c");

        assert_eq!(hasher.finalize(), sha256(b"abc"));
    }

    #[test]
    fn hashes_reader_range_with_partial_reads() {
        let mut reader = MemoryReader {
            bytes: b"0123456789".to_vec(),
            max_read: 2,
        };

        let digest = sha256_reader_range(&mut reader, 2, 5).expect("range");
        assert_eq!(digest, sha256(b"23456"));
    }
}
