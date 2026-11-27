//! 简易 POSIX `<stdio.h>` 输出函数（`putchar`、`puts`）。

#![cfg(target_arch = "riscv64")]

use core::ffi::{c_char, c_int, c_void};

use crate::{string::strlen, unistd::write};

const STDOUT_FD: c_int = 1;
const EOF: c_int = -1;

#[unsafe(no_mangle)]
pub extern "C" fn putchar(c: c_int) -> c_int {
    let byte = c as u8;
    let n = unsafe { write(STDOUT_FD, (&byte as *const u8) as *const c_void, 1) };
    if n == 1 { byte as c_int } else { EOF }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn puts(s: *const c_char) -> c_int {
    if s.is_null() {
        return EOF;
    }
    let len = unsafe { strlen(s) };
    if len > 0 {
        let n = unsafe { write(STDOUT_FD, s as *const c_void, len) };
        if n < 0 {
            return EOF;
        }
    }
    if putchar(b'\n' as c_int) == EOF {
        EOF
    } else {
        0
    }
}
