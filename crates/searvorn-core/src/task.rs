use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Default)]
pub struct CancellationFlag {
    cancelled: AtomicBool,
}

impl CancellationFlag {
    pub const fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}
