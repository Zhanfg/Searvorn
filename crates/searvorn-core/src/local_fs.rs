use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::FileExt,
    path::{Component, Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    error::{ErrorKind, Result, SearvornError},
    task::CancellationFlag,
    transfer::{copy_stream, CopyOptions, CopyProgress},
    vfs::{
        CommitOutcome, DirEntry, Metadata, NodeType, RandomRead, RandomWrite, VfsBackend,
        WriteMode, WriteTransaction,
    },
    Capability, CapabilitySet,
};

static STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CopyOutcome {
    pub copied: u64,
    pub directory_synced: bool,
}

pub struct LocalFsBackend {
    root: PathBuf,
    writable: bool,
}

impl LocalFsBackend {
    pub fn new(root: impl Into<PathBuf>, writable: bool) -> Result<Self> {
        let root = fs::canonicalize(root.into())
            .map_err(|error| SearvornError::from_io("local_fs.init", error))?;

        if !root.is_dir() {
            return Err(SearvornError::new(ErrorKind::InvalidInput, "local_fs.init"));
        }

        Ok(Self { root, writable })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn atomic_replace(&self, path: &str, bytes: &[u8]) -> Result<CommitOutcome> {
        self.atomic_write(path, |writer| {
            let mut offset = 0usize;

            while offset < bytes.len() {
                let count = writer.write_at(offset as u64, &bytes[offset..])?;
                if count == 0 {
                    return Err(SearvornError::with_detail(
                        ErrorKind::Io,
                        "local_fs.atomic_replace",
                        "writer made no progress",
                    ));
                }
                offset += count;
            }

            writer.set_len(bytes.len() as u64)
        })
    }

    pub fn atomic_copy_from<R, F>(
        &self,
        path: &str,
        reader: &mut R,
        options: CopyOptions,
        cancellation: Option<&CancellationFlag>,
        mut progress: F,
    ) -> Result<CopyOutcome>
    where
        R: RandomRead + ?Sized,
        F: FnMut(CopyProgress),
    {
        let mut copied = 0u64;
        let outcome = self.atomic_write(path, |writer| {
            copied = copy_stream(reader, writer, options, cancellation, &mut progress)?;
            Ok(())
        })?;

        Ok(CopyOutcome {
            copied,
            directory_synced: outcome.directory_synced,
        })
    }

    fn atomic_write<F>(&self, path: &str, write: F) -> Result<CommitOutcome>
    where
        F: FnOnce(&mut dyn RandomWrite) -> Result<()>,
    {
        let mut transaction = self.begin_replace(path)?;
        write(transaction.as_mut())?;
        transaction.commit()
    }

    fn create_stage(parent: &Path, file_name: &std::ffi::OsStr) -> Result<(PathBuf, File)> {
        for _ in 0..32 {
            let sequence = STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let stage_name = format!(
                ".{}.searvorn-{}-{sequence}.tmp",
                file_name.to_string_lossy(),
                process::id()
            );
            let stage = parent.join(stage_name);

            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&stage)
            {
                Ok(file) => return Ok((stage, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(SearvornError::from_io("local_fs.atomic_write", error));
                }
            }
        }

        Err(SearvornError::new(
            ErrorKind::Conflict,
            "local_fs.atomic_write",
        ))
    }

    fn lexical(&self, path: &str) -> Result<PathBuf> {
        let relative = Path::new(path.trim_start_matches('/'));
        let mut resolved = self.root.clone();

        for component in relative.components() {
            match component {
                Component::Normal(part) => resolved.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(SearvornError::new(
                        ErrorKind::InvalidInput,
                        "local_fs.resolve",
                    ));
                }
            }
        }

        Ok(resolved)
    }

    fn ensure_under_root(&self, path: PathBuf, operation: &'static str) -> Result<PathBuf> {
        if path.starts_with(&self.root) {
            Ok(path)
        } else {
            Err(SearvornError::new(ErrorKind::PermissionDenied, operation))
        }
    }

