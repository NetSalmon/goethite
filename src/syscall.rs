use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::{arch::asm, ptr::slice_from_raw_parts, ptr::slice_from_raw_parts_mut, str};

use goethite_abi::{Fd, ForkFlags, PosixErrno, SysResult, Syscall, SyscallOps};
use goethite_fs::{
    error::FsError,
    path::Path,
    types::Mode,
    vfs::{FileSystem, OpenFlags},
};
use goethite_riscv64::registers::Registers;
use goethite_sync::SpinLock;

use crate::{
    fs::{FileDesc, VFS},
    mm::stack::Stack,
    proc::{
        CURRENT_TASK, Task, TaskId, TaskStatus, alloc_tgid, alloc_tid, exec::load_task_image,
        free_tgid, free_tid, schedule::SCHEDULER,
    },
};

/// `sstatus.SUM`（bit 18）：允许 S-mode 临时访问 U=1 的用户态页。
const SSTATUS_SUM: usize = 1 << 18;

/// RAII 守卫：构造时置位 `sstatus.SUM`，析构时清除，用于内核安全读写用户态缓冲区。
pub struct UserAccessGuard;

impl UserAccessGuard {
    #[inline]
    pub fn enter() -> Self {
        unsafe {
            asm!("csrs sstatus, {}", in(reg) SSTATUS_SUM, options(nomem, nostack));
        }
        Self
    }
}

impl Drop for UserAccessGuard {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            asm!("csrc sstatus, {}", in(reg) SSTATUS_SUM, options(nomem, nostack));
        }
    }
}

fn fs_err_to_errno(err: FsError) -> PosixErrno {
    PosixErrno::from(err.errno() as i64)
}

fn current_tid() -> SysResult<TaskId> {
    (*CURRENT_TASK.current()).ok_or(PosixErrno::ESRCH)
}

fn get_fd(fd: usize) -> SysResult<FileDesc> {
    let tid = current_tid()?;
    let sched = SCHEDULER.lock();
    let task = sched.task(tid).ok_or(PosixErrno::ESRCH)?;
    task.fd(fd).ok_or(PosixErrno::EBADF)
}

fn encode_result<T>(result: SysResult<T>, ok_val: impl FnOnce(T) -> i64) -> u64 {
    match result {
        Ok(v) => ok_val(v) as u64,
        Err(errno) => (-i64::from(errno)) as u64,
    }
}

pub struct SyscallHandler;

impl SyscallOps for SyscallHandler {
    fn open(path: &str, flags: u32, mode: u16) -> SysResult<Fd> {
        let file =
            FileDesc::open(path, OpenFlags::from(flags), Mode::from(mode)).map_err(fs_err_to_errno)?;
        let tid = current_tid()?;
        let sched = SCHEDULER.lock();
        let task = sched.task(tid).ok_or(PosixErrno::ESRCH)?;
        Ok(task.alloc_fd(file))
    }

    fn close(fd: usize) -> SysResult<()> {
        let tid = current_tid()?;
        let mut sched = SCHEDULER.lock();
        let task = sched.task_mut(tid).ok_or(PosixErrno::ESRCH)?;
        task.close_fd(fd).ok_or(PosixErrno::EBADF)?;
        Ok(())
    }

    fn read(fd: usize, buf: &mut [u8]) -> SysResult<usize> {
        let file = get_fd(fd)?;
        file.read(buf).map_err(fs_err_to_errno)
    }

    fn write(fd: usize, buf: &[u8]) -> SysResult<usize> {
        let file = get_fd(fd)?;
        file.write(buf).map_err(fs_err_to_errno)
    }

    fn stat(fd: usize) -> SysResult<i32> {
        let file = get_fd(fd)?;
        file.stat().map_err(fs_err_to_errno)?;
        Ok(0)
    }

    fn file_stat(path: &str) -> SysResult<i32> {
        VFS.stat(&Path::new(path)).map_err(fs_err_to_errno)?;
        Ok(0)
    }

