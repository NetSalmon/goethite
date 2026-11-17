#![allow(unused)]

use alloc::collections::{BTreeMap, VecDeque};
use core::arch::naked_asm;

use goethite_macros::lazy;
use goethite_riscv64::registers::csr::{Sscratch, Stvec};

use crate::{
    KERNEL_STACK_SIZE,
    mm::stack::Stack,
    proc::{CURRENT_TASK, Task, TaskId, TaskStatus},
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
        let context = self.tasks.get(&task_id).ok_or(())?.context.clone();

        // 记录当前 hart 正在运行的任务。
        *CURRENT_TASK.current() = Some(task_id);

        // 进入用户态前准备好陷阱现场：
        // - `sscratch` 保存内核栈顶，供 `__user_trap_entry` 与用户 sp 交换；
        // - `stvec` 指向用户态陷阱入口，使下一次用户陷入被识别为用户陷阱。
        let kernel_stack_top = crate::KERNEL_STACK.as_ptr() as usize + KERNEL_STACK_SIZE;
        Sscratch::write(kernel_stack_top as u64);
        Stvec::write(__user_trap_entry as *const () as u64);

        context.restore()
    }

    pub fn run(&mut self) {
        loop {
            let tid = self.next();
            self.restore(tid);
        }
    }
}

#[lazy]
pub static SCHEDULER_STACK: Stack = Stack::new();

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}
