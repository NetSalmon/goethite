#![allow(unused)]

use alloc::collections::{BTreeMap, VecDeque};
use core::arch::naked_asm;

use goethite_macros::lazy;
use goethite_riscv64::registers::csr::{Sscratch, Stvec};

use crate::{
    mm::stack::Stack,
    proc::{CURRENT_TASK, Context, STACKS, Task, TaskGroupId, TaskId, TaskStatus},
    trap::entry::__user_trap_entry,
};

#[lazy(spin)]
pub static SCHEDULER: Scheduler = Scheduler::new();

pub struct Scheduler {
    pub tasks: BTreeMap<TaskId, Task>,
    runnable: VecDeque<TaskId>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            runnable: VecDeque::new(),
        }
    }

    pub fn next(&mut self) -> TaskId {
        self.try_next().expect("scheduler: no runnable task")
    }

    pub fn try_next(&mut self) -> Option<TaskId> {
        let tid = self.runnable.pop_front()?;
        self.runnable.push_back(tid);
        Some(tid)
    }

    pub fn change_status(&mut self, task_id: TaskId, new_status: TaskStatus) {
        let Some(task) = self.tasks.get_mut(&task_id) else {
            return;
        };

        let was_runnable = task.status.runnable();
        let now_runnable = new_status.runnable();
        task.status = new_status;

        match (was_runnable, now_runnable) {
            (false, true) => self.runnable.push_back(task_id),
            (true, false) => self.runnable.retain(|&tid| tid != task_id),
            _ => {}
        }
    }

    pub fn insert_task(&mut self, task: Task) {
        let tid = task.task_id;
        if task.status.runnable() {
            self.runnable.push_back(tid);
        }
        self.tasks.insert(tid, task);
    }

    pub fn remove_task(&mut self, tid: TaskId) {
        if self.tasks.remove(&tid).is_some() {
            self.runnable.retain(|&id| id != tid);
        }
    }

    pub fn task(&self, tid: TaskId) -> Option<&Task> {
        self.tasks.get(&tid)
    }

    pub fn task_mut(&mut self, tid: TaskId) -> Option<&mut Task> {
        self.tasks.get_mut(&tid)
    }

    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn runnable_len(&self) -> usize {
        self.runnable.len()
    }

    /// 调度并切入 `task_id`：设置当前任务、准备用户态陷阱现场，然后加载其
    /// [`Context`](crate::proc::Context) 并通过 `sret` 进入目标程序。
    ///
    /// 由于 [`Context::restore`](crate::proc::Context::restore) 永不返回，本函数
    /// 也永不返回；`Result` 仅用于在任务不存在时提前失败。
    ///
    /// # 前置条件
    ///
    /// 目标地址空间必须包含内核映射（见 [`Context::restore`] 的说明），否则
    /// `sret` 后第一次陷入用户态时无法进入内核陷阱入口。
    pub fn restore(&self, task_id: TaskId) -> Result<(), ()> {
        let task = self.tasks.get(&task_id).ok_or(())?;
        let context = task.context.clone();

        // 记录当前 hart 正在运行的任务。
        *CURRENT_TASK.current() = Some(task_id);

        // 进入用户态前准备好陷阱现场：
        // - `sscratch` 保存内核栈顶，供 `__user_trap_entry` 与用户 sp 交换；
        // - `stvec` 指向用户态陷阱入口，使下一次用户陷入被识别为用户陷阱。
        let kernel_stack_top = STACKS
            .lock()
            .entry(task.task_group_id)
            .or_insert_with(Stack::new)
            .stack_top() as u64;

        Sscratch::write(kernel_stack_top);
        Stvec::write(__user_trap_entry as *const () as u64);

        context.restore()
    }

    pub fn prepare_restore(&self, task_id: TaskId) -> Result<(TaskGroupId, Context), ()> {
        let task = self.tasks.get(&task_id).ok_or(())?;
        Ok((task.task_group_id, task.context.clone()))
    }

    pub fn run(&mut self) {
        loop {
            let tid = self.next();
            self.restore(tid);
        }
    }

    /// 切入指定任务 `tid`（不持锁进入用户态，永不返回）。
    pub fn restore_task(tid: TaskId) -> ! {
        let (tgid, context) = {
            let sched = SCHEDULER.lock();
            sched
                .prepare_restore(tid)
                .expect("scheduler: target task does not exist")
        };

        *CURRENT_TASK.current() = Some(tid);

        let kernel_stack_top = STACKS
            .lock()
            .entry(tgid)
            .or_insert_with(Stack::new)
            .stack_top() as u64;

        Sscratch::write(kernel_stack_top);
        Stvec::write(__user_trap_entry as *const () as u64);

        context.restore()
    }

    /// 调度器主循环：在释放 `SCHEDULER` 锁之后再切入用户态，避免用户态陷入后
    /// 再次获取 `SCHEDULER` 锁导致死锁。
    pub fn run_global() -> ! {
        let tid = {
            let mut sched = SCHEDULER.lock();
            sched.try_next()
        };

        let Some(tid) = tid else {
            goethite_console::println!("scheduler: all tasks exited, shutting down");
            let _ = goethite_riscv64::sbi::srst::system_reset(
                goethite_riscv64::sbi::srst::ResetType::SHUTDOWN,
                goethite_riscv64::sbi::srst::ResetReason::NONE,
            );
            loop {
                core::hint::spin_loop();
            }
        };

        Self::restore_task(tid)
    }
}

extern "C" fn scheduler_entry() {
    Scheduler::run_global();
}

#[lazy]
pub static SCHEDULER_STACK: Stack = {
    let mut stack = Stack::new();
    stack.set_entry(scheduler_entry);
    stack
};

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}