    fn fork(flags: ForkFlags, user_sp: usize, entry: usize, arg: usize) -> SysResult<usize> {
        // 同一线程组（share_tgid）必须共享同一虚拟地址空间（share_vm）。
        if flags.share_tgid() && !flags.share_vm() {
            return Err(PosixErrno::EINVAL);
        }

        let parent_tid = current_tid()?;
        let (parent_tgid, parent_vm, parent_fds, parent_stack, parent_priority, parent_context) = {
            let sched = SCHEDULER.lock();
            let parent = sched.task(parent_tid).ok_or(PosixErrno::ESRCH)?;
            (
                parent.task_group_id,
                Arc::clone(&parent.memory_set),
                Arc::clone(&parent.file_desc),
                parent.stack.clone(),
                parent.priority,
                parent.context.clone(),
            )
        };

        // 1. 地址空间与 tgid：共享或按 COW 克隆。
        let (child_vm, child_tgid, allocated_new_tgid) = if flags.share_vm() {
            let tgid = if flags.share_tgid() {
                parent_tgid
            } else {
                alloc_tgid().ok_or(PosixErrno::EAGAIN)?
            };
            (parent_vm, tgid, !flags.share_tgid())
        } else {
            let cow_ms = parent_vm
                .lock()
                .fork_cow()
                .map_err(|_| PosixErrno::ENOMEM)?;
            let tgid = cow_ms.tgid;
            (Arc::new(SpinLock::new(cow_ms)), tgid, false)
        };

        // 2. 分配子任务 tid。
        let Some(child_tid) = alloc_tid(child_tgid) else {
            if allocated_new_tgid {
                free_tgid(child_tgid);
            }
            return Err(PosixErrno::EAGAIN);
        };

        // 3. 文件描述符表：共享同一表或克隆独立副本。
        let child_fds = if flags.share_files() {
            parent_fds
        } else {
            let cloned = parent_fds.lock().clone();
            Arc::new(SpinLock::new(cloned))
        };

        // 4. 内核栈：共享或分配独立栈。
        let child_stack = if flags.share_stack() {
            parent_stack
        } else {
            Stack::try_new().map_err(|_| PosixErrno::ENOMEM)?
        };

        // 5. 构造子任务上下文。
        let mut child_context = parent_context;
        child_context.satp = child_vm.lock().satp();
        if user_sp != 0 {
            child_context.registers.sp = user_sp as u64;
        }
        if entry != 0 {
            child_context.sepc = entry as u64;
            child_context.registers.a0 = arg as u64;
        } else {
            child_context.registers.a0 = 0;
        }

        let child_task = Task {
            task_id: child_tid,
            task_group_id: child_tgid,
            parent: None,
            children: Vec::new(),
            memory_set: child_vm,
            status: TaskStatus::Ready,
            file_desc: child_fds,
            priority: parent_priority,
            stack: child_stack,
            context: child_context,
        };

        SCHEDULER.lock().insert_task(child_task);
        Ok(child_tid.0)
    }

    fn execve(path: &str, args: &[&str], env: &[(&str, &str)]) -> SysResult<i32> {
        let tid = current_tid()?;
        let argv: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut envp = BTreeMap::new();
        for &(k, v) in env {
            envp.insert(k.to_string(), v.to_string());
        }

        let (new_ms, mut new_ctx) =
            load_task_image(&Path::new(path), argv, &envp).map_err(|_| PosixErrno::ENOENT)?;

        let vm = {
            let sched = SCHEDULER.lock();
            let task = sched.task(tid).ok_or(PosixErrno::ESRCH)?;
            Arc::clone(&task.memory_set)
        };

        // 保留任务原有的 tgid，原地替换地址空间与 VMA。
        let new_satp = {
            let mut ms = vm.lock();
            ms.replace_with(new_ms);
            ms.satp()
        };
        new_ctx.satp = new_satp;

        let mut sched = SCHEDULER.lock();
        let task = sched.task_mut(tid).ok_or(PosixErrno::ESRCH)?;
        task.context = new_ctx;
        Ok(0)
    }

    fn read_dir(fd: usize, buf: &mut [u8]) -> SysResult<usize> {
        let file = get_fd(fd)?;
        let Some(entry) = file.read_dir().map_err(fs_err_to_errno)? else {
            return Ok(0);
        };
        let bytes = entry.name.as_bytes();
        let n = bytes.len().min(buf.len());
        buf[..n].copy_from_slice(&bytes[..n]);
        Ok(n)
    }