    fn resolve_entry(&self, path: &str) -> Result<PathBuf> {
        let lexical = self.lexical(path)?;

        if lexical == self.root {
            return Ok(lexical);
        }

        let parent = lexical
            .parent()
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "local_fs.resolve"))?;
        let parent = fs::canonicalize(parent)
            .map_err(|error| SearvornError::from_io("local_fs.resolve", error))?;
        let parent = self.ensure_under_root(parent, "local_fs.resolve")?;
        let name = lexical
            .file_name()
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "local_fs.resolve"))?;

        Ok(parent.join(name))
    }

    fn resolve_follow(&self, path: &str) -> Result<PathBuf> {
        let entry = self.resolve_entry(path)?;
        let canonical = fs::canonicalize(entry)
            .map_err(|error| SearvornError::from_io("local_fs.resolve", error))?;

        self.ensure_under_root(canonical, "local_fs.resolve")
    }

    fn resolve_open(&self, path: &str, mode: WriteMode) -> Result<PathBuf> {
        let entry = self.resolve_entry(path)?;

        match fs::symlink_metadata(&entry) {
            Ok(_) if mode == WriteMode::CreateNew => {
                Err(SearvornError::new(ErrorKind::Conflict, "local_fs.resolve"))
            }
            Ok(_) => {
                let canonical = fs::canonicalize(entry)
                    .map_err(|error| SearvornError::from_io("local_fs.resolve", error))?;
                self.ensure_under_root(canonical, "local_fs.resolve")
            }
            Err(error)
                if mode != WriteMode::Existing && error.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(entry)
            }
            Err(error) => Err(SearvornError::from_io("local_fs.resolve", error)),
        }
    }

    fn require_write(&self, operation: &'static str) -> Result<()> {
        if self.writable {
            Ok(())
        } else {
            Err(SearvornError::new(ErrorKind::PermissionDenied, operation))
        }
    }

    fn map_metadata(metadata: fs::Metadata) -> Metadata {
        let file_type = metadata.file_type();
        let node_type = if file_type.is_file() {
            NodeType::File
        } else if file_type.is_dir() {
            NodeType::Directory
        } else if file_type.is_symlink() {
            NodeType::Symlink
        } else {
            NodeType::Other
        };

        Metadata {
            node_type,
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

struct LocalFile {
    file: File,
}

impl RandomRead for LocalFile {
    fn len(&self) -> Result<u64> {
        self.file
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|error| SearvornError::from_io("local_fs.len", error))
    }

    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.file
            .read_at(buffer, offset)
            .map_err(|error| SearvornError::from_io("local_fs.read_at", error))
    }
}

impl RandomWrite for LocalFile {
    fn len(&self) -> Result<u64> {
        self.file
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|error| SearvornError::from_io("local_fs.len", error))
    }

    fn write_at(&mut self, offset: u64, buffer: &[u8]) -> Result<usize> {
        self.file
            .write_at(buffer, offset)
            .map_err(|error| SearvornError::from_io("local_fs.write_at", error))
    }

    fn set_len(&mut self, len: u64) -> Result<()> {
        self.file
            .set_len(len)
            .map_err(|error| SearvornError::from_io("local_fs.set_len", error))
    }

    fn flush(&mut self) -> Result<()> {
        self.file
            .sync_data()
            .map_err(|error| SearvornError::from_io("local_fs.flush", error))
    }
}

struct LocalWriteTransaction {
    writer: Option<LocalFile>,
    stage: PathBuf,
    target: PathBuf,
    parent: PathBuf,
    committed: bool,
}

impl LocalWriteTransaction {
    fn writer(&self) -> &LocalFile {
        self.writer.as_ref().expect("active write transaction")
    }

    fn writer_mut(&mut self) -> &mut LocalFile {
        self.writer.as_mut().expect("active write transaction")
    }
}

impl RandomWrite for LocalWriteTransaction {
    fn len(&self) -> Result<u64> {
        RandomWrite::len(self.writer())
    }

    fn write_at(&mut self, offset: u64, buffer: &[u8]) -> Result<usize> {
        self.writer_mut().write_at(offset, buffer)
    }

    fn set_len(&mut self, len: u64) -> Result<()> {
        self.writer_mut().set_len(len)
    }

    fn flush(&mut self) -> Result<()> {
        self.writer_mut().flush()
    }
}

impl WriteTransaction for LocalWriteTransaction {
    fn commit(mut self: Box<Self>) -> Result<CommitOutcome> {
        let writer = self
            .writer
            .take()
            .expect("active write transaction during commit");
        writer
            .file
            .sync_all()
            .map_err(|error| SearvornError::from_io("local_fs.commit", error))?;
        drop(writer);

        fs::rename(&self.stage, &self.target)
            .map_err(|error| SearvornError::from_io("local_fs.commit", error))?;
        self.committed = true;

        let directory_synced = File::open(&self.parent)
            .and_then(|directory| directory.sync_all())
            .is_ok();

        Ok(CommitOutcome {
            atomic: true,
            directory_synced,
        })
    }
}

impl Drop for LocalWriteTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.stage);
        }
    }
}

