//! POSIX 环境变量接口：`environ`、`getenv`、`setenv`、`unsetenv`、`putenv`、`clearenv`。
//!
//! 将内核提供的结构化 [`goethite_stack::StackRef`] 环境变量转换为标准 POSIX 的
//! `char **environ`（以 `NULL` 结尾的 `"KEY=VALUE\0"` 指针数组），并在堆上维护
//! 可变副本，保证直接遍历 `environ` 与调用 `getenv`/`setenv` 始终一致。

use core::{
    ffi::{c_char, c_int, c_void},
    mem::size_of,
    ptr, slice,
};

use goethite_abi::PosixErrno;
use goethite_stack::StackRef;

use crate::{
    alloc::{free, malloc, realloc},
    errno::set_errno,
    string::strlen,
};

/// POSIX 全局环境变量指针数组（以 `NULL` 结尾，每项形如 `"KEY=VALUE\0"`）。
#[allow(non_upper_case_globals)]
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub static mut environ: *mut *mut c_char = ptr::null_mut();

/// 内部维护的 `environ` 容量与长度（不含末尾 `NULL`）。
static mut ENV_LEN: usize = 0;
static mut ENV_CAP: usize = 0;

/// 辅助：检查 `entry` 是否匹配 `name` 键（即 `entry` 以 `name=` 开头），匹配则返回 `=` 后的指针。
unsafe fn match_entry(entry: *mut c_char, name: &[u8]) -> Option<*mut c_char> {
    if entry.is_null() {
        return None;
    }
    let len = name.len();
    for (i, &b) in name.iter().enumerate() {
        let ch = unsafe { *entry.add(i) as u8 };
        if ch != b {
            return None;
        }
    }
    if unsafe { *entry.add(len) as u8 } == b'=' {
        Some(unsafe { entry.add(len + 1) })
    } else {
        None
    }
}

/// 分配并填充 `"key=val\0"` 字符串。
unsafe fn alloc_kv_cstr(key: &[u8], val: &[u8]) -> *mut c_char {
    let total = key.len() + 1 + val.len() + 1;
    let buf = unsafe { malloc(total) as *mut u8 };
    if buf.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        ptr::copy_nonoverlapping(key.as_ptr(), buf, key.len());
        *buf.add(key.len()) = b'=';
        ptr::copy_nonoverlapping(val.as_ptr(), buf.add(key.len() + 1), val.len());
        *buf.add(key.len() + 1 + val.len()) = 0;
    }
    buf as *mut c_char
}

/// 确保 `environ` 数组至少能容纳 `needed` 条有效项（外加末尾 `NULL`）。
unsafe fn ensure_cap(needed: usize) -> bool {
    unsafe {
        if needed <= ENV_CAP && !environ.is_null() {
            return true;
        }
        let new_cap = (ENV_CAP.max(8) * 2).max(needed);
        let bytes = (new_cap + 1) * size_of::<*mut c_char>();
        let new_arr = if environ.is_null() {
            malloc(bytes) as *mut *mut c_char
        } else {
            realloc(environ as *mut c_void, bytes) as *mut *mut c_char
        };
        if new_arr.is_null() {
            return false;
        }
        environ = new_arr;
        ENV_CAP = new_cap;
        *environ.add(ENV_LEN) = ptr::null_mut();
        true
    }
}

/// 从 `goethite-stack` 零拷贝视图初始化 POSIX `environ`。
pub unsafe fn init_from_stack(stack: &StackRef<'_>) {
    unsafe {
        let _ = clearenv();
        let count = stack.env_count();
        if !ensure_cap(count) {
            return;
        }
        for (k, v) in stack.env() {
            let entry = alloc_kv_cstr(k.as_bytes(), v.as_bytes());
            if entry.is_null() {
                break;
            }
            *environ.add(ENV_LEN) = entry;
            ENV_LEN += 1;
        }
        *environ.add(ENV_LEN) = ptr::null_mut();
    }
}

/// POSIX `getenv`。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn getenv(name: *const c_char) -> *mut c_char {
    if name.is_null() {
        return ptr::null_mut();
    }
    let name_len = unsafe { strlen(name) };
    if name_len == 0 {
        return ptr::null_mut();
    }
    let name_bytes = unsafe { slice::from_raw_parts(name as *const u8, name_len) };
    if name_bytes.contains(&b'=') {
        return ptr::null_mut();
    }

    unsafe {
        let arr = environ;
        if arr.is_null() {
            return ptr::null_mut();
        }
        let mut i = 0usize;
        while !(*arr.add(i)).is_null() {
            if let Some(val) = match_entry(*arr.add(i), name_bytes) {
                return val;
            }
            i += 1;
        }
    }
    ptr::null_mut()
}

