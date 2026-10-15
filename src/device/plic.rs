//! PLIC（Platform-Level Interrupt Controller）中断控制器驱动。
//!
//! PLIC 汇集平台上的全部外部中断：每个**中断源**（source）有独立的优先级，
//! 每个 **context**（一个 hart 的一条中断线，如 hart0 的 M / S 模式中断线）
//! 有独立的中断使能位图与优先级阈值。一次外部中断的完整流程是
//!
//! ```text
//! 设备拉高中断线 → PLIC 置位 pending → 与 context 的使能位图 / 阈值比较
//!   → 向该 context 上报 → 内核读 claim 认领 → 处理设备 → 写 complete 完成
//! ```
//!
//! # 寄存器布局（SiFive PLIC / compatible `riscv,plic0`）
//!
//! ```text
//! 偏移          宽度              含义
//! 0x000000      4 × ndev          各中断源的优先级（源 0 保留，始终为 0）
//! 0x001000      ⌈ndev/8⌉         等待处理的中断源位图（只读）
//! 0x002000      0x80 × context    每个 context 的中断使能位图
//! 0x200000      0x1000 × context  每个 context 的阈值(+0x0) / claim-complete(+0x4)
//! ```
//!
//! # 与内核的衔接
//!
//! - [`init`]：从设备树探测控制器、设置各源优先级与当前 hart 的阈值，
//!   并允许 CPU 上报 S 模式外部中断；
//! - [`register`]：设备驱动为某个中断源登记处理函数；
//! - [`enable`] / [`disable`]：在当前 hart 的 S 模式 context 上使能 / 屏蔽中断源；
//! - [`handle_external_interrupt`]：由陷阱处理程序在收到 S 模式外部中断
//!   （`scause` 为 [`Interrupt::SUPERVISOR_EXTERNAL`](goethite_riscv64::trap::Interrupt)）
//!   时调用，完成 claim → 分派 → complete。
//!
//! 设备树中设备的 `interrupts` 属性给出的中断号**就是** PLIC 的中断源编号
//! （即 [`DeviceInfo::irq`](crate::device::DeviceInfo)），因此设备驱动接入外部
//! 中断时无需任何编号换算：
//!
//! ```ignore
//! let irq = info.irq.ok_or(DeviceError::NotReady)? as u32;
//! plic::register(irq, ack_device)?;   // 处理函数里确认设备、清中断
//! plic::enable(irq)?;                 // 在该 hart 的 S 模式 context 上放行
//! ```
//!
//! PLIC 是平台中断控制器而非块 / 字符设备（没有 [`Device`](crate::device::Device)
//! 的读写语义），因此与 `fw_cfg` / `reboot` 一样不实现
//! [`Driver`](crate::device::driver::Driver)——它没有可登记的字符设备节点，
//! 而是以全局函数 + 中断源处理函数表的形式提供服务。
#![allow(dead_code)]

use alloc::collections::BTreeMap;
use core::sync::atomic::{Ordering, fence};

use goethite_config::MAX_CPU;
use goethite_macros::{lazy, numeric};
use goethite_riscv64::{hart_id, registers::csr::Sie};

use crate::device::{
    DeviceInfo, Resource,
    desc::{DEVICE_DESC_MANAGER, DeviceDesc},
};

/// 中断源优先级寄存器区的基址（相对 MMIO 窗口）。
const PRIORITY_BASE: usize = 0x0000_0000;
/// 每个中断源的优先级寄存器宽度（字节）。
const PRIORITY_STRIDE: usize = 4;

/// 中断等待（pending）位图区的基址。
const PENDING_BASE: usize = 0x0000_1000;

/// 中断使能位图区的基址（每个 context 一份）。
const ENABLE_BASE: usize = 0x0000_2000;
/// 每个 context 的中断使能位图宽度（字节）。
const ENABLE_STRIDE: usize = 0x80;

/// context 私有寄存器区（阈值 / claim-complete）的基址。
const CONTEXT_BASE: usize = 0x0020_0000;
/// 每个 context 的私有寄存器区宽度（字节）。
const CONTEXT_STRIDE: usize = 0x1000;
/// context 区内优先级阈值寄存器的偏移。
const THRESHOLD_OFFSET: usize = 0x0;
/// context 区内 claim / complete 寄存器的偏移。
const CLAIM_OFFSET: usize = 0x4;

/// 位图 / 优先级寄存器的字宽（字节）。
const WORD: usize = 4;
/// 一个位图字覆盖的位数。
const WORD_BITS: u32 = (WORD * 8) as u32;

