//! POSIX `<unistd.h>` / `<fcntl.h>` / `<sys/wait.h>` 系统调用转换层。
//!
//! 将 C 风格参数（NUL 结尾字符串、`char *const argv[]`、`"KEY=VALUE"` `char *const envp[]`）
//! 转换为 goethite 原生切片 ABI，并将 `PosixErrno` 转换为 `-1` + `errno`。

#![cfg(target_arch = "riscv64")]

use core::{
    ffi::{c_char, c_int, c_uint, c_void},
    mem::size_of,
    slice, str,
};

use goethite_abi::{PosixErrno, SysResult, SyscallOps};
use goethite_syscall::Syscalls;

use crate::{
    alloc::{free, malloc},
    errno::set_errno,
    string::strlen,
};

#[inline]
fn cvt_i32<T: TryInto<c_int>>(res: SysResult<T>) -> c_int {
    match res {
        Ok(v) => v.try_into().unwrap_or(0),
        Err(e) => {
            set_errno(e.0 as c_int);
            -1
        }
    }
}

#[inline]
fn cvt_isize(res: SysResult<usize>) -> isize {
    match res {
        Ok(v) => v as isize,
        Err(e) => {
            set_errno(e.0 as c_int);
            -1
        }
    }
}

unsafe fn cstr_to_str<'a>(s: *const c_char) -> Result<&'a str, PosixErrno> {
    if s.is_null() {
        return Err(PosixErrno::EFAULT);
    }
    let len = unsafe { strlen(s) };
    let bytes = unsafe { slice::from_raw_parts(s as *const u8, len) };
    str::from_utf8(bytes).map_err(|_| PosixErrno::EINVAL)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: c_uint) -> c_int {
    let Ok(path_str) = (unsafe { cstr_to_str(path) }) else {
        set_errno(PosixErrno::EINVAL.0 as c_int);
        return -1;
    };
    cvt_i32(Syscalls::open(path_str, flags as u32, mode as u16))
}

#[unsafe(no_mangle)]
pub extern "C" fn close(fd: c_int) -> c_int {
    if fd < 0 {
        set_errno(PosixErrno::EBADF.0 as c_int);
        return -1;
    }
    match Syscalls::close(fd as usize) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(e.0 as c_int);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize {
    if fd < 0 {
        set_errno(PosixErrno::EBADF.0 as c_int);
        return -1;
    }
    if buf.is_null() && count != 0 {
        set_errno(PosixErrno::EFAULT.0 as c_int);
        return -1;
    }
    let dst = unsafe { slice::from_raw_parts_mut(buf as *mut u8, count) };
    cvt_isize(Syscalls::read(fd as usize, dst))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, count: usize) -> isize {
    if fd < 0 {
        set_errno(PosixErrno::EBADF.0 as c_int);
        return -1;
    }
    if buf.is_null() && count != 0 {
        set_errno(PosixErrno::EFAULT.0 as c_int);
        return -1;
    }
    let src = unsafe { slice::from_raw_parts(buf as *const u8, count) };
    cvt_isize(Syscalls::write(fd as usize, src))
}

#[unsafe(no_mangle)]
pub extern "C" fn fork() -> c_int {
    cvt_i32(Syscalls::spawn_process())
}

/// POSIX `execve`：把 C 风格 `argv` / `envp`（`"K=V"`）转换为 goethite 原生 `&[&str]` / `&[(&str, &str)]`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execve(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    let path_str = match unsafe { cstr_to_str(path) } {
        Ok(s) => s,
        Err(e) => {
            set_errno(e.0 as c_int);
            return -1;
        }
    };

    // 统计 argv 长度
    let mut argc = 0usize;
    if !argv.is_null() {
        unsafe {
            while !(*argv.add(argc)).is_null() {
                argc += 1;
            }
        }
    }
    let args_buf = if argc > 0 {
        let p = unsafe { malloc(argc * size_of::<&str>()) as *mut &str };
        if p.is_null() {
            set_errno(PosixErrno::ENOMEM.0 as c_int);
            return -1;
        }
        for i in 0..argc {
            match unsafe { cstr_to_str(*argv.add(i)) } {
                Ok(s) => unsafe { p.add(i).write(s) },
                Err(e) => {
                    unsafe { free(p as *mut c_void) };
                    set_errno(e.0 as c_int);
                    return -1;
                }
            }
        }
        p
    } else {
        core::ptr::null_mut()
    };

    // 统计 envp 长度并拆分为 (&str, &str)
    let mut env_cap = 0usize;
    if !envp.is_null() {
        unsafe {
            while !(*envp.add(env_cap)).is_null() {
                env_cap += 1;
            }
        }
    }
    let mut env_count = 0usize;
    let env_buf = if env_cap > 0 {
        let p = unsafe { malloc(env_cap * size_of::<(&str, &str)>()) as *mut (&str, &str) };
        if p.is_null() {
            if !args_buf.is_null() {
                unsafe { free(args_buf as *mut c_void) };
            }
            set_errno(PosixErrno::ENOMEM.0 as c_int);
            return -1;
        }
        for i in 0..env_cap {
            match unsafe { cstr_to_str(*envp.add(i)) } {
                Ok(entry) => {
                    if let Some((k, v)) = entry.split_once('=') {
                        unsafe { p.add(env_count).write((k, v)) };
                        env_count += 1;
                    }
                }
                Err(e) => {
                    unsafe {
                        free(p as *mut c_void);
                        if !args_buf.is_null() {
                            free(args_buf as *mut c_void);
                        }
                    }
                    set_errno(e.0 as c_int);
                    return -1;
                }
            }
        }
        p
    } else {
        core::ptr::null_mut()
    };

    let args_slice = if argc > 0 {
        unsafe { slice::from_raw_parts(args_buf, argc) }
    } else {
        &[]
    };
    let env_slice = if env_count > 0 {
        unsafe { slice::from_raw_parts(env_buf, env_count) }
    } else {
        &[]
    };

    let res = Syscalls::execve(path_str, args_slice, env_slice);

    // 仅在 execve 失败返回时释放临时缓冲
    unsafe {
        if !env_buf.is_null() {
            free(env_buf as *mut c_void);
        }
        if !args_buf.is_null() {
            free(args_buf as *mut c_void);
        }
    }
    cvt_i32(res)
}

/// POSIX `WNOHANG` 标志。
pub const WNOHANG: c_int = 1;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int {
    let target = if pid <= 0 { 0usize } else { pid as usize };
    loop {
        match Syscalls::wait(target) {
            Ok(tid) => {
                if !status.is_null() {
                    unsafe { *status = 0 };
                }
                return tid as c_int;
            }
            Err(PosixErrno::EAGAIN) => {
                if (options & WNOHANG) != 0 {
                    return 0;
                }
                let _ = Syscalls::yield_now();
            }
            Err(e) => {
                set_errno(e.0 as c_int);
                return -1;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wait(status: *mut c_int) -> c_int {
    unsafe { waitpid(-1, status, 0) }
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_yield() -> c_int {
    match Syscalls::yield_now() {
        Ok(()) => 0,
        Err(e) => {
            set_errno(e.0 as c_int);
            -1
        }
    }
}
