use std::time::SystemTime;

use crate::{error::Result, CapabilitySet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeType {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug)]
pub struct Metadata {
    pub node_type: NodeType,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub metadata: Metadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteMode {
    Existing,
    Create,
    CreateNew,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitOutcome {
    pub atomic: bool,
    pub directory_synced: bool,
}

pub trait RandomRead: Send {
    fn len(&self) -> Result<u64>;
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize>;

    fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }
}

pub trait RandomWrite: Send {
    fn len(&self) -> Result<u64>;
    fn write_at(&mut self, offset: u64, buffer: &[u8]) -> Result<usize>;
    fn set_len(&mut self, len: u64) -> Result<()>;
    fn flush(&mut self) -> Result<()>;

    fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }
}

pub trait WriteTransaction: RandomWrite {
    fn commit(self: Box<Self>) -> Result<CommitOutcome>;
}

pub trait VfsBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> CapabilitySet;

    fn metadata(&self, path: &str) -> Result<Metadata>;

    fn read_dir(&self, path: &str, visitor: &mut dyn FnMut(DirEntry) -> Result<()>) -> Result<()>;

    fn open_read(&self, path: &str) -> Result<Box<dyn RandomRead>>;
    fn open_write(&self, path: &str, mode: WriteMode) -> Result<Box<dyn RandomWrite>>;
    fn begin_replace(&self, path: &str) -> Result<Box<dyn WriteTransaction>>;

    fn create_dir(&self, path: &str) -> Result<()>;
    fn rename(&self, from: &str, to: &str) -> Result<()>;
    fn remove(&self, path: &str) -> Result<()>;
}
