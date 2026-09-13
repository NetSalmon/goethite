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

extern crate alloc;

use core::{arch::naked_asm};

use goethite_macros::{critical_section};
use goethite_riscv64::{
    addr::{PhysicalAddr, VirtualAddr},
};

const KERNEL_STACK_SIZE: usize = 0x1000 * 16;

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
        init::init_buddy_system().unwrap();
        init::init_page_table().unwrap();
    }

    loop {
        core::hint::spin_loop();
    }
}
