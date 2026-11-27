//! goethite 用户态运行时：`print` 系列宏与进程初始栈（argv / env / auxv）解析。
//!
//! 本 crate 提供用户程序入口 [`_start`]（crt0）。内核在用户栈顶放置一份
//! [`goethite_stack`] 栈映像，并在 `sp` 处存放一个指向映像起始地址的指针
//! （布局见 `goethite-stack::raw`）。[`_start`] 读出该指针、解析映像，把 `argv`
//! 还原为标准 C 风格指针数组（字符串拷贝到用户栈并 NUL 结尾），随后按寄存器约定
//! `a0 = argc`、`a1 = argv` 跳转到用户定义的 `main`。
//!
//! `main` 的符号名固定为 `main`，但签名不受限：参数经由 `a0` / `a1` 传递，
//! 用户可自行声明所需形参。
//!
//! 输出宏 [`print!`](macro@print) / [`println!`](macro@println) 写 fd 1，
//! [`eprint!`](macro@eprint) / [`eprintln!`](macro@eprintln) 写 fd 2，均通过
//! [`goethite_syscall::Syscalls`] 的 `write` 系统调用完成，支持 [`fmt`] 格式化。

#![no_std]

use core::{
    arch::{asm, naked_asm},
    fmt::{self, Write},
    mem::size_of,
    panic::PanicInfo,
    sync::atomic::{AtomicPtr, Ordering},
};

use goethite_abi::SyscallOps;
use goethite_stack::{ArgsIter, AuxvIter, EnvIter};
pub use goethite_stack::{
    AT_BASE, AT_BASE_PLATFORM, AT_CLKTCK, AT_EGID, AT_ENTRY, AT_EUID, AT_EXECFD, AT_EXECFN,
    AT_FLAGS, AT_GID, AT_HWCAP, AT_HWCAP2, AT_IGNORE, AT_NOTELF, AT_NULL, AT_PAGESZ, AT_PHDR,
    AT_PHENT, AT_PHNUM, AT_PLATFORM, AT_RANDOM, AT_SECURE, AT_SYSINFO, AT_SYSINFO_EHDR, AT_UID,
    AuxEntry, Error as StackError, StackRef,
};
use goethite_syscall::Syscalls;

/// 标准输出文件描述符。
pub const STDOUT: usize = 1;
/// 标准错误文件描述符。
pub const STDERR: usize = 2;

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
///
/// 由 `$crate::_print` 从外部 crate 引用，故标记为 `#[doc(hidden)] pub`。
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    let _ = FdWriter(STDOUT).write_fmt(args);
}

/// `eprint!` / `eprintln!` 宏的内部入口：把格式化结果写入 fd 2。
#[doc(hidden)]
pub fn _eprint(args: fmt::Arguments<'_>) {
    let _ = FdWriter(STDERR).write_fmt(args);
}

/// 向标准输出写入格式化内容，不追加换行。
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::_print(format_args!($($arg)*));
    }};
}

/// 向标准输出写入一行（末尾追加换行符）。
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n");
    }};
}

/// 向标准错误写入格式化内容，不追加换行。
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {{
        $crate::_eprint(format_args!($($arg)*));
    }};
}

/// 向标准错误写入一行（末尾追加换行符）。
#[macro_export]
macro_rules! eprintln {
    () => { $crate::eprint!("\n") };
    ($($arg:tt)*) => {{
        $crate::eprint!($($arg)*);
        $crate::eprint!("\n");
    }};
}

/// 初始栈映像起始地址；由 [`_start`] 在进入 `main` 前写入，未初始化时为 null。
static STACK_IMAGE: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

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

/// 迭代环境变量（`env`），产出 `(&str, &str)`。
pub fn envs() -> Option<EnvIter<'static>> {
    stack().map(|stack| stack.env())
}

/// 迭代辅助向量（`auxv`），产出 `(u64, u64)`。
pub fn auxv() -> Option<AuxvIter<'static>> {
    stack().map(|stack| stack.auxv())
}

/// 用户程序入口（crt0），由链接器作为 ELF 入口调用，最终跳转到用户定义的 `main`。
///
/// # 前置约定
///
/// `sp` 指向一个 `u64`，其值为初始栈映像起始地址（见 `goethite-stack::raw` 布局）。
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    naked_asm!(
        // a0 = 初始栈映像指针（*sp），a1 = 用户栈顶（供构造 C 风格 argv）
        "ld a0, 0(sp)",
        "mv a1, sp",
        "j {entry}",
        entry = sym __goethite_entry,
    )
}

