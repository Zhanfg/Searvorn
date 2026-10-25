use crate::CapabilitySet;

#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a, T> {
    pub backend: &'a T,
    pub capabilities: CapabilitySet,
    pub cost: u16,
}

pub fn resolve<'a, T>(
    required: CapabilitySet,
    candidates: impl IntoIterator<Item = Candidate<'a, T>>,
) -> Option<&'a T> {
    candidates
        .into_iter()
        .filter(|candidate| candidate.capabilities.contains_all(required))
        .min_by_key(|candidate| {
            let extra = candidate.capabilities.count() - required.count();
            (candidate.cost, extra)
        })
        .map(|candidate| candidate.backend)
}

#[cfg(test)]
mod tests {
    use super::{resolve, Candidate};
    use crate::{Capability, CapabilitySet};

    #[test]
    fn ignores_backends_without_required_capabilities() {
        let local = "local";
        let shell = "shell";
        let required = CapabilitySet::from(Capability::FileWrite);

        let selected = resolve(
            required,
            [
                Candidate {
                    backend: &local,
                    capabilities: Capability::FileRead.into(),
                    cost: 0,
                },
                Candidate {
                    backend: &shell,
                    capabilities: CapabilitySet::from(Capability::FileRead)
                        .union(Capability::FileWrite.into()),
                    cost: 1,
                },
            ],
        );

        assert_eq!(selected, Some(&shell));
    }

    #[test]
    fn prefers_lower_cost_then_narrower_capability_set() {
        let broad = "broad";
        let narrow = "narrow";
        let required = CapabilitySet::from(Capability::FileRead);

        let selected = resolve(
            required,
            [
                Candidate {
                    backend: &broad,
                    capabilities: required.union(Capability::PrivilegedFs.into()),
                    cost: 0,
                },
                Candidate {
                    backend: &narrow,
                    capabilities: required,
                    cost: 0,
                },
            ],
        );

        assert_eq!(selected, Some(&narrow));
    }
}