/// 内核支持的中断源数量上限：每个 context 的中断使能位图为 `0x80` 字节
/// （1024 位），即 PLIC 规范允许的最大中断源数量。
pub const MAX_SOURCES: u32 = 1024;

// 外部中断类型：设备树 `interrupts-extended` 中每个 context 的中断号
// （`numeric!` 生成的类型无法承载文档注释，故在此说明）。
numeric! {
    pub enum InterruptType: u32 {
        // S 模式外部中断：PLIC 路由到该 hart 的 S 模式 context。
        SUPERVISOR_EXTERNAL = 9,
        // M 模式外部中断（由固件 / M 模式代码处理）。
        MACHINE_EXTERNAL = 11,
    }
}

/// PLIC 驱动可能出现的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlicError {
    /// 设备树中没有 PLIC 节点，或其 `reg` 属性缺失（未探测到控制器）。
    #[error("plic interrupt controller not found in device tree")]
    NotFound,
    /// 指定 / 当前 hart 没有 S 模式 context，无法接收外部中断。
    #[error("no supervisor context for hart {0}")]
    NoContext(usize),
    /// 中断源编号越界（合法范围 `1..=source_count`，源 0 保留）。
    #[error("interrupt source out of range")]
    InvalidSource,
    /// 该中断源已登记处理函数。
    #[error("interrupt source already has a handler")]
    HandlerExists,
    /// 该中断源没有登记处理函数。
    #[error("interrupt source has no handler")]
    NoHandler,
}

/// PLIC 控制器的静态信息，供启动日志与调试使用。
#[derive(Debug, Clone, Copy)]
pub struct PlicInfo {
    /// MMIO 窗口的起始物理地址。
    pub base: usize,
    /// 中断源数量（设备树 `riscv,ndev`）。
    pub sources: u32,
    /// 当前 hart 的 S 模式 context 编号。
    pub context: usize,
}

/// PLIC 控制器：MMIO 窗口、中断源数量与各 hart 的 S 模式 context 编号。
///
/// 自身不保存任何寄存器状态（寄存器都在 MMIO 中），因此是 `Copy` 的；
/// 对寄存器的访问通过 [`context`](Self::context) /
/// [`supervisor_context`](Self::supervisor_context) 取出的 [`Context`] 句柄进行。
#[derive(Debug, Clone, Copy)]
pub struct Plic {
    /// 控制器寄存器所在的 MMIO 窗口。
    device: DeviceInfo,
    /// 中断源数量（`riscv,ndev`）。
    sources: u32,
    /// 各 hart 的 S 模式 context 编号；`None` 表示该 hart 没有 S 模式中断线。
    supervisor_contexts: [Option<usize>; MAX_CPU],
}

/// PLIC 设备树 `compatible` 串（从最具体到最通用）。
const COMPATIBLES: [&str; 2] = ["sifive,plic-1.0.0", "riscv,plic0"];

impl Plic {
    /// 从控制器节点描述构造：解析 MMIO 窗口、`riscv,ndev` 与
    /// `interrupts-extended` 中的 S 模式 context。
    ///
    /// 节点缺少 `reg` 时返回 `None`。
    pub fn new(desc: &DeviceDesc) -> Option<Self> {
        Some(Self {
            device: DeviceInfo::from_desc(desc)?,
            sources: parse_sources(desc),
            supervisor_contexts: parse_supervisor_contexts(desc),
        })
    }

    /// 从设备描述管理器探测 PLIC 控制器。
    pub fn probe() -> Option<Self> {
        let descs = DEVICE_DESC_MANAGER.read();
        descs
            .all
            .iter()
            .find(|desc| {
                desc.compatible
                    .iter()
                    .any(|compatible| COMPATIBLES.contains(&compatible.as_str()))
            })
            .and_then(|desc| Self::new(desc))
    }

    /// 中断源数量。
    pub fn source_count(&self) -> u32 {
        self.sources
    }

    /// 控制器寄存器所在的 MMIO 窗口。
    pub fn mmio(&self) -> Resource {
        self.device.mmio
    }

    /// 中断源编号是否合法（`1..=source_count`；源 0 保留）。
    pub fn is_valid_source(&self, source: u32) -> bool {
        source >= 1 && source <= self.sources
    }

    /// 指定 hart 的 S 模式 context 句柄。
    pub fn context(&self, hart: usize) -> Option<Context> {
        Some(Context {
            mmio: self.device.mmio,
            index: self.supervisor_contexts.get(hart).copied().flatten()?,
            sources: self.sources,
        })
    }