// 用户定义的 `main`。符号名固定，签名不受限：`a0 = argc`、`a1 = argv`。
unsafe extern "C" {
    fn main();
}

/// [`_start`] 的裸入口：先在原栈上计算 argv 区域布局，切换到该区域顶部作为新
/// `sp`，再在**新栈**上构造 argv 并进入 `main`。
///
/// 分两阶段是为了避免构造 argv 时与自身栈帧（可能远超 argv 区域大小）相互覆盖：
/// [`layout`] 只读、返回新 `sp` 与 `argc`；随后 `sp` 被下调到 argv 区域起点，
/// [`build`] 的栈帧便位于区域之下，写入互不干扰。
///
/// # 前置约定
///
/// 入参沿用 `_start` 传入的 `a0 = image`、`a1 = stack_top`。
#[unsafe(naked)]
unsafe extern "C" fn __goethite_entry(_image: *const u8, _stack_top: usize) -> ! {
    naked_asm!(
        "mv s0, a0",       // s0 = image
        "mv s1, a1",       // s1 = stack_top
        "call {layout}",   // (image, stack_top) -> a0 = new_sp, a1 = argc
        "mv a2, a1",       // a2 = argc
        "mv sp, a0",       // 切到 argv 区域起点
        "mv a0, s0",       // a0 = image
        "mv a1, s1",       // a1 = stack_top
        "call {build}",    // build(image, stack_top, argc) -> !
        "j .",
        layout = sym layout,
        build = sym build,
    )
}

/// [`layout`] 的结果：`new_sp` 为 argv 区域起点，`argc` 为参数个数。
///
/// 以 `#[repr(C)]` 两字结构体返回，按 C ABI 落在 `a0` / `a1`。
#[repr(C)]
struct Layout {
    new_sp: usize,
    argc: usize,
}

/// 计算 argv 区域布局。
///
/// `new_sp` 为在 `stack_top` 之下为指针数组与字符串腾出空间后的 16 字节对齐栈顶。
/// 本函数只读栈映像，其栈帧会在调用方切换 `sp` 后被丢弃。
extern "C" fn layout(image: *const u8, stack_top: usize) -> Layout {
    let stack = match StackRef::parse(unsafe { image_bytes(image) }) {
        Ok(stack) => stack,
        Err(_) => loop {
            core::hint::spin_loop()
        },
    };
    let argc = stack.argc();
    let strings_len: usize = stack.argv().map(|arg| arg.len() + 1).sum();
    let ptrs_len = (argc + 1) * size_of::<*const u8>();
    let new_sp = (stack_top - strings_len - ptrs_len) & !(16 - 1);
    Layout { new_sp, argc }
}

/// 在新栈上构造 C 风格 `argv`（字符串 NUL 结尾），随后按 `a0 = argc`、`a1 = argv`
/// 调用 `main`；`main` 返回后停在此处。
///
/// # Safety
///
/// 调用前 `sp` 必须已由 [`layout`] 下调到 argv 区域起点，使本函数栈帧位于区域之下。
unsafe extern "C" fn build(image: *const u8, stack_top: usize, argc: usize) -> ! {
    STACK_IMAGE.store(image as *mut u8, Ordering::Release);

    let stack = match StackRef::parse(unsafe { image_bytes(image) }) {
        Ok(stack) => stack,
        Err(_) => loop {
            core::hint::spin_loop()
        },
    };

    let strings_len: usize = stack.argv().map(|arg| arg.len() + 1).sum();
    let ptrs_len = (argc + 1) * size_of::<*const u8>();
    let region = (stack_top - strings_len - ptrs_len) & !(16 - 1);
    let argv = region as *mut *const u8;
    let mut cursor = (region + ptrs_len) as *mut u8;

    for (i, arg) in stack.argv().enumerate() {
        let bytes = arg.as_bytes();
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), cursor, bytes.len());
            cursor.add(bytes.len()).write(0);
            argv.add(i).write(cursor as *const u8);
        }
        cursor = unsafe { cursor.add(bytes.len() + 1) };
    }
    unsafe { argv.add(argc).write(core::ptr::null()) };

    unsafe {
        asm!(
            "call {main}",
            "j .",
            in("a0") argc,
            in("a1") argv,
            main = sym main,
            options(noreturn),
        )
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let _ = _eprint(format_args!("userland panic\n"));
    loop {
        core::hint::spin_loop()
    }
}
