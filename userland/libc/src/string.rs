//! POSIX `<string.h>` 基础内存与 C 字符串操作。

use core::ffi::{c_char, c_int, c_void};

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut n = 0usize;
    unsafe {
        while *s.add(n) != 0 {
            n += 1;
        }
    }
    n
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn strnlen(s: *const c_char, maxlen: usize) -> usize {
    let mut n = 0usize;
    unsafe {
        while n < maxlen && *s.add(n) != 0 {
            n += 1;
        }
    }
    n
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int {
    let mut i = 0usize;
    unsafe {
        loop {
            let a = *s1.add(i) as u8;
            let b = *s2.add(i) as u8;
            if a != b || a == 0 {
                return (a as c_int) - (b as c_int);
            }
            i += 1;
        }
    }
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int {
    let mut i = 0usize;
    unsafe {
        while i < n {
            let a = *s1.add(i) as u8;
            let b = *s2.add(i) as u8;
            if a != b || a == 0 {
                return (a as c_int) - (b as c_int);
            }
            i += 1;
        }
    }
    0
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    let mut i = 0usize;
    unsafe {
        loop {
            let c = *src.add(i);
            *dst.add(i) = c;
            if c == 0 {
                break;
            }
            i += 1;
        }
    }
    dst
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn strncpy(dst: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    let mut i = 0usize;
    unsafe {
        while i < n {
            let c = *src.add(i);
            *dst.add(i) = c;
            i += 1;
            if c == 0 {
                break;
            }
        }
        while i < n {
            *dst.add(i) = 0;
            i += 1;
        }
    }
    dst
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    let target = c as u8;
    let mut i = 0usize;
    unsafe {
        loop {
            let ch = *s.add(i) as u8;
            if ch == target {
                return s.add(i) as *mut c_char;
            }
            if ch == 0 {
                return core::ptr::null_mut();
            }
            i += 1;
        }
    }
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let d = dst as *mut u8;
    let s = src as *const u8;
    let mut i = 0usize;
    unsafe {
        while i < n {
            *d.add(i) = *s.add(i);
            i += 1;
        }
    }
    dst
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let d = dst as *mut u8;
    let s = src as *const u8;
    unsafe {
        if (d as usize) < (s as usize) {
            let mut i = 0usize;
            while i < n {
                *d.add(i) = *s.add(i);
                i += 1;
            }
        } else if (d as usize) > (s as usize) {
            let mut i = n;
            while i > 0 {
                i -= 1;
                *d.add(i) = *s.add(i);
            }
        }
    }
    dst
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    let d = s as *mut u8;
    let byte = c as u8;
    let mut i = 0usize;
    unsafe {
        while i < n {
            *d.add(i) = byte;
            i += 1;
        }
    }
    s
}

#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    let a = s1 as *const u8;
    let b = s2 as *const u8;
    let mut i = 0usize;
    unsafe {
        while i < n {
            let va = *a.add(i);
            let vb = *b.add(i);
            if va != vb {
                return (va as c_int) - (vb as c_int);
            }
            i += 1;
        }
    }
    0
}