    /// 当前 hart 的 S 模式 context 句柄（内核的外中断入口即由它触发）。
    pub fn supervisor_context(&self) -> Option<Context> {
        self.context(hart_id())
    }
}

/// 一个 PLIC context 上的寄存器访问句柄。
///
/// 只携带 MMIO 窗口与 context 编号，是 `Copy` 的：调用方可以在**不持有**
/// [`PLIC`] 全局锁的情况下完成 claim / complete，从而避免在处理函数中重入
/// 本模块接口时死锁。
#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// context 寄存器所在的 MMIO 窗口。
    mmio: Resource,
    /// context 编号（PLIC 全局唯一，按设备树 `interrupts-extended` 顺序编号）。
    index: usize,
    /// 该 context 可见的中断源数量。
    sources: u32,
}

impl Context {
    /// context 编号。
    pub fn index(&self) -> usize {
        self.index
    }

    /// 该 context 可见的中断源数量。
    pub fn source_count(&self) -> u32 {
        self.sources
    }

    /// 校验中断源编号，并返回其优先级寄存器偏移。
    fn priority_offset(&self, source: u32) -> Result<usize, PlicError> {
        (source >= 1 && source <= self.sources)
            .then(|| PRIORITY_BASE + PRIORITY_STRIDE * source as usize)
            .ok_or(PlicError::InvalidSource)
    }

    /// 使能位图寄存器（字）的偏移与该源对应的位掩码。
    fn enable_bit(&self, source: u32) -> (usize, u32) {
        let word = (source / WORD_BITS) as usize;
        (
            ENABLE_BASE + ENABLE_STRIDE * self.index + WORD * word,
            1 << (source % WORD_BITS),
        )
    }

    /// context 私有寄存器区内某偏移处的地址。
    fn private(&self, offset: usize) -> usize {
        CONTEXT_BASE + CONTEXT_STRIDE * self.index + offset
    }

    /// 中断源 `source` 的优先级（0 表示该源永远不会上报；越界时为 0）。
    pub fn priority(&self, source: u32) -> u32 {
        match self.priority_offset(source) {
            Ok(offset) => self.mmio.read::<u32>(offset),
            Err(_) => 0,
        }
    }

    /// 设置中断源 `source` 的优先级。
    ///
    /// # Errors
    ///
    /// - [`PlicError::InvalidSource`]：源编号越界。
    pub fn set_priority(&self, source: u32, priority: u32) -> Result<(), PlicError> {
        let offset = self.priority_offset(source)?;
        self.mmio.write::<u32>(offset, priority);
        Ok(())
    }

    /// 在该 context 上使能中断源 `source`。
    ///
    /// # Errors
    ///
    /// - [`PlicError::InvalidSource`]：源编号越界。
    pub fn enable(&self, source: u32) -> Result<(), PlicError> {
        self.priority_offset(source)?;
        let (offset, mask) = self.enable_bit(source);
        self.mmio
            .write::<u32>(offset, self.mmio.read::<u32>(offset) | mask);
        Ok(())
    }

    /// 在该 context 上屏蔽中断源 `source`。
    ///
    /// # Errors
    ///
    /// - [`PlicError::InvalidSource`]：源编号越界。
    pub fn disable(&self, source: u32) -> Result<(), PlicError> {
        self.priority_offset(source)?;
        let (offset, mask) = self.enable_bit(source);
        self.mmio
            .write::<u32>(offset, self.mmio.read::<u32>(offset) & !mask);
        Ok(())
    }

    /// 中断源 `source` 在该 context 上是否已使能（源编号越界时为 `false`）。
    pub fn is_enabled(&self, source: u32) -> bool {
        if self.priority_offset(source).is_err() {
            return false;
        }
        let (offset, mask) = self.enable_bit(source);
        self.mmio.read::<u32>(offset) & mask != 0
    }

    /// 屏蔽该 context 上的全部中断源（清除固件遗留的使能状态）。
    pub fn disable_all(&self) {
        for word in 0..self.sources.div_ceil(WORD_BITS) as usize {
            let offset = ENABLE_BASE + ENABLE_STRIDE * self.index + WORD * word;
            self.mmio.write::<u32>(offset, 0);
        }
    }

    /// 该 context 的优先级阈值（只有优先级**大于**阈值的源才会上报）。
    pub fn threshold(&self) -> u32 {
        self.mmio.read::<u32>(self.private(THRESHOLD_OFFSET))
    }

