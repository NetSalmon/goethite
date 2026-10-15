use core::ptr::slice_from_raw_parts;

use goethite_abi::{Fd, SysResult, Syscall, SyscallTrait};
use goethite_riscv64::sbi::legacy::console_putchar;

use crate::proc::Context;

pub struct SyscallHandler;

impl SyscallTrait for SyscallHandler {
    fn open() -> SysResult<Fd> {
        todo!()
    }

    fn close(fd: usize) -> SysResult<()> {
        todo!()
    }

    fn read(fd: usize, buf: &mut [u8]) -> SysResult<usize> {
        todo!()
    }

    fn write(fd: usize, buf: &[u8]) -> SysResult<usize> {
        for ch in buf {
            console_putchar(*ch)
        }
        Ok(buf.len())
    }

    fn stat(fd: usize) -> SysResult<i32> {
        todo!()
    }

    fn file_stat(path: &str) -> SysResult<i32> {
        todo!()
    }

    fn fork() -> SysResult<usize> {
        todo!()
    }

    fn execve(path: &str, args: &[&str], env: &[&str]) -> SysResult<i32> {
        todo!()
    }
}

pub fn syscall_handler(context: &Context) {
    let regs = context.registers.clone();

    match Syscall::from(regs.a7) {
        Syscall::WRITE => {
            let buf = unsafe { &*slice_from_raw_parts(regs.a1 as *const u8, regs.a2 as usize) };
            SyscallHandler::write(regs.a0 as usize, buf).unwrap();
        }
        _ => (),
    }
}
