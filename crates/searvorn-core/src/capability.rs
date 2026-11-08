#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Capability {
    FileRead = 0,
    FileWrite,
    ProcessExec,
    PackageQuery,
    PackageInstall,
    AndroidShell,
    PrivilegedFs,
    MountNamespace,
    RawDevice,
}

impl Capability {
    const fn mask(self) -> u128 {
        1u128 << self as u8
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet {
    bits: u128,
}

impl CapabilitySet {
    pub const EMPTY: Self = Self { bits: 0 };

    pub const fn from_bits(bits: u128) -> Self {
        Self { bits }
    }

    pub const fn bits(self) -> u128 {
        self.bits
    }

    pub const fn contains(self, capability: Capability) -> bool {
        self.bits & capability.mask() != 0
    }

    pub const fn contains_all(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    pub const fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    pub const fn count(self) -> u32 {
        self.bits.count_ones()
    }

    pub fn insert(&mut self, capability: Capability) {
        self.bits |= capability.mask();
    }

    pub fn remove(&mut self, capability: Capability) {
        self.bits &= !capability.mask();
    }
}

impl From<Capability> for CapabilitySet {
    fn from(value: Capability) -> Self {
        Self { bits: value.mask() }
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, CapabilitySet};

    #[test]
    fn tracks_individual_capabilities() {
        let mut set = CapabilitySet::EMPTY;

        set.insert(Capability::FileRead);
        set.insert(Capability::AndroidShell);

        assert!(set.contains(Capability::FileRead));
        assert!(set.contains(Capability::AndroidShell));
        assert!(!set.contains(Capability::RawDevice));

        set.remove(Capability::FileRead);
        assert!(!set.contains(Capability::FileRead));
    }

    #[test]
    fn checks_required_sets() {
        let granted = CapabilitySet::from(Capability::FileRead).union(Capability::FileWrite.into());

        assert!(granted.contains_all(Capability::FileRead.into()));
        assert!(granted.contains_all(Capability::FileWrite.into()));
        assert!(!granted.contains_all(Capability::PackageInstall.into()));
        assert_eq!(granted.count(), 2);
    }
}