    /// 设置该 context 的优先级阈值（`0` 表示放行所有非零优先级的源）。
    pub fn set_threshold(&self, threshold: u32) {
        self.mmio
            .write::<u32>(self.private(THRESHOLD_OFFSET), threshold);
    }

    /// 中断源 `source` 当前是否处于等待处理（pending）状态。
    ///
    /// 认领后该位被清除，直到对应的 complete 写入为止。
    pub fn is_pending(&self, source: u32) -> bool {
        if self.priority_offset(source).is_err() {
            return false;
        }
        let offset = PENDING_BASE + WORD * (source / WORD_BITS) as usize;
        self.mmio.read::<u32>(offset) & (1 << (source % WORD_BITS)) != 0
    }

    /// 认领该 context 上当前优先级最高的等待中断。
    ///
    /// 读 claim 寄存器会原子地清除该源的等待位并把它标记为「处理中」；
    /// 返回 `None` 表示没有可认领的中断（伪中断）。
    pub fn claim(&self) -> Option<u32> {
        match self.mmio.read::<u32>(self.private(CLAIM_OFFSET)) {
            0 => None,
            source => Some(source),
        }
    }

    /// 完成中断源 `source` 的处理，允许它再次上报。
    ///
    /// 写 complete 之前先做一次全序栅栏：保证处理函数对**设备**的确认写
    /// （如 virtio 的 `interrupt_ack`）对设备可见，否则 PLIC 可能在该确认
    /// 之前就把同一中断重新上报，处理函数看到的仍是未确认的设备状态。
    pub fn complete(&self, source: u32) {
        fence(Ordering::SeqCst);
        self.mmio.write::<u32>(self.private(CLAIM_OFFSET), source);
    }
}

/// 中断处理函数：参数为被认领的中断源编号。
pub type IrqHandler = fn(u32);

/// 已登记的中断源处理函数：中断源编号 → 处理函数。
#[lazy(rwlock)]
static HANDLERS: BTreeMap<u32, IrqHandler> = BTreeMap::new();

/// 全内核唯一的 PLIC 控制器，首次访问时从设备树探测。
#[lazy(spin)]
pub static PLIC: Option<Plic> = Plic::probe();

/// 解析 `riscv,ndev`（大端 u32）；属性缺失或为 0 时按规范上限处理。
fn parse_sources(desc: &DeviceDesc) -> u32 {
    desc.props
        .get("riscv,ndev")
        .and_then(|raw| be_cells(raw).next())
        .filter(|&ndev| ndev > 0)
        .unwrap_or(MAX_SOURCES)
        .min(MAX_SOURCES)
}

/// 解析 `interrupts-extended`，取出各 hart 的 S 模式 context 编号。
///
/// 该属性是 `(中断控制器 phandle, 中断号)` 单元对构成的列表，**每个单元对
/// 对应一个 PLIC context，其下标即 context 编号**。设备树按 hart 顺序列出各
/// hart 的中断线，故第 n 个 S 模式中断线属于第 n 个 hart。属性缺失时按 SiFive
/// 约定回退为 `context = hart * 2 + 1`（每个 hart 依次是 M、S 模式）。
fn parse_supervisor_contexts(desc: &DeviceDesc) -> [Option<usize>; MAX_CPU] {
    let mut contexts = [None; MAX_CPU];

    let Some(raw) = desc.props.get("interrupts-extended") else {
        for (hart, context) in contexts.iter_mut().enumerate() {
            *context = Some(hart * 2 + 1);
        }
        return contexts;
    };

    let mut hart = 0;
    for (cell, value) in be_cells(raw).enumerate() {
        // 偶数单元是控制器 phandle，奇数单元才是中断号。
        if cell % 2 == 0 {
            continue;
        }
        if InterruptType::from(value) == InterruptType::SUPERVISOR_EXTERNAL {
            if let Some(context) = contexts.get_mut(hart) {
                *context = Some(cell / 2);
            }
            hart += 1;
        }
    }

    contexts
}

/// 把 FDT 原始属性值按大端 32 位单元解码（尾部不足 4 字节的部分被忽略）。
fn be_cells(raw: &[u8]) -> impl Iterator<Item = u32> + '_ {
    raw.as_chunks::<4>()
        .0
        .iter()
        .map(|cell| u32::from_be_bytes(*cell))
}

/// 已探测到的 PLIC 控制器（`Copy` 副本）。
///
/// # Errors
///
/// - [`PlicError::NotFound`]：设备树中没有 PLIC 节点。
pub fn controller() -> Result<Plic, PlicError> {
    (*PLIC.lock()).ok_or(PlicError::NotFound)
}

