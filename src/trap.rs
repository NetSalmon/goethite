pub mod entry;

use goethite_riscv64::{
    registers::{Registers, csr::Sepc},
    trap::{Exception, Trap},
};
use goethite_stdio::println;

use crate::proc::{CURRENT_TASK, schedule::SCHEDULER};

pub enum Todo {
    Return { value: u64, skip: u64 },
    NoReturn { skip: u64 },
    Yield,
    Discard,
}

#[unsafe(no_mangle)]
pub fn trap_handler(sepc: u64, scause: i64, stval: u64, stvec: u64, sstatus: u64, sp: u64) {
    let is_user_trap = stvec == entry::__user_trap_entry as *const () as u64;

    let context = unsafe { &mut *(sp as *mut Registers).offset(-1) };

    let todo = match Trap::from(scause) {
        Trap::Exception(ept) => match ept {
            Exception::BREAKPOINT => {
                println!("breakpoint at {:#p}", sepc as *const ());
                Todo::NoReturn { skip: 1 }
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
            _ => Todo::Discard,
        },
        Trap::Interrupt(int) => match int {
            _ => Todo::Discard,
        },
    };

    Sepc::write(match todo {
        Todo::Return { value, skip } => {
            context.a0 = value;
            sepc + skip * 4
        }
        Todo::NoReturn { skip } => sepc + skip * 4,
        Todo::Yield => sepc,
        Todo::Discard => sepc,
    });

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
