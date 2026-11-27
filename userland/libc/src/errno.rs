//! POSIX `errno` 支持。

use core::ffi::c_int;

/// 全局错误码（单线程用户进程模型）。
#[allow(non_upper_case_globals)]
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub static mut errno: c_int = 0;

/// 返回当前线程 `errno` 的地址（glibc/musl 兼容符号）。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn __errno_location() -> *mut c_int {
    core::ptr::addr_of_mut!(errno)
}

#[inline]
pub fn set_errno(val: c_int) {
    unsafe {
        *__errno_location() = val;
    }
}

#[inline]
pub fn get_errno() -> c_int {
    unsafe { *__errno_location() }
}
