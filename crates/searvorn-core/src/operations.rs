use crate::{
    task::CancellationFlag,
    transfer::{copy_stream, CopyOptions, CopyProgress},
    vfs::{CommitOutcome, VfsBackend},
    Result,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileCopyOutcome {
    pub copied: u64,
    pub commit: CommitOutcome,
}

pub fn copy_file<F>(
    source: &dyn VfsBackend,
    source_path: &str,
    destination: &dyn VfsBackend,
    destination_path: &str,
    options: CopyOptions,
    cancellation: Option<&CancellationFlag>,
    progress: F,
) -> Result<FileCopyOutcome>
where
    F: FnMut(CopyProgress),
{
    let mut reader = source.open_read(source_path)?;
    let mut transaction = destination.begin_replace(destination_path)?;
    let copied = copy_stream(
        reader.as_mut(),
        transaction.as_mut(),
        options,
        cancellation,
        progress,
    )?;
    let commit = transaction.commit()?;

    Ok(FileCopyOutcome { copied, commit })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::copy_file;
    use crate::{task::CancellationFlag, transfer::CopyOptions, ErrorKind, LocalFsBackend};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("searvorn-op-{}-{nonce}", process::id()));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn copies_between_backends_transactionally() {
        let source_dir = TestDir::new();
        let destination_dir = TestDir::new();
        let input = vec![0x41; 512 * 1024 + 3];
        fs::write(source_dir.0.join("source.bin"), &input).expect("seed source");

        let source = LocalFsBackend::new(&source_dir.0, false).expect("source backend");
        let destination =
            LocalFsBackend::new(&destination_dir.0, true).expect("destination backend");

        let outcome = copy_file(
            &source,
            "source.bin",
            &destination,
            "target.bin",
            CopyOptions::default(),
            None,
            |_| {},
        )
        .expect("copy");

        assert_eq!(outcome.copied, input.len() as u64);
        assert!(outcome.commit.atomic);
        assert_eq!(
            fs::read(destination_dir.0.join("target.bin")).expect("read target"),
            input
        );
    }

    #[test]
    fn cancelled_copy_leaves_existing_target_untouched() {
        let source_dir = TestDir::new();
        let destination_dir = TestDir::new();
        fs::write(source_dir.0.join("source.bin"), vec![0x42; 64 * 1024]).expect("seed source");
        fs::write(destination_dir.0.join("target.bin"), b"old").expect("seed target");

        let source = LocalFsBackend::new(&source_dir.0, false).expect("source backend");
        let destination =
            LocalFsBackend::new(&destination_dir.0, true).expect("destination backend");
        let cancellation = CancellationFlag::new();

        let error = copy_file(
            &source,
            "source.bin",
            &destination,
            "target.bin",
            CopyOptions { chunk_size: 4096 },
            Some(&cancellation),
            |progress| {
                if progress.copied >= 8192 {
                    cancellation.cancel();
                }
            },
        )
        .expect_err("copy must be cancelled");

        assert_eq!(error.kind(), ErrorKind::Interrupted);
        assert_eq!(
            fs::read(destination_dir.0.join("target.bin")).expect("read target"),
            b"old"
        );
    }
}
