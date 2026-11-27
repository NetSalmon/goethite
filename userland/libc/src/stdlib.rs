//! POSIX `<stdlib.h>` 基础函数（`atoi`、`exit`、`_exit`、`abort`）。

use core::ffi::{c_char, c_int};

/// POSIX `atoi`。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn atoi(nptr: *const c_char) -> c_int {
    if nptr.is_null() {
        return 0;
    }
    let mut i = 0usize;
    unsafe {
        while matches!(*nptr.add(i) as u8, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            i += 1;
        }
        let mut neg = false;
        match *nptr.add(i) as u8 {
            b'-' => {
                neg = true;
                i += 1;
            }
            b'+' => {
                i += 1;
            }
            _ => {}
        }
        let mut acc: i64 = 0;
        loop {
            let ch = *nptr.add(i) as u8;
            if !ch.is_ascii_digit() {
                break;
            }
            acc = acc.wrapping_mul(10).wrapping_add((ch - b'0') as i64);
            i += 1;
        }
        if neg { (-acc) as c_int } else { acc as c_int }
    }
}

#[cfg(target_arch = "riscv64")]
#[unsafe(no_mangle)]
pub extern "C" fn _exit(status: c_int) -> ! {
    use goethite_abi::SyscallOps;
    let _ = goethite_syscall::Syscalls::exit(status);
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(target_arch = "riscv64")]
#[unsafe(no_mangle)]
pub extern "C" fn exit(status: c_int) -> ! {
    _exit(status)
}

#[cfg(target_arch = "riscv64")]
#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
    _exit(134)
}
