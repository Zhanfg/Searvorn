use core::ffi::c_void;
use std::{
    fs::File,
    os::fd::{FromRawFd, OwnedFd},
    sync::{Mutex, MutexGuard, OnceLock},
};

use crate::handle_table::{Handle, HandleTable};

pub const NATIVE_ABI_VERSION: i32 = 1;

static FILE_HANDLES: OnceLock<Mutex<HandleTable<File>>> = OnceLock::new();

#[unsafe(no_mangle)]
pub extern "system" fn Java_cc_axymorrsen_searvorn_NativeCore_nativeAbiVersion(
    _env: *mut c_void,
    _receiver: *mut c_void,
) -> i32 {
    NATIVE_ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cc_axymorrsen_searvorn_NativeCore_nativeAdoptFd(
    _env: *mut c_void,
    _receiver: *mut c_void,
    fd: i32,
) -> i64 {
    if fd < 0 {
        return 0;
    }

    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let file = File::from(owned);

    lock_handles()
        .insert(file)
        .ok()
        .and_then(|handle| i64::try_from(handle.raw()).ok())
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cc_axymorrsen_searvorn_NativeCore_nativeHandleLen(
    _env: *mut c_void,
    _receiver: *mut c_void,
    raw_handle: i64,
) -> i64 {
    let Ok(raw_handle) = u64::try_from(raw_handle) else {
        return -1;
    };
    let Some(handle) = Handle::from_raw(raw_handle) else {
        return -1;
    };

    lock_handles()
        .get(handle)
        .and_then(|file| file.metadata().ok())
        .and_then(|metadata| i64::try_from(metadata.len()).ok())
        .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cc_axymorrsen_searvorn_NativeCore_nativeReleaseHandle(
    _env: *mut c_void,
    _receiver: *mut c_void,
    raw_handle: i64,
) -> i32 {
    let Ok(raw_handle) = u64::try_from(raw_handle) else {
        return 0;
    };
    let Some(handle) = Handle::from_raw(raw_handle) else {
        return 0;
    };

    i32::from(lock_handles().remove(handle).is_some())
}

fn lock_handles() -> MutexGuard<'static, HandleTable<File>> {
    FILE_HANDLES
        .get_or_init(|| Mutex::new(HandleTable::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
