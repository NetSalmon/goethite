//! goethite Rust 原生用户态运行时（`librs`）。
//!
//! 面向纯 Rust 用户程序，**不做 C / POSIX ABI 转换**：
//! - `_start` 直接读取 `sp` 指向的 [`goethite_stack`] 栈映像地址并调用 `main()`，
//!   不在栈上重建 C 风格 `argv` / `envp` 指针数组；
//! - 内置静态 `.bss` 堆分配器（[`heap::LockedHeap`]），支持 `alloc`；
//! - 通过 [`env`] 模块提供懒加载、可增删改的全局变量表，并可直接以 `&[(&str, &str)]`
//!   传入 `execve`；
//! - 保留 [`stack`] / [`args`] / [`initial_envs`] / [`auxv`] 零拷贝只读视图。

#![no_std]

pub extern crate alloc;

pub mod env;
pub mod heap;
pub mod sync;

use core::sync::atomic::{AtomicPtr, Ordering};

pub use goethite_abi::{Fd, ForkFlags, PosixErrno, SysResult, SyscallOps};
pub use goethite_stack::{
    AT_BASE, AT_BASE_PLATFORM, AT_CLKTCK, AT_EGID, AT_ENTRY, AT_EUID, AT_EXECFD, AT_EXECFN,
    AT_FLAGS, AT_GID, AT_HWCAP, AT_HWCAP2, AT_IGNORE, AT_NOTELF, AT_NULL, AT_PAGESZ, AT_PHDR,
    AT_PHENT, AT_PHNUM, AT_PLATFORM, AT_RANDOM, AT_SECURE, AT_SYSINFO, AT_SYSINFO_EHDR, AT_UID,
    ArgsIter, AuxEntry, AuxvIter, EnvIter, Error as StackError, StackRef,
};

#[cfg(target_arch = "riscv64")]
pub use goethite_syscall::Syscalls;

/// 标准输入文件描述符。
pub const STDIN: usize = 0;
/// 标准输出文件描述符。
pub const STDOUT: usize = 1;
/// 标准错误文件描述符。
pub const STDERR: usize = 2;

/// 初始栈映像起始地址；由 `_start` 在进入 `main` 前写入，未初始化时为 null。
static STACK_IMAGE: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

/// 显式设置初始栈映像指针（由 `_start` 或测试代码调用）。
///
/// # Safety
///
/// `image` 必须为 null，或指向一份在整个访问期间有效且合法的 `goethite-stack` 映像。
pub unsafe fn set_stack_image(image: *const u8) {
    STACK_IMAGE.store(image as *mut u8, Ordering::Release);
}

/// 从映像起始地址读取头部中的 `total_size`，据此构造整段映像字节切片。
///
/// # Safety
///
/// `image` 必须指向一份至少 [`goethite_stack::HEADER_SIZE`] 字节、且头部
/// `total_size` 字段描述的长度范围内均可读的合法栈映像。
unsafe fn image_bytes(image: *const u8) -> &'static [u8] {
    let total = u32::from_le_bytes([
        unsafe { *image.add(8) },
        unsafe { *image.add(9) },
        unsafe { *image.add(10) },
        unsafe { *image.add(11) },
    ]) as usize;
    unsafe { core::slice::from_raw_parts(image, total) }
}

/// 解析进程初始栈映像，返回零拷贝视图；尚未初始化或映像非法时返回 `None`。
pub fn stack() -> Option<StackRef<'static>> {
    let image = STACK_IMAGE.load(Ordering::Acquire);
    if image.is_null() {
        return None;
    }
    StackRef::parse(unsafe { image_bytes(image) }).ok()
}

/// 迭代命令行参数（`argv`），产出 `&str`。
pub fn args() -> Option<ArgsIter<'static>> {
    stack().map(|stack| stack.argv())
}

/// 迭代初始栈映像中的只读环境变量，产出 `(&str, &str)`。
///
/// 如需读取包含运行期修改的最新环境变量，请使用 [`env::get`] / [`env::vars`]。
pub fn initial_envs() -> Option<EnvIter<'static>> {
    stack().map(|stack| stack.env())
}

