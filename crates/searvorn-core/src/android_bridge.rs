use core::ffi::c_void;

pub const NATIVE_ABI_VERSION: i32 = 1;

#[unsafe(no_mangle)]
pub extern "system" fn Java_cc_axymorrsen_searvorn_NativeCore_nativeAbiVersion(
    _env: *mut c_void,
    _receiver: *mut c_void,
) -> i32 {
    NATIVE_ABI_VERSION
}
