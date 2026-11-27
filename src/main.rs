#![no_std]
#![no_main]

//! goethite —— 面向 RISC-V 64（`riscv64gc`）的宏内核，`#![no_std]` / `#![no_main]`。
//!
//! # 启动流程
//!
//! 1. 链接器入口 [`__kernel_entry`]（`src/main.rs`）设置内核栈后调用 [`main`]；
//!    在此之前，`boot` 模块已建立 Sv39 页表、恒等映射并跳转过来。
//! 2. [`main`] 从设备树定位物理内存，初始化伙伴系统（[`FRAME_ALLOCATOR`]）。
//! 3. 内核基础设施由本 crate 的模块与 `goethite-*` 外围 crate 提供。
//!
//! # 模块
//!
//! - [`arch`]：与硬件/架构相关的辅助（当前为占位）。
//! - `boot`：启动阶段的页表构建与寄存器初始化（内核内部使用）。
//! - [`mm`]：内存管理子系统（页表、伙伴系统、SLUB、地址空间）。

pub mod arch;
mod boot;
pub mod device;
pub mod error;
pub mod fs;
pub mod init;
pub mod mm;
pub mod proc;
pub mod switch;
pub mod syscall;
pub mod trap;
pub mod ipc;

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::vec;
use core::arch::naked_asm;

use goethite_macros::{critical_section, load_symbol};
use goethite_riscv64::{
    addr::{PhysicalAddr, VirtualAddr},
    breakpoint,
    registers::csr::Stvec,
};
use trap::entry::__kernel_trap_entry;

use crate::{
    boot::BOOT_STACK_SIZE,
    mm::alloc::FRAME_ALLOCATOR,
    proc::{exec::try_execve, schedule::SCHEDULER_STACK},
    switch::switch_stack,
};

/// POSIX / Linux 惯例的初始用户进程（PID 1）搜索路径。
const INIT_PATHS: &[&str] = &["/sbin/init", "/etc/init", "/bin/init", "/init"];

/// Kernel Stack Size limit 16KiB
pub const KERNEL_STACK_SIZE: usize = 0x1000 * 16;

pub static mut FDT_ADDR: VirtualAddr = VirtualAddr::new();

#[unsafe(link_section = ".bss.__stack")]
#[unsafe(export_name = "__kernel_stack")]
pub static KERNEL_STACK: [u8; KERNEL_STACK_SIZE] = [0; KERNEL_STACK_SIZE];

/// 高地址内核唯一入口
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn __kernel_entry() {
    naked_asm!(
        "la sp, __kernel_stack + {}",
        "j main",
        const KERNEL_STACK_SIZE,
    )
}

#[unsafe(no_mangle)]
pub fn main(ptr: *mut u32) {
    critical_section! {
        let fdt_ptr = PhysicalAddr::from(ptr as usize).as_vaddr();
        unsafe { FDT_ADDR = fdt_ptr; }

        let boot_stack = PhysicalAddr::from(load_symbol!(__boot_stack)).as_vaddr().into();

        // 销毁boot栈
        FRAME_ALLOCATOR
            .force()
            .lock()
            .add_frame(boot_stack, boot_stack + BOOT_STACK_SIZE);

        let memory = init::init_buddy_system().unwrap();
        init::init_page_table(&memory).unwrap();
        init::init_devices();
        init::init_interrupts();
        init::init_filesystem();
    }

    Stvec::write(__kernel_trap_entry as *const () as u64);

    breakpoint();

    let mut init_loaded = false;
    for path in INIT_PATHS {
        if try_execve((*path).into(), vec![(*path).to_string()], BTreeMap::new()).is_ok() {
            goethite_console::println!("starting init: {path}");
            init_loaded = true;
            break;
        }
    }
    assert!(
        init_loaded,
        "no init binary found in {:?}",
        INIT_PATHS
    );

    // 切入调度器独立内核栈并进入调度循环（KERNEL_STACK 不再使用）
    let mut boot_saved_sp = 0usize;
    unsafe {
        switch_stack(&mut boot_saved_sp, SCHEDULER_STACK.sp());
    }
}
