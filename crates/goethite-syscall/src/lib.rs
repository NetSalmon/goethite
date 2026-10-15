#![no_std]

use core::arch::asm;

use goethite_abi::{PosixErrno, SysResult, Syscall, SyscallTrait};
use goethite_riscv64::registers::gpr::A0;

pub fn syscall(args: [u64; 8]) -> SysResult<u64> {
    unsafe {
        asm!(
            "mv a0, {}",
            "mv a1, {}",
            "mv a2, {}",
            "mv a3, {}",
            "mv a4, {}",
            "mv a5, {}",
            "mv a6, {}",
            "mv a7, {}",
            "ecall",
            in (reg) args[0],
            in (reg) args[1],
            in (reg) args[2],
            in (reg) args[3],
            in (reg) args[4],
            in (reg) args[5],
            in (reg) args[6],
            in (reg) args[7],
        )
    }
    let result = A0::read() as i64;

    if result < 0 {
        Err(PosixErrno::from(result.abs()))
    } else {
        Ok(result as u64)
    }
}

pub struct Syscalls;

impl SyscallTrait for Syscalls {
    fn write(fd: usize, buf: &[u8]) -> SysResult<usize> {
        let result = syscall([
            fd as u64,
            buf.as_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            0,
            0,
            Syscall::WRITE.into(),
        ]);

        result.map(|len| len as usize)
    }
}
