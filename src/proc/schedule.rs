#![allow(unused)]

use alloc::collections::{BTreeMap, VecDeque};
use goethite_macros::lazy;
use crate::proc::{Task, TaskId, TaskStatus};


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
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}
