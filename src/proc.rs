pub mod exec;
pub mod schedule;
pub mod vma;

use alloc::{collections::BTreeMap, rc::Weak, sync::Arc, vec::Vec};
use core::{arch::naked_asm, mem::offset_of};

use goethite_config::MAX_CPU;
use goethite_id_alloc::{Id, IdAlloc};
use goethite_macros::lazy;
use goethite_riscv64::registers::Registers;
use goethite_sync::{LazyLock, PerCpu, SpinLock};

use crate::{fs::FileDesc, mm::stack::Stack, proc::vma::MemorySet};

#[lazy]
pub static CURRENT_TASK: PerCpu<Option<TaskId>, MAX_CPU> = PerCpu::new([None; MAX_CPU]);

/// 一次上下文切换需保存 / 恢复的寄存器集合。
#[derive(Debug, Clone)]
pub struct Context {
    /// 触发切换的程序计数器（`sepc`）。
    pub sepc: u64,
    /// 切换前的状态寄存器（`sstatus`）。
    pub sstatus: u64,
    /// 切换前的地址空间根页表（`satp`）。
    pub satp: u64,

    /// 32 个通用寄存器的值（`x0..x31`）。
    pub registers: Registers,
}

impl Context {
    /// 加载本上下文并 `sret` 进入目标程序（通常是用户态），永不返回。
    ///
    /// 依次完成：
    /// 1. 从上下文取出 `satp` / `sepc` / `sstatus` 并写入对应 CSR；
    /// 2. 按 [`Registers`] 布局恢复全部通用寄存器（`x0` 恒为 0 跳过，`t6` 作基址最后恢复）；
    /// 3. 执行 `sret`，按 `sepc` 取指、按 `sstatus.SPP` 决定返回特权级。
    ///
    /// # 前置条件
    ///
    /// 切到 `satp` 后仍需继续取指 / 读取本上下文（`&self` 位于内核栈），因此目标
    /// 地址空间**必须包含内核映射**（内核镜像与线性映射）；否则会在 `csrw satp`
    /// 之后立刻触发缺页。
    pub fn restore(&self) -> ! {
        unsafe {
            __context_restore(self as *const Context);
            core::hint::unreachable_unchecked()
        }
    }
}

/// [`Registers`] 在 [`Context`] 内的起始偏移；各寄存器再叠加其在
/// [`Registers`] 内的偏移，得到相对 `Context` 指针的偏移。
const CONTEXT_REGISTERS: usize = offset_of!(Context, registers);

