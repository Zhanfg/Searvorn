use crate::{
    error::{ErrorKind, SearvornError},
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

#[derive(Debug)]
pub enum FileMoveOutcome {
    Completed {
        copy: FileCopyOutcome,
    },
    CopiedSourcePreserved {
        copy: FileCopyOutcome,
        cleanup_error: SearvornError,
    },
}

impl FileMoveOutcome {
    pub const fn copy(&self) -> FileCopyOutcome {
        match self {
            Self::Completed { copy } | Self::CopiedSourcePreserved { copy, .. } => *copy,
        }
    }

    pub const fn source_removed(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }
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

pub fn move_file<F>(
    source: &dyn VfsBackend,
    source_path: &str,
    destination: &dyn VfsBackend,
    destination_path: &str,
    options: CopyOptions,
    cancellation: Option<&CancellationFlag>,
    progress: F,
) -> Result<FileMoveOutcome>
where
    F: FnMut(CopyProgress),
{
    if std::ptr::eq(source, destination) && source_path == destination_path {
        return Err(SearvornError::new(
            ErrorKind::InvalidInput,
            "operations.move_file",
        ));
    }

    let copy = copy_file(
        source,
        source_path,
        destination,
        destination_path,
        options,
        cancellation,
        progress,
    )?;

    match source.remove(source_path) {
        Ok(()) => Ok(FileMoveOutcome::Completed { copy }),
        Err(cleanup_error) => Ok(FileMoveOutcome::CopiedSourcePreserved {
            copy,
            cleanup_error,
        }),
    }
}

pub fn rename_entry(backend: &dyn VfsBackend, from: &str, to: &str) -> Result<()> {
    if from == to {
        return Ok(());
    }

    backend.rename(from, to)
}

pub fn delete_entry(backend: &dyn VfsBackend, path: &str) -> Result<()> {
    backend.remove(path)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{copy_file, delete_entry, move_file, rename_entry, FileMoveOutcome};
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

    #[test]
    fn move_copies_then_removes_source() {
        let source_dir = TestDir::new();
        let destination_dir = TestDir::new();
        fs::write(source_dir.0.join("source.bin"), b"move me").expect("seed source");

        let source = LocalFsBackend::new(&source_dir.0, true).expect("source backend");
        let destination =
            LocalFsBackend::new(&destination_dir.0, true).expect("destination backend");

        let outcome = move_file(
            &source,
            "source.bin",
            &destination,
            "target.bin",
            CopyOptions::default(),
            None,
            |_| {},
        )
        .expect("move");

        assert!(matches!(outcome, FileMoveOutcome::Completed { .. }));
        assert!(outcome.source_removed());
        assert!(!source_dir.0.join("source.bin").exists());
        assert_eq!(
            fs::read(destination_dir.0.join("target.bin")).expect("read target"),
            b"move me"
        );
    }

    #[test]
    fn move_preserves_source_when_cleanup_is_denied() {
        let source_dir = TestDir::new();
        let destination_dir = TestDir::new();
        fs::write(source_dir.0.join("source.bin"), b"keep source").expect("seed source");

        let source = LocalFsBackend::new(&source_dir.0, false).expect("source backend");
        let destination =
            LocalFsBackend::new(&destination_dir.0, true).expect("destination backend");

        let outcome = move_file(
            &source,
            "source.bin",
            &destination,
            "target.bin",
            CopyOptions::default(),
            None,
            |_| {},
        )
        .expect("move returns partial outcome");

        match outcome {
            FileMoveOutcome::CopiedSourcePreserved {
                copy,
                cleanup_error,
            } => {
                assert_eq!(copy.copied, 11);
                assert_eq!(cleanup_error.kind(), ErrorKind::PermissionDenied);
            }
            FileMoveOutcome::Completed { .. } => panic!("source must be preserved"),
        }

        assert_eq!(
            fs::read(source_dir.0.join("source.bin")).expect("read source"),
            b"keep source"
        );
        assert_eq!(
            fs::read(destination_dir.0.join("target.bin")).expect("read target"),
            b"keep source"
        );
    }

    #[test]
    fn rename_and_delete_are_thin_backend_operations() {
        let temp = TestDir::new();
        fs::write(temp.0.join("before.bin"), b"x").expect("seed file");
        let backend = LocalFsBackend::new(&temp.0, true).expect("backend");

        rename_entry(&backend, "before.bin", "after.bin").expect("rename");
        assert!(!temp.0.join("before.bin").exists());
        assert!(temp.0.join("after.bin").exists());

        delete_entry(&backend, "after.bin").expect("delete");
        assert!(!temp.0.join("after.bin").exists());
    }

    #[test]
    fn move_rejects_identical_source_and_destination() {
        let temp = TestDir::new();
        fs::write(temp.0.join("same.bin"), b"x").expect("seed file");
        let backend = LocalFsBackend::new(&temp.0, true).expect("backend");

        let error = move_file(
            &backend,
            "same.bin",
            &backend,
            "same.bin",
            CopyOptions::default(),
            None,
            |_| {},
        )
        .expect_err("same path move must fail");

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert_eq!(
            fs::read(temp.0.join("same.bin")).expect("read source"),
            b"x"
        );
    }
}
