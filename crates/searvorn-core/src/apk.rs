use std::collections::BTreeSet;

use crate::{
    error::Result,
    vfs::RandomRead,
    zip::{scan_zip, ZipEntry},
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApkSummary {
    pub total_entries: u64,
    pub has_android_manifest: bool,
    pub has_resources_arsc: bool,
    pub dex_files: u32,
    pub native_libraries: u32,
    pub native_abis: Vec<String>,
    pub asset_entries: u32,
    pub resource_entries: u32,
    pub meta_inf_entries: u32,
    pub v1_signature_files: u32,
}

pub fn inspect_apk<R>(reader: &mut R) -> Result<ApkSummary>
where
    R: RandomRead + ?Sized,
{
    let mut builder = ApkSummaryBuilder::default();
    let zip = scan_zip(reader, |entry| {
        builder.observe(entry);
        Ok(())
    })?;

    let mut summary = builder.finish();
    summary.total_entries = zip.entries;
    Ok(summary)
}

#[derive(Default)]
struct ApkSummaryBuilder {
    summary: ApkSummary,
    abis: BTreeSet<String>,
}

impl ApkSummaryBuilder {
    fn observe(&mut self, entry: &ZipEntry) {
        let name = entry.name.as_slice();

        if name == b"AndroidManifest.xml" {
            self.summary.has_android_manifest = true;
        } else if name == b"resources.arsc" {
            self.summary.has_resources_arsc = true;
        }

        if is_dex_entry(name) {
            self.summary.dex_files = self.summary.dex_files.saturating_add(1);
        }

        if name.starts_with(b"assets/") && name.len() > b"assets/".len() {
            self.summary.asset_entries = self.summary.asset_entries.saturating_add(1);
        }

        if name.starts_with(b"res/") && name.len() > b"res/".len() {
            self.summary.resource_entries = self.summary.resource_entries.saturating_add(1);
        }

        if name.starts_with(b"META-INF/") && name.len() > b"META-INF/".len() {
            self.summary.meta_inf_entries = self.summary.meta_inf_entries.saturating_add(1);

            if is_v1_signature_file(name) {
                self.summary.v1_signature_files = self.summary.v1_signature_files.saturating_add(1);
            }
        }

        if let Some(abi) = native_library_abi(name) {
            self.summary.native_libraries = self.summary.native_libraries.saturating_add(1);
            self.abis.insert(abi.to_owned());
        }
    }

    fn finish(self) -> ApkSummary {
        ApkSummary {
            native_abis: self.abis.into_iter().collect(),
            ..self.summary
        }
    }
}

fn is_dex_entry(name: &[u8]) -> bool {
    let Some(stem) = name
        .strip_prefix(b"classes")
        .and_then(|value| value.strip_suffix(b".dex"))
    else {
        return false;
    };

    stem.is_empty() || stem.iter().all(u8::is_ascii_digit)
}

fn native_library_abi(name: &[u8]) -> Option<&str> {
    let rest = name.strip_prefix(b"lib/")?;
    let slash = rest.iter().position(|byte| *byte == b'/')?;
    let abi = &rest[..slash];
    let file = &rest[slash + 1..];

    if abi.is_empty() || !file.ends_with(b".so") || file.len() <= 3 {
        return None;
    }

    std::str::from_utf8(abi).ok()
}

fn is_v1_signature_file(name: &[u8]) -> bool {
    [b".SF".as_slice(), b".RSA", b".DSA", b".EC"]
        .into_iter()
        .any(|suffix| {
            name.len() >= suffix.len()
                && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
        })
}

#[cfg(test)]
mod tests {
    use super::{is_dex_entry, is_v1_signature_file, native_library_abi, ApkSummaryBuilder};
    use crate::zip::ZipEntry;

    fn entry(name: &[u8]) -> ZipEntry {
        ZipEntry {
            name: name.to_vec(),
            flags: 0,
            compression_method: 0,
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            local_header_offset: 0,
        }
    }

    #[test]
    fn recognizes_primary_and_secondary_dex_files() {
        assert!(is_dex_entry(b"classes.dex"));
        assert!(is_dex_entry(b"classes2.dex"));
        assert!(is_dex_entry(b"classes17.dex"));
        assert!(!is_dex_entry(b"classesx.dex"));
        assert!(!is_dex_entry(b"path/classes.dex"));
    }

    #[test]
    fn extracts_native_abi_only_from_shared_libraries() {
        assert_eq!(
            native_library_abi(b"lib/arm64-v8a/libfoo.so"),
            Some("arm64-v8a")
        );
        assert_eq!(native_library_abi(b"lib/x86_64/readme.txt"), None);
        assert_eq!(native_library_abi(b"lib//libfoo.so"), None);
    }

    #[test]
    fn recognizes_v1_signature_sidecars_case_insensitively() {
        assert!(is_v1_signature_file(b"META-INF/CERT.RSA"));
        assert!(is_v1_signature_file(b"META-INF/cert.sf"));
        assert!(!is_v1_signature_file(b"META-INF/MANIFEST.MF"));
    }

    #[test]
    fn builder_counts_apk_structure_without_retaining_every_entry() {
        let mut builder = ApkSummaryBuilder::default();

        for name in [
            b"AndroidManifest.xml".as_slice(),
            b"resources.arsc",
            b"classes.dex",
            b"classes2.dex",
            b"assets/config.json",
            b"res/layout/main.xml",
            b"lib/arm64-v8a/libfoo.so",
            b"lib/arm64-v8a/libbar.so",
            b"lib/x86_64/libfoo.so",
            b"META-INF/CERT.RSA",
            b"META-INF/CERT.SF",
        ] {
            builder.observe(&entry(name));
        }

        let summary = builder.finish();

        assert!(summary.has_android_manifest);
        assert!(summary.has_resources_arsc);
        assert_eq!(summary.dex_files, 2);
        assert_eq!(summary.native_libraries, 3);
        assert_eq!(summary.native_abis, ["arm64-v8a", "x86_64"]);
        assert_eq!(summary.asset_entries, 1);
        assert_eq!(summary.resource_entries, 1);
        assert_eq!(summary.meta_inf_entries, 2);
        assert_eq!(summary.v1_signature_files, 2);
    }
}
