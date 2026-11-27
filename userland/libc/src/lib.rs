//! goethite 简易 POSIX 兼容 C 标准库（`libc`）。
//!
//! 编译产出 `libc.a`（`staticlib`）与 `libc.rlib`，在内核原生的 `goethite-stack`
//! 结构化栈映像与 Rust 风格系统调用 ABI 之上提供尽量符合 POSIX 的 C 接口转换层：
//! - `crt0`（`_start`）：还原 C 风格 `argc` / `argv` / `environ` 并调用 `main(argc, argv, envp)`；
//! - `env`：`environ`、`getenv`、`setenv`、`unsetenv`、`putenv`、`clearenv`；
//! - `alloc`：`malloc`、`free`、`calloc`、`realloc`；
//! - `unistd`：`open`、`close`、`read`、`write`、`fork`、`execve`、`waitpid`、`wait`、`sched_yield`；
//! - `stdlib`：`atoi`、`exit`、`_exit`、`abort`；
//! - `string`：`strlen`、`strnlen`、`strcmp`、`strncmp`、`strcpy`、`strncpy`、`strchr`、`memcpy`、`memmove`、`memset`、`memcmp`；
//! - `stdio`：`putchar`、`puts`；
//! - `errno`：`errno`、`__errno_location`。

#![cfg_attr(target_arch = "riscv64", no_std)]

pub mod alloc;
pub mod crt0;
pub mod env;
pub mod errno;
pub mod stdio;
pub mod stdlib;
pub mod string;
pub mod unistd;

#[cfg(all(target_arch = "riscv64", not(test)))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    stdlib::abort()
}
