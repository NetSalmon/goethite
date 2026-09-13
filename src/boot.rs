//! 启动阶段：在 MMP 尚未开启的低地址环境下建立映射并进入内核。
//!
//! [`_boot`] 在 `.text.low` / `.bss.low.stack` 中运行，先切换到 `__boot_stack`，
//! 再进入 [`_jump`]。`_jump` 会：
//!
//! 1. 把 hart id 写入 `tp`（供后续 [`goethite_riscv64::hart_id`] 读取）；
//! 2. 在 `_end` 之后清零一块 4 KiB 作为根页表；
//! 3. 使用 1 GB 大页做两块映射——低地址恒等映射
//!    （`0x8020_0000..0xc000_0000`）与高地址内核映射
//!    （`0xffff_ffc0_8000_0000..` 对应同一块物理内存）；
//! 4. 写入 `satp` 开启 Sv39 分页并 `sfence.vma`；
//! 5. 设置 `sstatus.SPP` 为 Supervisor、`sepc` 为 [`crate::__kernel_entry`]，
//!    用 `sret` 跳转到内核入口。
//!
//! 注意：此阶段 MMU 未开启，地址都是物理地址，因此通过裸指针写页表，
//! 避免构造引用而引入 `core::panicking` 的远地址检查。
use core::arch::{asm, naked_asm};

use goethite_config::{_end, PAGE_SIZE, align};
use goethite_riscv64::{
    addr::{PhysicalAddr, VirtualAddr},
    registers::{
        csr::{Satp, Sepc, Sstatus},
        gpr::A0,
        values::{SatpMode, SatpValue},
    },
};

use crate::mm::page_table::PageTableEntry;

/// 启动栈大小（4 KiB），位于 `.bss.low.stack`。
const BOOT_STACK_SIZE: usize = 4096;

/// 启动阶段使用的栈，位于低地址 `.bss`，在 `ummu` 开启前用物理地址寻址。
#[unsafe(link_section = ".bss.low.stack")]
#[unsafe(export_name = "__boot_stack")]
static BOOT_STACK: [u8; BOOT_STACK_SIZE] = [0u8; BOOT_STACK_SIZE];

/// 最底层的启动入口：切换到 [`BOOT_STACK`] 后跳到 [`_jump`]。
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.low")]
extern "C" fn _boot() {
    naked_asm! {
        "la sp, __boot_stack + {}",
        "j _jump",
        const BOOT_STACK_SIZE,
    }
}

/// 建立启动页表、开启 Sv39 分页并跳转到内核入口。
///
/// `hart_id` 是当前 hart 编号，`dtb_p_addr` 是设备树（DTB）的物理地址。
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.low")]
fn _jump(hart_id: u64, dtb_p_addr: u64) {
    unsafe { asm!( "mv tp, {}", in(reg) hart_id ) };

    let addr = align(_end as *const () as usize, PAGE_SIZE);
    // MMU 尚未开启，`_end` 是物理地址；通过裸指针写入页表，
    // 避免构造引用而拉入 core::panicking 的远地址检查函数。
    let page_table_paddr = PhysicalAddr::from(addr);
    let page_table = page_table_paddr.as_mut_ptr::<PageTableEntry>();

    // 清零根页表（使用 write_volatile 防止编译器优化为调用高地址的 memset）
    for i in 0..512 {
        unsafe { core::ptr::write_volatile(page_table.add(i), PageTableEntry::from(0)) };
    }

    // 物理内存 1GB 块的基物理页号（0x8000_0000 >> 12 = 0x80000）
    // 1GB 大页要求 ppn[1:0] 必须为 0，0x80000 的低 18 位全为 0，符合 Sv39 大页规范。
    let phys_ppn = PhysicalAddr::from(0x8000_0000usize).ppn() as u64;
    let pte = PageTableEntry::builder()
        .set_ppn(phys_ppn)
        .set_dirty(true)
        .set_accessed(true)
        .set_executable(true)
        .set_writable(true)
        .set_readable(true)
        .set_validate(true)
        .build();

    // 1. 低地址恒等映射 (0x80000000..0xc0000000 -> 0x80000000..0xc0000000, vpn2 = 2)
    let va_low = VirtualAddr::from(0x8020_0000);
    unsafe { page_table.add(va_low.vpn2()).write(pte) };

    // 2. 高地址内核映射 (0xffffffc080000000..0xffffffc0c0000000 -> 0x80000000..0xc0000000, vpn2 = 258)
    let va_high = PhysicalAddr::from(0x8020_0000).as_vaddr();
    unsafe { page_table.add(va_high.vpn2()).write(pte) };

    let value = SatpValue::builder()
        .set_ppn(page_table_paddr.ppn() as u64)
        .set_mode(u64::from(SatpMode::SV39))
        .build();

    Satp::write(value.into());

    unsafe { asm!("sfence.vma") };

    // 设置 sstatus.SPP 为 Supervisor 模式 (bit 8 = 1)
    Sstatus::write(Sstatus::read() | (1 << 8));

    // 使用 load_symbol! 安全加载 64 位符号地址，支持标识符与字符串字面量
    goethite_macros::load_symbol!(let main_va: u64, __kernel_entry);

    Sepc::write(main_va);
    A0::write(dtb_p_addr);

    unsafe { asm!("sret") };
}
