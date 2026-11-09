//! 内存布局常量与懒加载范围。
//!
//! [`PAGE_SIZE`] 为页大小；[`BUDDY_MAX_ORDER`] 与 SLUB 参数
//! （[`SLUB_MAX_ORDER`] / [`SLUB_MIN_ORDER`]）限定内核分配器行为；
//! [`KERNEL_OFFSET`] 为内核虚拟地址偏移；[`USER_STACK_TOP`]、
//! [`USER_STACK_SIZE`] 与 [`USER_STACK_LOWER_BOUND`] 定义用户栈布局。
//! `DEFAULT_MEMORY_START` / `DEFAULT_MEMORY_SIZE` 为设备树未指定内存
//! 范围时的默认物理内存起始地址与大小。
use goethite_macros::const_val;

/// 默认物理内存起始地址（设备树未指定内存范围时使用）。
#[const_val]
const DEFAULT_MEMORY_START: usize = 0x80000000;
/// 默认物理内存大小（128 MiB）。
#[const_val]
const DEFAULT_MEMORY_SIZE: usize = 0x8000000;
/// 页大小（4 KiB），页表与地址对齐的基本单位。
#[const_val]
pub const PAGE_SIZE: usize = 4096;

/// 计算 `size` 的向上取整、以 2 为底的对数（`ilog2` 的 ceiling）。
///
/// `size == 1` 时返回 0；否则等于 `(size - 1).ilog2() + 1`。
#[inline]
pub const fn ilog2_ceil(size: usize) -> usize {
    if size == 1 {
        0
    } else {
        (size - 1).ilog2() as usize + 1
    }
}

/// 将 `addr` 向上对齐到 `align` 的整数倍（`align` 必须是 2 的幂）。
#[inline(always)]
pub const fn align(addr: usize, align: usize) -> usize {
    let mask = align.wrapping_sub(1);
    addr.wrapping_add(mask) & !mask
}

/// 伙伴分配器的最大阶数：`PAGE_SIZE << BUDDY_MAX_ORDER` 为单个最大块（8 MiB）。
#[const_val]
pub const BUDDY_MAX_ORDER: usize = 11;
/// 页大小以 2 为底的对数，等价于 `ilog2(PAGE_SIZE)`。
pub const PAGE_SIZE_LOG_2: usize = ilog2_ceil(PAGE_SIZE);
/// 内核堆（SLUB/heap）允许的最大大小（20 MiB）。
#[const_val]
pub const MAX_KERNEL_HEAP_SIZE: usize = 20 * 1024 * 1024;
/// SLUB 缓存页的最大阶数，超过 `SLUB_MAX_ORDER` 的对象直接走伙伴系统。
#[const_val(max = 12)]
pub const SLUB_MAX_ORDER: usize = 12;
/// SLUB 缓存页的最小阶数（对应页大小 `PAGE_SIZE << SLUB_MIN_ORDER`）。
#[const_val(min = 3)]
pub const SLUB_MIN_ORDER: usize = 4;
/// SLUB 缓存数量（`SLUB_MAX_ORDER - SLUB_MIN_ORDER`），即 `SlowCaches` 数组长度。
pub const CACHES_MAX: usize = SLUB_MAX_ORDER - SLUB_MIN_ORDER;

/// 每个 SLUB 页最少容纳的对象数，不足则尝试扩大页大小。
pub const SLUB_MIN_OBJECTS: usize = 4;
/// 单个 SLUB 页大小上限（1 MiB），超过则不再扩大。
pub const SLUB_MAX_PAGE_SIZE: usize = 1 << 20;

/// 单次恒等映射的块大小（8 MiB）。分块映射可以限制单次页表更新的工作量，
/// 也避免启动阶段一次性占用过多内存管理器资源。
pub const IDENTITY_MAP_CHUNK: usize = 8 * 1024 * 1024;

/// 内核虚拟地址偏移：物理地址加上该值得到内核虚拟地址（对应链接脚本 `KERNEL_OFFSET`）。
pub const KERNEL_OFFSET: usize = 0xffff_ffc0_0000_0000;

/// 用户栈大小（`PAGE_SIZE * 8`，即 32 KiB），须为页大小的整数倍。
#[const_val(multiple_of = PAGE_SIZE)]
pub const USER_STACK_SIZE: usize = PAGE_SIZE * 8;
/// 用户栈顶虚拟地址（4 GiB，即 `0x1_0000_0000`），须为页大小的整数倍。
#[const_val(multiple_of = PAGE_SIZE)]
pub const USER_STACK_TOP: usize = 0x1_0000_0000;

/// 用户栈最低地址（`USER_STACK_TOP - USER_STACK_SIZE`）。
pub const USER_STACK_LOWER_BOUND: usize = USER_STACK_TOP - USER_STACK_SIZE;

/// 单级页表的页表项数量（RISC-V Sv39 每级固定 512 项）。
pub const PTE_NUMBER: usize = 512;

pub const fn vpn2(addr: usize) -> usize {
    const MASK: usize = ((1 << (38 - 30 + 1)) - 1) << 30;
    (addr & MASK) >> 30
}

pub const KERNEL_PTE_START: usize = vpn2(KERNEL_OFFSET);

pub const PAGE_TABLE_SIZE: usize = 512;
