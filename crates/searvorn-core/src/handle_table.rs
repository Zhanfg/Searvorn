use crate::{
    error::{ErrorKind, Result, SearvornError},
};

const FREE_NONE: u32 = u32::MAX;
const MAX_GENERATION: u32 = 0x7fff_ffff;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Handle(u64);

impl Handle {
    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    fn parts(self) -> Option<(usize, u32)> {
        let low = self.0 as u32;
        let generation = (self.0 >> 32) as u32;

        if low == 0 || generation == 0 || generation > MAX_GENERATION {
            return None;
        }

        Some(((low - 1) as usize, generation))
    }
}

#[derive(Debug)]
struct Slot<T> {
    value: Option<T>,
    generation: u32,
    next_free: u32,
}

#[derive(Debug)]
pub struct HandleTable<T> {
    slots: Vec<Slot<T>>,
    free_head: u32,
    occupied: usize,
}

impl<T> HandleTable<T> {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_head: FREE_NONE,
            occupied: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.occupied
    }

    pub const fn is_empty(&self) -> bool {
        self.occupied == 0
    }

    pub fn insert(&mut self, value: T) -> Result<Handle> {
        let (index, generation) = if self.free_head != FREE_NONE {
            let index = self.free_head as usize;
            let slot = self
                .slots
                .get_mut(index)
                .ok_or_else(|| SearvornError::new(ErrorKind::Io, "handle_table.insert"))?;

            self.free_head = slot.next_free;
            slot.next_free = FREE_NONE;
            slot.value = Some(value);
            (index, slot.generation)
        } else {
            let index = self.slots.len();
            if index >= u32::MAX as usize {
                return Err(SearvornError::new(
                    ErrorKind::Unsupported,
                    "handle_table.insert",
                ));
            }

            self.slots.push(Slot {
                value: Some(value),
                generation: 1,
                next_free: FREE_NONE,
            });
            (index, 1)
        };

        self.occupied += 1;
        encode_handle(index, generation)
    }

    pub fn get(&self, handle: Handle) -> Option<&T> {
        let (index, generation) = handle.parts()?;
        let slot = self.slots.get(index)?;

        if slot.generation != generation {
            return None;
        }

        slot.value.as_ref()
    }

    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        let (index, generation) = handle.parts()?;
        let slot = self.slots.get_mut(index)?;

        if slot.generation != generation {
            return None;
        }

        slot.value.as_mut()
    }

    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        let (index, generation) = handle.parts()?;
        let slot = self.slots.get_mut(index)?;

        if slot.generation != generation {
            return None;
        }

        let value = slot.value.take()?;
        slot.generation = next_generation(slot.generation);
        slot.next_free = self.free_head;
        self.free_head = index as u32;
        self.occupied -= 1;
        Some(value)
    }
}

impl<T> Default for HandleTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

fn encode_handle(index: usize, generation: u32) -> Result<Handle> {
    let low = u32::try_from(index + 1)
        .map_err(|_| SearvornError::new(ErrorKind::Unsupported, "handle_table.encode"))?;
    Ok(Handle((u64::from(generation) << 32) | u64::from(low)))
}

const fn next_generation(generation: u32) -> u32 {
    if generation >= MAX_GENERATION {
        1
    } else {
        generation + 1
    }
}

#[cfg(test)]
mod tests {
    use super::{Handle, HandleTable};

    #[test]
    fn inserts_reads_and_removes_values() {
        let mut table = HandleTable::new();
        let handle = table.insert("one").expect("insert");

        assert_eq!(table.len(), 1);
        assert_eq!(table.get(handle), Some(&"one"));
        assert_eq!(table.remove(handle), Some("one"));
        assert!(table.is_empty());
        assert_eq!(table.get(handle), None);
    }

    #[test]
    fn recycled_slot_changes_generation() {
        let mut table = HandleTable::new();
        let first = table.insert(10).expect("first");
        assert_eq!(table.remove(first), Some(10));

        let second = table.insert(20).expect("second");

        assert_ne!(first, second);
        assert_eq!(table.get(first), None);
        assert_eq!(table.get(second), Some(&20));
    }

    #[test]
    fn stale_handle_cannot_remove_recycled_value() {
        let mut table = HandleTable::new();
        let stale = table.insert(1).expect("insert");
        table.remove(stale).expect("remove");
        let current = table.insert(2).expect("reuse");

        assert_eq!(table.remove(stale), None);
        assert_eq!(table.get(current), Some(&2));
    }

    #[test]
    fn rejects_zero_raw_handle() {
        assert_eq!(Handle::from_raw(0), None);
    }
}
