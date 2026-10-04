use crate::{
    error::{ErrorKind, Result, SearvornError},
    vfs::RandomRead,
    window::{read_window_into, WindowRead},
};

pub const DEFAULT_BYTES_PER_ROW: usize = 16;
pub const MAX_HEX_WINDOW_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HexLayout {
    bytes_per_row: usize,
}

impl HexLayout {
    pub fn new(bytes_per_row: usize) -> Result<Self> {
        if bytes_per_row == 0 || bytes_per_row > 256 {
            return Err(SearvornError::new(
                ErrorKind::InvalidInput,
                "hex.layout",
            ));
        }

        Ok(Self { bytes_per_row })
    }

    pub const fn bytes_per_row(self) -> usize {
        self.bytes_per_row
    }

    pub fn row_for_offset(self, offset: u64) -> u64 {
        offset / self.bytes_per_row as u64
    }

    pub fn offset_for_row(self, row: u64) -> Result<u64> {
        row.checked_mul(self.bytes_per_row as u64)
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "hex.row"))
    }

    pub fn column_for_offset(self, offset: u64) -> usize {
        (offset % self.bytes_per_row as u64) as usize
    }
}

impl Default for HexLayout {
    fn default() -> Self {
        Self {
            bytes_per_row: DEFAULT_BYTES_PER_ROW,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HexWindow {
    pub first_row: u64,
    pub row_capacity: usize,
    pub byte_offset: u64,
    pub bytes_read: usize,
    pub total_len: u64,
    pub eof: bool,
}

pub fn read_hex_rows_into<R>(
    reader: &mut R,
    layout: HexLayout,
    first_row: u64,
    row_capacity: usize,
    buffer: &mut [u8],
) -> Result<HexWindow>
where
    R: RandomRead + ?Sized,
{
    let requested = row_capacity
        .checked_mul(layout.bytes_per_row())
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "hex.window"))?;

    if requested > MAX_HEX_WINDOW_BYTES || requested > buffer.len() {
        return Err(SearvornError::new(
            ErrorKind::InvalidInput,
            "hex.window",
        ));
    }

    let offset = layout.offset_for_row(first_row)?;
    let WindowRead {
        read,
        total_len,
        eof,
        ..
    } = read_window_into(reader, offset, &mut buffer[..requested])?;

    Ok(HexWindow {
        first_row,
        row_capacity,
        byte_offset: offset,
        bytes_read: read,
        total_len,
        eof,
    })
}

pub fn absolute_offset(
    layout: HexLayout,
    first_row: u64,
    row_in_window: usize,
    column: usize,
) -> Result<u64> {
    if column >= layout.bytes_per_row() {
        return Err(SearvornError::new(
            ErrorKind::InvalidInput,
            "hex.offset",
        ));
    }

    let row = first_row
        .checked_add(row_in_window as u64)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "hex.offset"))?;
    let base = layout.offset_for_row(row)?;

    base.checked_add(column as u64)
        .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "hex.offset"))
}

#[cfg(test)]
mod tests {
    use super::{
        absolute_offset, read_hex_rows_into, HexLayout, DEFAULT_BYTES_PER_ROW,
        MAX_HEX_WINDOW_BYTES,
    };
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
    fn default_layout_maps_offsets_to_rows_and_columns() {
        let layout = HexLayout::default();

        assert_eq!(layout.bytes_per_row(), DEFAULT_BYTES_PER_ROW);
        assert_eq!(layout.row_for_offset(31), 1);
        assert_eq!(layout.column_for_offset(31), 15);
        assert_eq!(layout.offset_for_row(2).expect("offset"), 32);
    }

    #[test]
    fn reads_aligned_rows_into_reused_buffer() {
        let mut reader = MemoryReader((0u8..80).collect());
        let layout = HexLayout::new(16).expect("layout");
        let mut buffer = [0u8; 64];

        let window =
            read_hex_rows_into(&mut reader, layout, 2, 3, &mut buffer).expect("window");

        assert_eq!(window.byte_offset, 32);
        assert_eq!(window.bytes_read, 48);
        assert_eq!(&buffer[..48], &(32u8..80).collect::<Vec<_>>());
        assert!(window.eof);
    }

    #[test]
    fn computes_cell_absolute_offset() {
        let layout = HexLayout::new(16).expect("layout");

        assert_eq!(absolute_offset(layout, 10, 2, 7).expect("offset"), 199);
    }

    #[test]
    fn rejects_invalid_columns_and_oversized_windows() {
        let layout = HexLayout::new(16).expect("layout");
        assert_eq!(
            absolute_offset(layout, 0, 0, 16).expect_err("column").kind(),
            ErrorKind::InvalidInput
        );

        let mut reader = MemoryReader(vec![0; 32]);
        let mut buffer = vec![0u8; MAX_HEX_WINDOW_BYTES + 16];
        let rows = MAX_HEX_WINDOW_BYTES / 16 + 1;

        assert_eq!(
            read_hex_rows_into(&mut reader, layout, 0, rows, &mut buffer)
                .expect_err("oversized")
                .kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn rejects_zero_width_layout() {
        assert_eq!(
            HexLayout::new(0).expect_err("zero width").kind(),
            ErrorKind::InvalidInput
        );
    }
}