/// [`Context::restore`] 的裸汇编实现：`a0` 指向待恢复的 [`Context`]。
///
/// 先把 CSR 值读入 `t3`/`t4`/`t5`（此时仍在原地址空间，可安全读内核内存），
/// 再切 `satp` 并写 `sepc`/`sstatus`，最后逐字节恢复通用寄存器并 `sret`。
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn __context_restore(_context: *const Context) {
    naked_asm!(
        // a0 = &Context
        "mv t6, a0",
        // 取 CSR 值到临时寄存器（切 satp 之前完成对内核内存的读取）
        "ld t3, {satp}(t6)",
        "ld t4, {sepc}(t6)",
        "ld t5, {sstatus}(t6)",
        // 切换地址空间
        "csrw satp, t3",
        "sfence.vma",
        // 设置 sret 返回现场
        "csrw sepc, t4",
        "csrw sstatus, t5",
        // 恢复通用寄存器（x0 硬连零；t6 作为基址最后恢复）
        "ld ra, {ra}(t6)",
        "ld sp, {sp}(t6)",
        "ld gp, {gp}(t6)",
        "ld tp, {tp}(t6)",
        "ld t0, {t0}(t6)",
        "ld t1, {t1}(t6)",
        "ld t2, {t2}(t6)",
        "ld s0, {s0}(t6)",
        "ld s1, {s1}(t6)",
        "ld a0, {a0}(t6)",
        "ld a1, {a1}(t6)",
        "ld a2, {a2}(t6)",
        "ld a3, {a3}(t6)",
        "ld a4, {a4}(t6)",
        "ld a5, {a5}(t6)",
        "ld a6, {a6}(t6)",
        "ld a7, {a7}(t6)",
        "ld s2, {s2}(t6)",
        "ld s3, {s3}(t6)",
        "ld s4, {s4}(t6)",
        "ld s5, {s5}(t6)",
        "ld s6, {s6}(t6)",
        "ld s7, {s7}(t6)",
        "ld s8, {s8}(t6)",
        "ld s9, {s9}(t6)",
        "ld s10, {s10}(t6)",
        "ld s11, {s11}(t6)",
        "ld t3, {t3}(t6)",
        "ld t4, {t4}(t6)",
        "ld t5, {t5}(t6)",
        "ld t6, {t6}(t6)",
        "sret",
        satp = const offset_of!(Context, satp),
        sepc = const offset_of!(Context, sepc),
        sstatus = const offset_of!(Context, sstatus),
        ra = const CONTEXT_REGISTERS + offset_of!(Registers, ra),
        sp = const CONTEXT_REGISTERS + offset_of!(Registers, sp),
        gp = const CONTEXT_REGISTERS + offset_of!(Registers, gp),
        tp = const CONTEXT_REGISTERS + offset_of!(Registers, tp),
        t0 = const CONTEXT_REGISTERS + offset_of!(Registers, t0),
        t1 = const CONTEXT_REGISTERS + offset_of!(Registers, t1),
        t2 = const CONTEXT_REGISTERS + offset_of!(Registers, t2),
        s0 = const CONTEXT_REGISTERS + offset_of!(Registers, s0),
        s1 = const CONTEXT_REGISTERS + offset_of!(Registers, s1),
        a0 = const CONTEXT_REGISTERS + offset_of!(Registers, a0),
        a1 = const CONTEXT_REGISTERS + offset_of!(Registers, a1),
        a2 = const CONTEXT_REGISTERS + offset_of!(Registers, a2),
        a3 = const CONTEXT_REGISTERS + offset_of!(Registers, a3),
        a4 = const CONTEXT_REGISTERS + offset_of!(Registers, a4),
        a5 = const CONTEXT_REGISTERS + offset_of!(Registers, a5),
        a6 = const CONTEXT_REGISTERS + offset_of!(Registers, a6),
        a7 = const CONTEXT_REGISTERS + offset_of!(Registers, a7),
        s2 = const CONTEXT_REGISTERS + offset_of!(Registers, s2),
        s3 = const CONTEXT_REGISTERS + offset_of!(Registers, s3),
        s4 = const CONTEXT_REGISTERS + offset_of!(Registers, s4),
        s5 = const CONTEXT_REGISTERS + offset_of!(Registers, s5),
        s6 = const CONTEXT_REGISTERS + offset_of!(Registers, s6),
        s7 = const CONTEXT_REGISTERS + offset_of!(Registers, s7),
        s8 = const CONTEXT_REGISTERS + offset_of!(Registers, s8),
        s9 = const CONTEXT_REGISTERS + offset_of!(Registers, s9),
        s10 = const CONTEXT_REGISTERS + offset_of!(Registers, s10),
        s11 = const CONTEXT_REGISTERS + offset_of!(Registers, s11),
        t3 = const CONTEXT_REGISTERS + offset_of!(Registers, t3),
        t4 = const CONTEXT_REGISTERS + offset_of!(Registers, t4),
        t5 = const CONTEXT_REGISTERS + offset_of!(Registers, t5),
        t6 = const CONTEXT_REGISTERS + offset_of!(Registers, t6),
    )
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
    /// 本任务的内核栈；`Stack` 的克隆通过 `RefHandle` 共享同一段栈内存。
    pub stack: Stack,
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

/// 线程 id（tid）：进程内每个线程唯一。
#[derive(Id)]
pub struct TaskId(pub usize);

/// 线程组（进程）id（tgid）：同一进程的所有线程共享同一 tgid 与地址空间。
#[derive(Id)]
pub struct TaskGroupId(pub usize);

#[lazy(spin)]
pub static STACKS: BTreeMap<TaskGroupId, Stack> = BTreeMap::new();
