use std::{fmt, io};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    NotFound,
    PermissionDenied,
    Unsupported,
    InvalidInput,
    Conflict,
    Interrupted,
    Io,
}

#[derive(Debug)]
pub struct SearvornError {
    kind: ErrorKind,
    operation: &'static str,
    detail: Option<String>,
}

impl SearvornError {
    pub fn new(kind: ErrorKind, operation: &'static str) -> Self {
        Self {
            kind,
            operation,
            detail: None,
        }
    }

    pub fn with_detail(
        kind: ErrorKind,
        operation: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            operation,
            detail: Some(detail.into()),
        }
    }

    pub fn from_io(operation: &'static str, error: io::Error) -> Self {
        let kind = match error.kind() {
            io::ErrorKind::NotFound => ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied => ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => ErrorKind::InvalidInput,
            io::ErrorKind::AlreadyExists => ErrorKind::Conflict,
            io::ErrorKind::Interrupted => ErrorKind::Interrupted,
            io::ErrorKind::Unsupported => ErrorKind::Unsupported,
            _ => ErrorKind::Io,
        };

        Self::with_detail(kind, operation, error.to_string())
    }

    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

impl fmt::Display for SearvornError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {:?}", self.operation, self.kind)?;

        if let Some(detail) = &self.detail {
            write!(f, " ({detail})")?;
        }

        Ok(())
    }
}

impl std::error::Error for SearvornError {}

pub type Result<T> = std::result::Result<T, SearvornError>;
