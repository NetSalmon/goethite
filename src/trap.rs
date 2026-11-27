pub mod entry;

use goethite_riscv64::{
    registers::{Registers, csr::Sepc},
    trap::{Exception, Interrupt, Trap},
};
use goethite_riscv64_asm::Encoded;
use goethite_console::println;

use crate::{
    device::plic,
    proc::{CURRENT_TASK, schedule::SCHEDULER},
};

pub enum TrapAction {
    /// 写入 `a0`，并向后跳过 `skip` 条完整指令后继续执行。
    Return { value: u64, skip: u64 },
    /// 不修改返回值，并向后跳过 `skip` 条完整指令后继续执行。
    NoReturn { skip: u64 },
    /// 保持 `sepc` 不变，让调度器决定后续执行流。
    Yield,
    /// 保持 `sepc` 不变，直接返回。
    Discard,
}

/// 按实际指令长度推进 `skip` 条指令。
///
/// 每条指令的低两位决定它是 16 位压缩指令还是 32 位标准指令，
/// 因此不能假设固定 4 字节步长。
///
/// # Safety
///
/// 从 `pc` 开始的 `skip` 条指令都必须位于可读内存中。
#[inline]
unsafe fn skip_instructions(mut pc: u64, skip: u64) -> u64 {
    for _ in 0..skip {
        pc += unsafe { Encoded::read(pc as *const u8) }.size() as u64;
    }
    pc
}

#[unsafe(no_mangle)]
pub fn trap_handler(sepc: u64, scause: i64, stval: u64, stvec: u64, sstatus: u64, sp: u64) {
    let is_user_trap = stvec == entry::__user_trap_entry as *const () as u64;

    let context = unsafe { &mut *(sp as *mut Registers).offset(-1) };

    let action = match Trap::from(scause) {
        Trap::Exception(ept) => match ept {
            Exception::BREAKPOINT => {
                println!("breakpoint at {:#p}", sepc as *const ());
                TrapAction::NoReturn { skip: 1 }
            }
            Exception::U_MODE_ECALL => {
                todo!()
            }
            Exception::ILLEGAL_INSTRUCTION => {
                panic!("illegal instruction: {:#p}", sepc as *const ());
            }
            Exception::STORE_PAGE_FAULT | Exception::LOAD_PAGE_FAULT => {
                panic!("store page fault: {:#p}", sepc as *const ());
            }
            _ => TrapAction::Discard,
        },
        Trap::Interrupt(int) => {
            // S 模式外部中断交由 PLIC 认领并分派给已登记的设备处理函数。
            // 中断的 sepc 已指向被打断的指令，保持 `TrapAction::Discard` 即可原地返回。
            if int == Interrupt::SUPERVISOR_EXTERNAL {
                plic::handle_external_interrupt();
            }
            TrapAction::Discard
        }
    };

    let sepc = match action {
        TrapAction::Return { value, skip } => {
            context.a0 = value;
            unsafe { skip_instructions(sepc, skip) }
        }
        TrapAction::NoReturn { skip } => unsafe { skip_instructions(sepc, skip) },
        TrapAction::Yield => sepc,
        TrapAction::Discard => sepc,
    };

    Sepc::write(sepc);

    if is_user_trap {
        let current = CURRENT_TASK.current();

        if let Some(tid) = *current {
            let clone_context = context.clone();

            if let Some(current_task) = SCHEDULER.lock().tasks.get_mut(&tid) {
                current_task.context.registers = clone_context;
                current_task.context.sepc = Sepc::read();
            }
        }
    }
}
