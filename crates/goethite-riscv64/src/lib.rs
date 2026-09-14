//! RISC-V 64 内核目标 crate（`#![no_std]`）。
//!
//! 提供最底层的 CPU / 运行时原语与硬件抽象，供内核与 boot 代码使用：
//!
//! - [`addr`]：Sv39 虚拟地址与物理地址类型及相互转换；
//! - [`sbi`]：SBI 固件调用封装（`ecall` / legacy 与各扩展）；
//! - [`registers`]：CSR 与通用寄存器抽象；
//! - [`trap`]：陷阱（异常 / 中断）类型与 `scause` 解析；
//! - 自由函数：汇编辅助原语，如 [`wait_for_interrupt`]、[`hart_id`]、
//!   [`disable_interrupt`] / [`enable_interrupt`]、[`instruction_fence`]。
//!
//! 本 crate 不依赖标准库，只做类型与内联汇编包装，不引入任何运行时状态。

#![no_std]
use core::arch::asm;

pub mod addr;
pub mod registers;
pub mod sbi;
pub mod trap;

/// 停止当前 hart，直到有中断到达。
///
/// `wfi` 只是低功耗等待提示；实现可以提前返回，因此调用者不能把它当作
/// 永久阻塞或同步原语使用。
#[inline]
pub fn wait_for_interrupt() {
    unsafe { asm!("wfi", options(nomem, nostack, preserves_flags)) }
}

/// 使指令缓存和取指流水线观察到此前写入的代码。
#[inline]
pub fn instruction_fence() {
    unsafe { asm!("fence.i", options(nostack, preserves_flags)) }
}

/// 刷新当前 hart 的全部地址转换缓存。
///
/// 该操作只影响当前 hart；多 hart 场景需要额外使用 SBI RFENCE 或 IPI。
#[inline]
pub fn address_translation_fence() {
    unsafe { asm!("sfence.vma", options(nostack, preserves_flags)) }
}

/// 返回启动入口保存到 `tp` 的当前 hart 编号。
///
/// OpenSBI 会在进入 S 模式时把 hart 编号作为启动参数传入，启动汇编将其
/// 保存到 `tp`。相比读取 M-mode 的 `mhartid`，这种方式适用于 S 模式内核。
#[inline]
pub fn hart_id() -> usize {
    registers::gpr::Tp::read() as usize
}

/// 触发一个软件断点。
#[inline]
pub fn breakpoint() {
    unsafe {
        asm!("ebreak");
    }
}

/// 关中断并返回此前是否处于开中断状态。
///
/// 使用 RISC-V `csrrc` 原子指令清除 `sstatus.SIE` 位（bit 1）并返回旧状态。
#[inline]
pub fn disable_interrupt() -> bool {
    let sstatus: u64;
    unsafe {
        asm!(
        "csrrc {}, sstatus, {}",
        out(reg) sstatus,
        in(reg) 1 << 1,
        options(nomem, nostack, preserves_flags)
        );
    }
    (sstatus & (1 << 1)) != 0
}

/// 恢复中断使能状态。
///
/// 若此前处于开中断状态（`prev_sie == true`），则重新置位 `sstatus.SIE`。
#[inline]
pub fn enable_interrupt(prev_sie: bool) {
    if prev_sie {
        unsafe {
            asm!(
            "csrrs zero, sstatus, {}",
            in(reg) 1 << 1,
            options(nomem, nostack, preserves_flags)
            );
        }
    }
}

pub struct InterruptGuard(bool);

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        enable_interrupt(self.0);
    }
}

impl Default for InterruptGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl InterruptGuard {
    pub fn new() -> Self {
        InterruptGuard(disable_interrupt())
    }
}