/// [`initial_envs`] 的兼容别名。
pub fn envs() -> Option<EnvIter<'static>> {
    initial_envs()
}

/// 迭代辅助向量（`auxv`），产出 `(u64, u64)`。
pub fn auxv() -> Option<AuxvIter<'static>> {
    stack().map(|stack| stack.auxv())
}

#[cfg(target_arch = "riscv64")]
mod runtime {
    use core::{
        arch::{asm, naked_asm},
        fmt::{self, Write},
        panic::PanicInfo,
    };

    use goethite_abi::SyscallOps;
    use goethite_syscall::Syscalls;

    use crate::{STDERR, STDOUT, heap::LockedHeap, set_stack_image};

    #[cfg(not(test))]
    #[global_allocator]
    static ALLOCATOR: LockedHeap = LockedHeap::new();

    /// 终止当前用户进程。
    pub fn exit(code: i32) -> ! {
        let _ = Syscalls::exit(code);
        loop {
            core::hint::spin_loop();
        }
    }

    /// 把格式化结果按 fd 写入的 [`Write`] 实现，按需拆分部分写入。
    struct FdWriter(usize);

    impl Write for FdWriter {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            let mut rest = s.as_bytes();
            while !rest.is_empty() {
                match Syscalls::write(self.0, rest) {
                    Ok(0) => return Err(fmt::Error),
                    Ok(n) => rest = &rest[n..],
                    Err(_) => return Err(fmt::Error),
                }
            }
            Ok(())
        }
    }

    /// `print!` / `println!` 宏的内部入口：把格式化结果写入 fd 1。
    #[doc(hidden)]
    pub fn _print(args: fmt::Arguments<'_>) {
        let _ = FdWriter(STDOUT).write_fmt(args);
    }

    /// `eprint!` / `eprintln!` 宏的内部入口：把格式化结果写入 fd 2。
    #[doc(hidden)]
    pub fn _eprint(args: fmt::Arguments<'_>) {
        let _ = FdWriter(STDERR).write_fmt(args);
    }

    unsafe extern "C" {
        fn main();
    }

    /// 纯 Rust 用户程序入口：直接记录栈映像指针并调用 `main`，不做任何 C ABI 栈转换。
    #[cfg(not(test))]
    #[unsafe(naked)]
    #[unsafe(no_mangle)]
    pub extern "C" fn _start() -> ! {
        naked_asm!(
            "ld a0, 0(sp)",
            "j {entry}",
            entry = sym __goethite_rs_entry,
        )
    }

    #[cfg(not(test))]
    extern "C" fn __goethite_rs_entry(image: *const u8) -> ! {
        unsafe { set_stack_image(image) };
        unsafe {
            asm!(
                "call {main}",
                main = sym main,
                clobber_abi("C"),
            );
        }
        exit(0)
    }

    #[cfg(not(test))]
    #[panic_handler]
    fn panic(info: &PanicInfo) -> ! {
        _eprint(format_args!("userland panic: {}\n", info.message()));
        exit(1)
    }
}

#[cfg(target_arch = "riscv64")]
pub use runtime::{_eprint, _print, exit};

/// 向标准输出写入格式化内容，不追加换行。
#[cfg(target_arch = "riscv64")]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::_print(format_args!($($arg)*));
    }};
}

/// 向标准输出写入一行（末尾追加换行符）。
#[cfg(target_arch = "riscv64")]
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n");
    }};
}

/// 向标准错误写入格式化内容，不追加换行。
#[cfg(target_arch = "riscv64")]
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {{
        $crate::_eprint(format_args!($($arg)*));
    }};
}

/// 向标准错误写入一行（末尾追加换行符）。
#[cfg(target_arch = "riscv64")]
#[macro_export]
macro_rules! eprintln {
    () => { $crate::eprint!("\n") };
    ($($arg:tt)*) => {{
        $crate::eprint!($($arg)*);
        $crate::eprint!("\n");
    }};
}
