//! 内核栈切换。
//!
//! [`switch_stack`] 保存返回所需的被调用者寄存器，把当时的 `sp` 写入调用方
//! 给出的槽位（[`crate::mm::stack::Stack::sp`]），再从目标 `sp` 恢复。
//! 切回时，原先的 [`Stack::switch_to`](crate::mm::stack::Stack::switch_to)
//! 从调用点返回。
//!
//! 帧布局（低地址在前，共 [`SWITCH_FRAME_SIZE`] 字节）：
//!
//! - `0`：`ra`
//! - `8`：`s0`
//! - `16..104`：`s1`–`s11`
//! - `104`：填充，使帧长保持 16 字节对齐

use core::arch::{asm, naked_asm};

/// 切换帧大小：`ra` + `s0`–`s11`，再补一个字对齐到 16 字节。
pub(crate) const SWITCH_FRAME_SIZE: usize = 14 * 8;

const _: () = assert!(SWITCH_FRAME_SIZE % 16 == 0);
const _: () = assert!(size_of::<usize>() == 8);

/// 读当前栈指针。
pub(crate) fn current_sp() -> usize {
    let sp: usize;
    unsafe {
        asm!(
            "mv {}, sp",
            out(reg) sp,
            options(nomem, nostack, preserves_flags),
        );
    }
    sp
}

/// 在 `stack_top` 之下布置初始帧。
///
/// `ra` 指向 [`enter`]，`s0` 保存真正的入口。切换回来后 `ret` 只能填一个
/// 地址，所以入口先落到桩上，再由 `s0` 跳进 `entry`。
///
/// # Safety
///
/// `stack_top` 必须 16 字节对齐，且其下至少 [`SWITCH_FRAME_SIZE`] 字节可写。
pub(crate) unsafe fn write_entry_frame(stack_top: *mut u8, entry: usize) -> *mut u8 {
    debug_assert!(!stack_top.is_null() && stack_top as usize % 16 == 0);
    unsafe {
        let frame = stack_top.sub(SWITCH_FRAME_SIZE);
        core::ptr::write_bytes(frame, 0, SWITCH_FRAME_SIZE);
        let regs = frame.cast::<usize>();
        regs.write(enter as *const () as usize);
        regs.add(1).write(entry);
        frame
    }
}

/// `a0` = 保存当前 `sp` 的槽位，`a1` = 目标栈指针。
///
/// # Safety
///
/// - `saved_sp` 在本次切走期间必须一直有效。
/// - `next_sp` 必须 16 字节对齐，并指向本函数以前保存的帧，或
///   [`write_entry_frame`] 写好的初始帧。
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn switch_stack(saved_sp: *mut usize, next_sp: usize) {
    naked_asm!(
        "addi sp, sp, -{frame}",
        "sd ra,  0(sp)",
        "sd s0,  8(sp)",
        "sd s1,  16(sp)",
        "sd s2,  24(sp)",
        "sd s3,  32(sp)",
        "sd s4,  40(sp)",
        "sd s5,  48(sp)",
        "sd s6,  56(sp)",
        "sd s7,  64(sp)",
        "sd s8,  72(sp)",
        "sd s9,  80(sp)",
        "sd s10, 88(sp)",
        "sd s11, 96(sp)",
        // 帧已经压好，把这个 sp 记入 Stack::sp（或调用方给出的槽位）。
        "sd sp, 0(a0)",
        "mv sp, a1",
        "ld ra,  0(sp)",
        "ld s0,  8(sp)",
        "ld s1,  16(sp)",
        "ld s2,  24(sp)",
        "ld s3,  32(sp)",
        "ld s4,  40(sp)",
        "ld s5,  48(sp)",
        "ld s6,  56(sp)",
        "ld s7,  64(sp)",
        "ld s8,  72(sp)",
        "ld s9,  80(sp)",
        "ld s10, 88(sp)",
        "ld s11, 96(sp)",
        "addi sp, sp, {frame}",
        "ret",
        frame = const SWITCH_FRAME_SIZE,
    );
}

/// 初始帧的落点：`s0` 是 [`write_entry_frame`] 写入的入口。
///
/// 入口若返回，则进入 [`entry_returned`]。
#[unsafe(naked)]
unsafe extern "C" fn enter() {
    naked_asm!(
        "jalr ra, s0, 0",
        "tail {halt}",
        halt = sym entry_returned,
    );
}

extern "C" fn entry_returned() -> ! {
    panic!("kernel stack entry returned");
}
