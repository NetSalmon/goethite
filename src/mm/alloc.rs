//! 内核全局内存分配器。
//!
//! 提供两个入口：
//!
//! - [`FRAME_ALLOCATOR`]：面向物理页帧的伙伴系统（[`BuddyAllocator`]），
//!   由 [`#[lazy(spin)]`](lazy) 包装，首次访问时才初始化。
//! - [`GLOBAL_ALLOCATOR`]：面向字节的 [`GlobalAlloc`]，底层是
//!   [`Caches`]（SLUB 对象缓存），供 `alloc` / `Vec` 等使用。
use core::alloc::{GlobalAlloc, Layout};

use goethite_config::BUDDY_MAX_ORDER;
use goethite_macros::lazy;
use goethite_sync::LazyLock;

use crate::mm::{buddy_system::BuddyAllocator, slub::Caches};

/// 全局物理页帧分配器，按 `BUDDY_MAX_ORDER` 参数化阶数，懒加载并以上锁保护。
#[lazy(spin)]
pub static FRAME_ALLOCATOR: BuddyAllocator<{ BUDDY_MAX_ORDER }> = BuddyAllocator::new();

/// 内核堆分配器：内部懒加载一个 SLUB 缓存集合 [`Caches`]。
pub struct GlobalAllocator(LazyLock<Caches>);

impl GlobalAllocator {
    /// 构造一个尚未初始化的分配器（实际的 [`Caches`] 在首次分配时才懒加载）。
    const fn new() -> GlobalAllocator {
        GlobalAllocator(LazyLock::new(|| Caches::new()))
    }
}

/// 把 [`GlobalAllocator`] 适配为 [`GlobalAlloc`]，因此可安全地用于 `#[global_allocator]`。
///
/// # Safety
///
/// `GlobalAlloc` 的方法委托给内层 `LazyLock<Caches>`；`Caches` 的并发访问由
/// `PerCpu` 与自旋锁串行化，满足 `GlobalAlloc` 对线程安全的要求。
unsafe impl GlobalAlloc for GlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.force().alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.force().dealloc(ptr, layout)
    }
}

/// 注册为全内核唯一的 `#[global_allocator]`。
#[global_allocator]
pub static GLOBAL_ALLOCATOR: GlobalAllocator = GlobalAllocator::new();
