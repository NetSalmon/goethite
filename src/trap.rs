pub mod entry;

use goethite_riscv64::{
    registers::{Context, csr::Sepc},
    trap::{Exception, Trap},
};
use goethite_stdio::println;

pub enum Todo {
    Return { value: u64, skip: u64 },
    NoReturn { skip: u64 },
    Yield,
    Discard,
}

#[unsafe(no_mangle)]
pub fn trap_handler(sepc: u64, scause: i64, stval: u64, stvec: u64, sstatus: u64, sp: u64) {
    let is_user_trap = stvec == entry::__user_trap_entry as *const () as u64;

    let context = unsafe { &mut *(sp as *mut Context).offset(-1) };

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

    match todo {
        Todo::Return { value, skip } => {
            context.a0 = value;
            Sepc::modify(|i| i + 4 * skip);
        }
        Todo::NoReturn { skip } => {
            Sepc::modify(|i| i + 4 * skip);
        }
        Todo::Yield => {
            todo!()
        }
        Todo::Discard => (),
    }
}
