#![no_std]

use core::arch::asm;

use goethite_abi::{Fd, ForkFlags, PosixErrno, SysResult, Syscall, SyscallOps};

pub fn syscall(args: [u64; 8]) -> SysResult<u64> {
    let mut ret = args[0];
    unsafe {
        asm!(
            "ecall",
            inout("a0") ret,
            in("a1") args[1],
            in("a2") args[2],
            in("a3") args[3],
            in("a4") args[4],
            in("a5") args[5],
            in("a6") args[6],
            in("a7") args[7],
        );
    }
    let result = ret as i64;

    if result < 0 {
        Err(PosixErrno::from(result.abs()))
    } else {
        Ok(result as u64)
    }
}

pub struct Syscalls;

impl SyscallOps for Syscalls {
    fn open(path: &str, flags: u32, mode: u16) -> SysResult<Fd> {
        let result = syscall([
            path.as_ptr() as u64,
            path.len() as u64,
            flags as u64,
            mode as u64,
            0,
            0,
            0,
            Syscall::OPEN.into(),
        ]);

        result.map(|fd| fd as usize)
    }

    fn close(fd: usize) -> SysResult<()> {
        syscall([fd as u64, 0, 0, 0, 0, 0, 0, Syscall::CLOSE.into()]).map(|_| ())
    }

    fn read(fd: usize, buf: &mut [u8]) -> SysResult<usize> {
        let result = syscall([
            fd as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            0,
            0,
            Syscall::READ.into(),
        ]);

        result.map(|len| len as usize)
    }

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

    fn fork(flags: ForkFlags, user_sp: usize, entry: usize, arg: usize) -> SysResult<usize> {
        let result = syscall([
            u64::from(flags),
            user_sp as u64,
            entry as u64,
            arg as u64,
            0,
            0,
            0,
            Syscall::FORK.into(),
        ]);

        result.map(|tid| tid as usize)
    }

    fn execve(path: &str, args: &[&str], env: &[(&str, &str)]) -> SysResult<i32> {
        let result = syscall([
            path.as_ptr() as u64,
            path.len() as u64,
            args.as_ptr() as u64,
            args.len() as u64,
            env.as_ptr() as u64,
            env.len() as u64,
            0,
            Syscall::EXECVE.into(),
        ]);

        result.map(|v| v as i32)
    }

    fn spawn_process() -> SysResult<usize> {
        let result = syscall([0, 0, 0, 0, 0, 0, 0, Syscall::SPAWN_PROCESS.into()]);
        result.map(|tid| tid as usize)
    }

    fn spawn_thread(entry: usize, user_sp: usize, arg: usize) -> SysResult<usize> {
        let result = syscall([
            entry as u64,
            user_sp as u64,
            arg as u64,
            0,
            0,
            0,
            0,
            Syscall::SPAWN_THREAD.into(),
        ]);

        result.map(|tid| tid as usize)
    }

    fn read_dir(fd: usize, buf: &mut [u8]) -> SysResult<usize> {
        let result = syscall([
            fd as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            0,
            0,
            Syscall::READ_DIR.into(),
        ]);

        result.map(|len| len as usize)
    }

    fn exit(code: i32) -> SysResult<()> {
        syscall([code as u64, 0, 0, 0, 0, 0, 0, Syscall::EXIT.into()]).map(|_| ())
    }

    fn wait(tid: usize) -> SysResult<usize> {
        let result = syscall([tid as u64, 0, 0, 0, 0, 0, 0, Syscall::WAIT.into()]);
        result.map(|v| v as usize)
    }

    fn yield_now() -> SysResult<()> {
        syscall([0, 0, 0, 0, 0, 0, 0, Syscall::YIELD.into()]).map(|_| ())
    }
}