    fn exit(_code: i32) -> SysResult<()> {
        let tid = current_tid()?;
        SCHEDULER.lock().change_status(tid, TaskStatus::Zombie);
        Ok(())
    }

    fn wait(target_tid: usize) -> SysResult<usize> {
        let self_tid = current_tid()?;
        let mut sched = SCHEDULER.lock();

        let zombie_tid = if target_tid != 0 {
            let tid = TaskId(target_tid);
            match sched.task(tid) {
                None => return Err(PosixErrno::ECHILD),
                Some(t) if matches!(t.status, TaskStatus::Zombie) => Some(tid),
                Some(_) => None,
            }
        } else {
            sched
                .tasks()
                .find(|t| t.task_id != self_tid && matches!(t.status, TaskStatus::Zombie))
                .map(|t| t.task_id)
        };

        if let Some(ztid) = zombie_tid {
            sched.remove_task(ztid);
            drop(sched);
            free_tid(ztid);
            Ok(ztid.0)
        } else {
            Err(PosixErrno::EAGAIN)
        }
    }

    fn yield_now() -> SysResult<()> {
        Ok(())
    }
}

/// 系统调用分发后的控制流结果。
pub enum SyscallOutcome {
    /// 将返回值写入 `a0` 并返回原任务继续执行。
    Return(u64),
    /// 将返回值写入 `a0` 后主动让出 CPU，切换到下一个可运行任务。
    Yield(u64),
    /// 当前任务已退出，直接切入下一个可运行任务。
    Exit,
    /// `execve` 已替换当前任务映像与上下文，直接恢复新上下文。
    ExecReplaced,
}

