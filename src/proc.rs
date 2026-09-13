mod schedule;
pub mod vma;

use alloc::{rc::Weak, sync::Arc, vec::Vec};

use goethite_id_alloc::{Id, IdAlloc};
use goethite_macros::lazy;

use crate::{fs::FileDesc, proc::vma::MemorySet};

/// 一次上下文切换需保存 / 恢复的寄存器集合。
pub struct Context {
    /// 触发切换的程序计数器（`sepc`）。
    pub sepc: u64,
    /// 切换前的状态寄存器（`sstatus`）。
    pub sstatus: u64,
    /// 切换前的地址空间根页表（`satp`）。
    pub satp: u64,

    /// 32 个通用寄存器的值（`x0..x31`）。
    pub registers: [u64; 32],
}

/// 一个内核线程（task）：持有可恢复的 [`Context`]，既是调度的最小单位。
pub struct Task {
    pub task_id: TaskId,
    pub task_group_id: TaskGroupId,
    pub parent: Option<Weak<Task>>,
    pub children: Vec<Arc<Task>>,

    pub memory_set: MemorySet,
    pub status: TaskStatus,

    pub file_desc: Vec<FileDesc>,

    pub priority: u8,
    pub context: Context,
}

pub enum TaskStatus {
    Ready,
    Running,
    Blocking,
    Zombie,
}

impl TaskStatus {
    pub fn runnable(&self) -> bool {
        match self {
            TaskStatus::Ready | TaskStatus::Running => true,
            _ => false,
        }
    }
}

/// 线程 id（tid）：进程内每个线程唯一。
#[derive(Id)]
pub struct TaskId(pub usize);

/// 线程组（进程）id（tgid）：同一进程的所有线程共享同一 tgid 与地址空间。
#[derive(Id)]
pub struct TaskGroupId(pub usize);

/// 单个进程可容纳的最大线程数，即一个 tgid 块的大小 `2^TASK_ID_MINOR_BITS`。
const TASK_ID_MINOR_BITS: usize = 5;

/// tgid / tid 两级分配器（值域 `[0, goethite_config::TID_MAX)`）：
///
/// - [`alloc_major`](IdAlloc::alloc_major) 预留一个 `2^TASK_ID_MINOR_BITS` 对齐块，
///   得到一个 **tgid**；
/// - [`alloc_minor`](IdAlloc::alloc_minor) 在该 tgid 块内分配一个 **tid**。
///
/// 这与经典 pid 模型的「进程（tgid）→ 线程（tid）」两级结构一致，也是
/// `goethite-id-alloc` 两级分配设计的典型用法。
#[lazy(spin)]
static TASK_ID_ALLOC: IdAlloc<TaskGroupId, TASK_ID_MINOR_BITS> =
    IdAlloc::from_range(TaskGroupId::MIN..TaskGroupId(goethite_config::TID_MAX))
        .expect("task id range must be aligned to 2^TASK_ID_MINOR_BITS");

/// 分配一个新的进程（tgid）。
pub fn alloc_tgid() -> Option<TaskGroupId> {
    TASK_ID_ALLOC.lock().alloc_major()
}

/// 在给定 tgid 内分配一个线程（tid）。
pub fn alloc_tid(tgid: TaskGroupId) -> Option<TaskId> {
    TASK_ID_ALLOC
        .lock()
        .alloc_minor(tgid)
        .map(|id: TaskGroupId| TaskId(id.0))
}

/// 释放一个 tid（自动路由到其所属 tgid 的子分配器）。
pub fn free_tid(tid: TaskId) {
    TASK_ID_ALLOC.lock().dealloc(TaskGroupId(tid.0)).ok();
}

/// 释放整个 tgid（仅当其中已无存活线程时安全）。
pub fn free_tgid(tgid: TaskGroupId) {
    TASK_ID_ALLOC.lock().dealloc_major(tgid).ok();
}