impl VfsBackend for LocalFsBackend {
    fn name(&self) -> &'static str {
        "local"
    }

    fn capabilities(&self) -> CapabilitySet {
        let base = CapabilitySet::from(Capability::FileRead);

        if self.writable {
            base.union(Capability::FileWrite.into())
        } else {
            base
        }
    }

    fn metadata(&self, path: &str) -> Result<Metadata> {
        fs::symlink_metadata(self.resolve_entry(path)?)
            .map(Self::map_metadata)
            .map_err(|error| SearvornError::from_io("local_fs.metadata", error))
    }

    fn read_dir(&self, path: &str, visitor: &mut dyn FnMut(DirEntry) -> Result<()>) -> Result<()> {
        let entries = fs::read_dir(self.resolve_follow(path)?)
            .map_err(|error| SearvornError::from_io("local_fs.read_dir", error))?;

        for entry in entries {
            let entry =
                entry.map_err(|error| SearvornError::from_io("local_fs.read_dir", error))?;
            let metadata = entry
                .metadata()
                .map_err(|error| SearvornError::from_io("local_fs.read_dir", error))?;

            visitor(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                metadata: Self::map_metadata(metadata),
            })?;
        }

        Ok(())
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn RandomRead>> {
        let file = File::open(self.resolve_follow(path)?)
            .map_err(|error| SearvornError::from_io("local_fs.open_read", error))?;

        Ok(Box::new(LocalFile { file }))
    }

    fn open_write(&self, path: &str, mode: WriteMode) -> Result<Box<dyn RandomWrite>> {
        self.require_write("local_fs.open_write")?;

        let mut options = OpenOptions::new();
        options.read(true).write(true);

        match mode {
            WriteMode::Existing => {}
            WriteMode::Create => {
                options.create(true);
            }
            WriteMode::CreateNew => {
                options.create_new(true);
            }
        }

        let file = options
            .open(self.resolve_open(path, mode)?)
            .map_err(|error| SearvornError::from_io("local_fs.open_write", error))?;

        Ok(Box::new(LocalFile { file }))
    }

    fn begin_replace(&self, path: &str) -> Result<Box<dyn WriteTransaction>> {
        self.require_write("local_fs.begin_replace")?;

        let target = self.resolve_entry(path)?;
        if target == self.root {
            return Err(SearvornError::new(
                ErrorKind::InvalidInput,
                "local_fs.begin_replace",
            ));
        }

        let parent = target
            .parent()
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "local_fs.begin_replace"))?
            .to_path_buf();
        let file_name = target
            .file_name()
            .ok_or_else(|| SearvornError::new(ErrorKind::InvalidInput, "local_fs.begin_replace"))?;
        let (stage, file) = Self::create_stage(&parent, file_name)?;

        Ok(Box::new(LocalWriteTransaction {
            writer: Some(LocalFile { file }),
            stage,
            target,
            parent,
            committed: false,
        }))
    }

    fn create_dir(&self, path: &str) -> Result<()> {
        self.require_write("local_fs.create_dir")?;

        fs::create_dir(self.resolve_entry(path)?)
            .map_err(|error| SearvornError::from_io("local_fs.create_dir", error))
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.require_write("local_fs.rename")?;

        fs::rename(self.resolve_entry(from)?, self.resolve_entry(to)?)
            .map_err(|error| SearvornError::from_io("local_fs.rename", error))
    }

    fn remove(&self, path: &str) -> Result<()> {
        self.require_write("local_fs.remove")?;
        let resolved = self.resolve_entry(path)?;
        let metadata = fs::symlink_metadata(&resolved)
            .map_err(|error| SearvornError::from_io("local_fs.remove", error))?;

        if metadata.is_dir() {
            fs::remove_dir(&resolved)
        } else {
            fs::remove_file(&resolved)
        }
        .map_err(|error| SearvornError::from_io("local_fs.remove", error))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::symlink,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::LocalFsBackend;
    use crate::{
        error::ErrorKind,
        task::CancellationFlag,
        transfer::CopyOptions,
        vfs::{NodeType, VfsBackend, WriteMode},
    };

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("searvorn-{}-{nonce}", process::id()));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }

        fn stage_count(&self) -> usize {
            fs::read_dir(&self.0)
                .expect("read test directory")
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".searvorn-"))
                .count()
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_and_writes_inside_root() {
        let temp = TestDir::new();
        fs::write(temp.0.join("input.bin"), b"abcdef").expect("seed file");

        let backend = LocalFsBackend::new(&temp.0, true).expect("backend");
        let metadata = backend.metadata("/input.bin").expect("metadata");
        assert_eq!(metadata.node_type, NodeType::File);
        assert_eq!(metadata.len, 6);

        let mut reader = backend.open_read("input.bin").expect("reader");
        let mut buffer = [0u8; 3];
        reader.read_at(2, &mut buffer).expect("read");
        assert_eq!(&buffer, b"cde");

        let mut writer = backend
            .open_write("input.bin", WriteMode::Existing)
            .expect("writer");
        writer.write_at(1, b"XY").expect("write");
        writer.flush().expect("flush");

        assert_eq!(
            fs::read(temp.0.join("input.bin")).expect("read back"),
            b"aXYdef"
        );
    }

    #[test]
    fn refuses_parent_traversal() {
        let temp = TestDir::new();
        let backend = LocalFsBackend::new(&temp.0, true).expect("backend");

        let error = backend
            .metadata("../outside")
            .expect_err("must reject traversal");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn refuses_symlink_escape_on_open() {
        let root = TestDir::new();
        let outside = TestDir::new();
        let secret = outside.0.join("secret");
        fs::write(&secret, b"outside").expect("seed outside file");
        symlink(&secret, root.0.join("link")).expect("create symlink");

        let backend = LocalFsBackend::new(&root.0, true).expect("backend");
        let error = match backend.open_read("link") {
            Ok(_) => panic!("must reject symlink escape"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn read_only_backend_rejects_mutation() {
        let temp = TestDir::new();
        fs::write(temp.0.join("input.bin"), b"x").expect("seed file");
        let backend = LocalFsBackend::new(&temp.0, false).expect("backend");

        let error = match backend.open_write("input.bin", WriteMode::Existing) {
            Ok(_) => panic!("must reject write"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn create_mode_opens_missing_file() {
        let temp = TestDir::new();
        let backend = LocalFsBackend::new(&temp.0, true).expect("backend");

        let mut writer = backend
            .open_write("created.bin", WriteMode::Create)
            .expect("create writer");
        writer.write_at(0, b"created").expect("write");
        writer.flush().expect("flush");

        assert_eq!(
            fs::read(temp.0.join("created.bin")).expect("read created file"),
            b"created"
        );
    }

    #[test]
    fn create_new_rejects_existing_file() {
        let temp = TestDir::new();
        fs::write(temp.0.join("input.bin"), b"old").expect("seed file");
        let backend = LocalFsBackend::new(&temp.0, true).expect("backend");

        let error = match backend.open_write("input.bin", WriteMode::CreateNew) {
            Ok(_) => panic!("must reject existing file"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), ErrorKind::Conflict);
    }

    #[test]
    fn atomic_replace_commits_complete_contents() {
        let temp = TestDir::new();
        fs::write(temp.0.join("input.bin"), b"old").expect("seed file");
        let backend = LocalFsBackend::new(&temp.0, true).expect("backend");

        backend
            .atomic_replace("input.bin", b"new contents")
            .expect("replace");

        assert_eq!(
            fs::read(temp.0.join("input.bin")).expect("read back"),
            b"new contents"
        );
        assert_eq!(temp.stage_count(), 0);
    }

    #[test]
    fn atomic_copy_streams_and_commits() {
        let temp = TestDir::new();
        let input = vec![0x5a; 1024 * 1024 + 17];
        fs::write(temp.0.join("source.bin"), &input).expect("seed source");
        fs::write(temp.0.join("target.bin"), b"old target").expect("seed target");
        let backend = LocalFsBackend::new(&temp.0, true).expect("backend");
        let mut reader = backend.open_read("source.bin").expect("source reader");
        let mut last_progress = 0;

        let outcome = backend
            .atomic_copy_from(
                "target.bin",
                reader.as_mut(),
                CopyOptions::default(),
                None,
                |progress| last_progress = progress.copied,
            )
            .expect("copy");

        assert_eq!(outcome.copied, input.len() as u64);
        assert_eq!(last_progress, input.len() as u64);
        assert_eq!(
            fs::read(temp.0.join("target.bin")).expect("read target"),
            input
        );
        assert_eq!(temp.stage_count(), 0);
    }

    #[test]
    fn cancelled_atomic_copy_preserves_target() {
        let temp = TestDir::new();
        fs::write(temp.0.join("source.bin"), vec![3; 128 * 1024]).expect("seed source");
        fs::write(temp.0.join("target.bin"), b"keep me").expect("seed target");
        let backend = LocalFsBackend::new(&temp.0, true).expect("backend");
        let mut reader = backend.open_read("source.bin").expect("source reader");
        let cancellation = CancellationFlag::new();

        let error = backend
            .atomic_copy_from(
                "target.bin",
                reader.as_mut(),
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
            fs::read(temp.0.join("target.bin")).expect("read target"),
            b"keep me"
        );
        assert_eq!(temp.stage_count(), 0);
    }
}