/// POSIX `setenv`。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn setenv(
    name: *const c_char,
    value: *const c_char,
    overwrite: c_int,
) -> c_int {
    if name.is_null() || value.is_null() {
        set_errno(PosixErrno::EINVAL.0 as c_int);
        return -1;
    }
    let name_len = unsafe { strlen(name) };
    let name_bytes = unsafe { slice::from_raw_parts(name as *const u8, name_len) };
    if name_len == 0 || name_bytes.contains(&b'=') {
        set_errno(PosixErrno::EINVAL.0 as c_int);
        return -1;
    }
    let val_len = unsafe { strlen(value) };
    let val_bytes = unsafe { slice::from_raw_parts(value as *const u8, val_len) };

    unsafe {
        if !environ.is_null() {
            for i in 0..ENV_LEN {
                if match_entry(*environ.add(i), name_bytes).is_some() {
                    if overwrite == 0 {
                        return 0;
                    }
                    let new_entry = alloc_kv_cstr(name_bytes, val_bytes);
                    if new_entry.is_null() {
                        set_errno(PosixErrno::ENOMEM.0 as c_int);
                        return -1;
                    }
                    *environ.add(i) = new_entry;
                    return 0;
                }
            }
        }

        if !ensure_cap(ENV_LEN + 1) {
            set_errno(PosixErrno::ENOMEM.0 as c_int);
            return -1;
        }
        let new_entry = alloc_kv_cstr(name_bytes, val_bytes);
        if new_entry.is_null() {
            set_errno(PosixErrno::ENOMEM.0 as c_int);
            return -1;
        }
        *environ.add(ENV_LEN) = new_entry;
        ENV_LEN += 1;
        *environ.add(ENV_LEN) = ptr::null_mut();
        0
    }
}

/// POSIX `unsetenv`。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn unsetenv(name: *const c_char) -> c_int {
    if name.is_null() {
        set_errno(PosixErrno::EINVAL.0 as c_int);
        return -1;
    }
    let name_len = unsafe { strlen(name) };
    let name_bytes = unsafe { slice::from_raw_parts(name as *const u8, name_len) };
    if name_len == 0 || name_bytes.contains(&b'=') {
        set_errno(PosixErrno::EINVAL.0 as c_int);
        return -1;
    }

    unsafe {
        if environ.is_null() {
            return 0;
        }
        let mut i = 0usize;
        while i < ENV_LEN {
            if match_entry(*environ.add(i), name_bytes).is_some() {
                for j in i..ENV_LEN - 1 {
                    *environ.add(j) = *environ.add(j + 1);
                }
                ENV_LEN -= 1;
                *environ.add(ENV_LEN) = ptr::null_mut();
            } else {
                i += 1;
            }
        }
    }
    0
}

/// POSIX `putenv`：直接将调用方提供的 `"KEY=VALUE\0"` 指针放入 `environ`（无 `'='` 时删除该键）。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn putenv(string: *mut c_char) -> c_int {
    if string.is_null() {
        set_errno(PosixErrno::EINVAL.0 as c_int);
        return -1;
    }
    let len = unsafe { strlen(string) };
    let bytes = unsafe { slice::from_raw_parts(string as *const u8, len) };

    let Some(eq_pos) = bytes.iter().position(|&b| b == b'=') else {
        return unsafe { unsetenv(string) };
    };
    if eq_pos == 0 {
        set_errno(PosixErrno::EINVAL.0 as c_int);
        return -1;
    }
    let key = &bytes[..eq_pos];

    unsafe {
        if !environ.is_null() {
            for i in 0..ENV_LEN {
                if match_entry(*environ.add(i), key).is_some() {
                    *environ.add(i) = string;
                    return 0;
                }
            }
        }
        if !ensure_cap(ENV_LEN + 1) {
            set_errno(PosixErrno::ENOMEM.0 as c_int);
            return -1;
        }
        *environ.add(ENV_LEN) = string;
        ENV_LEN += 1;
        *environ.add(ENV_LEN) = ptr::null_mut();
        0
    }
}

/// 清空整个 `environ` 表（glibc 扩展 `clearenv`）。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn clearenv() -> c_int {
    unsafe {
        if !environ.is_null() {
            free(environ as *mut c_void);
            environ = ptr::null_mut();
        }
        ENV_LEN = 0;
        ENV_CAP = 0;
    }
    0
}