/// 当前 hart 的 S 模式 context 句柄。
///
/// # Errors
///
/// - [`PlicError::NotFound`]：设备树中没有 PLIC 节点。
/// - [`PlicError::NoContext`]：当前 hart 没有 S 模式 context。
pub fn current_context() -> Result<Context, PlicError> {
    controller()?
        .supervisor_context()
        .ok_or(PlicError::NoContext(hart_id()))
}

/// 初始化 PLIC：设置各中断源优先级、清空当前 hart 的使能位图并把阈值清零，
/// 最后允许 CPU 上报 S 模式外部中断（`sie.SEIE`）。
///
/// 是否真正投递还取决于 `sstatus.SIE`（由调度器在进入用户态 / 空闲等待时开启）
/// 以及各中断源是否已通过 [`enable`] 使能。
///
/// # Errors
///
/// - [`PlicError::NotFound`]：设备树中没有 PLIC 节点。
/// - [`PlicError::NoContext`]：当前 hart 没有 S 模式 context。
pub fn init() -> Result<PlicInfo, PlicError> {
    let plic = controller()?;
    let context = current_context()?;

    // 源优先级对所有 context 全局生效：统一置 1（0 表示屏蔽），
    // 某个源是否上报仍由各 context 的使能位图单独决定。
    for source in 1..=plic.source_count() {
        context.set_priority(source, 1)?;
    }

    // 清除固件 / 上一位使用者遗留的使能状态与阈值，避免未注册的源触发中断。
    context.disable_all();
    context.set_threshold(0);

    Sie::set_bits(1u64 << InterruptType::SUPERVISOR_EXTERNAL.0);

    Ok(PlicInfo {
        base: plic.mmio().start,
        sources: plic.source_count(),
        context: context.index(),
    })
}

/// 为中断源 `source` 登记处理函数（不改变该源的使能状态）。
///
/// # Errors
///
/// - [`PlicError::NotFound`]：设备树中没有 PLIC 节点。
/// - [`PlicError::InvalidSource`]：源编号越界。
/// - [`PlicError::HandlerExists`]：该源已登记处理函数。
pub fn register(source: u32, handler: IrqHandler) -> Result<(), PlicError> {
    if !controller()?.is_valid_source(source) {
        return Err(PlicError::InvalidSource);
    }

    let mut handlers = HANDLERS.write();
    if handlers.contains_key(&source) {
        return Err(PlicError::HandlerExists);
    }
    handlers.insert(source, handler);
    Ok(())
}

/// 注销中断源 `source` 的处理函数（不改变该源的使能状态）。
///
/// # Errors
///
/// - [`PlicError::NoHandler`]：该源没有登记处理函数。
pub fn unregister(source: u32) -> Result<(), PlicError> {
    HANDLERS
        .write()
        .remove(&source)
        .map(|_| ())
        .ok_or(PlicError::NoHandler)
}

/// 在当前 hart 的 S 模式 context 上使能中断源 `source`。
///
/// # Errors
///
/// - [`PlicError::NotFound`] / [`PlicError::NoContext`]：控制器或 context 不可用。
/// - [`PlicError::InvalidSource`]：源编号越界。
pub fn enable(source: u32) -> Result<(), PlicError> {
    current_context()?.enable(source)
}

/// 在当前 hart 的 S 模式 context 上屏蔽中断源 `source`。
///
/// # Errors
///
/// 同 [`enable`]。
pub fn disable(source: u32) -> Result<(), PlicError> {
    current_context()?.disable(source)
}

/// 处理一次 S 模式外部中断：认领中断源 → 调用其处理函数 → 完成中断。
///
/// 由陷阱处理程序在 `scause` 为 S 模式外部中断时调用。返回 `false` 表示没有
/// 可认领的中断（控制器不可用或是一次伪中断）。
///
/// 认领与完成之间**不持有任何全局锁**：处理函数可以自由调用本模块的其它接口。
/// 未登记处理函数的中断源会被直接完成，以免该源永久停留在「处理中」状态。
pub fn handle_external_interrupt() -> bool {
    let Ok(context) = current_context() else {
        return false;
    };
    let Some(source) = context.claim() else {
        return false;
    };

    // 先把处理函数取出来（读锁在本语句结束时释放），再调用——否则读锁会一直
    // 持有到 `if let` 结束，处理函数若调用 register / unregister 就会死锁。
    let handler = HANDLERS.read().get(&source).copied();
    if let Some(handler) = handler {
        handler(source);
    }

    context.complete(source);
    true
}