/// 根据用户态寄存器 `a0..a7` 分发系统调用。
pub fn syscall_handler(regs: &Registers) -> SyscallOutcome {
    match Syscall::from(regs.a7) {
        Syscall::OPEN => {
            if regs.a0 == 0 && regs.a1 != 0 {
                return SyscallOutcome::Return((-i64::from(PosixErrno::EFAULT)) as u64);
            }
            let _sum = UserAccessGuard::enter();
            let bytes = unsafe { &*slice_from_raw_parts(regs.a0 as *const u8, regs.a1 as usize) };
            let Ok(path) = str::from_utf8(bytes) else {
                return SyscallOutcome::Return((-i64::from(PosixErrno::EINVAL)) as u64);
            };
            SyscallOutcome::Return(encode_result(
                SyscallHandler::open(path, regs.a2 as u32, regs.a3 as u16),
                |fd| fd as i64,
            ))
        }
        Syscall::CLOSE => {
            SyscallOutcome::Return(encode_result(SyscallHandler::close(regs.a0 as usize), |_| 0))
        }
        Syscall::READ => {
            if regs.a1 == 0 && regs.a2 != 0 {
                return SyscallOutcome::Return((-i64::from(PosixErrno::EFAULT)) as u64);
            }
            let _sum = UserAccessGuard::enter();
            let buf = unsafe {
                &mut *slice_from_raw_parts_mut(regs.a1 as *mut u8, regs.a2 as usize)
            };
            SyscallOutcome::Return(encode_result(
                SyscallHandler::read(regs.a0 as usize, buf),
                |n| n as i64,
            ))
        }
        Syscall::WRITE => {
            if regs.a1 == 0 && regs.a2 != 0 {
                return SyscallOutcome::Return((-i64::from(PosixErrno::EFAULT)) as u64);
            }
            let _sum = UserAccessGuard::enter();
            let buf = unsafe { &*slice_from_raw_parts(regs.a1 as *const u8, regs.a2 as usize) };
            SyscallOutcome::Return(encode_result(
                SyscallHandler::write(regs.a0 as usize, buf),
                |n| n as i64,
            ))
        }
        Syscall::STAT => {
            SyscallOutcome::Return(encode_result(SyscallHandler::stat(regs.a0 as usize), |v| {
                v as i64
            }))
        }
        Syscall::FILE_STAT => {
            if regs.a0 == 0 && regs.a1 != 0 {
                return SyscallOutcome::Return((-i64::from(PosixErrno::EFAULT)) as u64);
            }
            let _sum = UserAccessGuard::enter();
            let bytes = unsafe { &*slice_from_raw_parts(regs.a0 as *const u8, regs.a1 as usize) };
            let Ok(path) = str::from_utf8(bytes) else {
                return SyscallOutcome::Return((-i64::from(PosixErrno::EINVAL)) as u64);
            };
            SyscallOutcome::Return(encode_result(SyscallHandler::file_stat(path), |v| v as i64))
        }
        Syscall::FORK => SyscallOutcome::Return(encode_result(
            SyscallHandler::fork(
                ForkFlags::from(regs.a0),
                regs.a1 as usize,
                regs.a2 as usize,
                regs.a3 as usize,
            ),
            |tid| tid as i64,
        )),
        Syscall::EXECVE => {
            if regs.a0 == 0 && regs.a1 != 0 {
                return SyscallOutcome::Return((-i64::from(PosixErrno::EFAULT)) as u64);
            }
            // 先在开启 SUM 期间把用户态路径与 argv/envp 字符串拷贝到内核内存。
            let (path, argv_owned, env_owned) = {
                let _sum = UserAccessGuard::enter();
                let path_bytes =
                    unsafe { &*slice_from_raw_parts(regs.a0 as *const u8, regs.a1 as usize) };
                let Ok(path) = str::from_utf8(path_bytes) else {
                    return SyscallOutcome::Return((-i64::from(PosixErrno::EINVAL)) as u64);
                };
                let mut argv_owned = Vec::new();
                if regs.a2 != 0 && regs.a3 != 0 {
                    let arg_slices =
                        unsafe { &*slice_from_raw_parts(regs.a2 as *const &str, regs.a3 as usize) };
                    for &s in arg_slices {
                        argv_owned.push(s.to_string());
                    }
                }
                let mut env_owned: Vec<(String, String)> = Vec::new();
                if regs.a4 != 0 && regs.a5 != 0 {
                    let env_slices = unsafe {
                        &*slice_from_raw_parts(regs.a4 as *const (&str, &str), regs.a5 as usize)
                    };
                    for &(k, v) in env_slices {
                        env_owned.push((k.to_string(), v.to_string()));
                    }
                }
                (path.to_string(), argv_owned, env_owned)
            };

            let arg_refs: Vec<&str> = argv_owned.iter().map(String::as_str).collect();
            let env_refs: Vec<(&str, &str)> = env_owned
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            match SyscallHandler::execve(&path, &arg_refs, &env_refs) {
                Ok(_) => SyscallOutcome::ExecReplaced,
                Err(errno) => SyscallOutcome::Return((-i64::from(errno)) as u64),
            }
        }
        Syscall::SPAWN_PROCESS => {
            SyscallOutcome::Return(encode_result(SyscallHandler::spawn_process(), |tid| {
                tid as i64
            }))
        }
        Syscall::SPAWN_THREAD => SyscallOutcome::Return(encode_result(
            SyscallHandler::spawn_thread(regs.a0 as usize, regs.a1 as usize, regs.a2 as usize),
            |tid| tid as i64,
        )),
        Syscall::READ_DIR => {
            if regs.a1 == 0 && regs.a2 != 0 {
                return SyscallOutcome::Return((-i64::from(PosixErrno::EFAULT)) as u64);
            }
            let _sum = UserAccessGuard::enter();
            let buf = unsafe {
                &mut *slice_from_raw_parts_mut(regs.a1 as *mut u8, regs.a2 as usize)
            };
            SyscallOutcome::Return(encode_result(
                SyscallHandler::read_dir(regs.a0 as usize, buf),
                |n| n as i64,
            ))
        }
        Syscall::EXIT => {
            let _ = SyscallHandler::exit(regs.a0 as i32);
            SyscallOutcome::Exit
        }
        Syscall::WAIT => {
            SyscallOutcome::Return(encode_result(SyscallHandler::wait(regs.a0 as usize), |t| {
                t as i64
            }))
        }
        Syscall::YIELD => {
            let _ = SyscallHandler::yield_now();
            SyscallOutcome::Yield(0)
        }
        _ => SyscallOutcome::Return((-i64::from(PosixErrno::ENOSYS)) as u64),
    }
}
