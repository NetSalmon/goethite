//! 物理内存块（[`Region`]）：从伙伴系统按阶分配、析构时自动归还的页块。
//!
//! [`Region`] 持有伙伴系统中一个固定阶（order）的物理块。它只暴露起始物理地址、
//! 字节大小与访问指针，所有访问都假定页块已被成功映射到虚拟地址（由调用方负责）。
use goethite_riscv64::addr::PhysicalAddr;

use crate::mm::{alloc::FRAME_ALLOCATOR, buddy_system::order_size};

/// 一段由伙伴系统按阶分配、析构时自动归还的物理内存块。
pub struct Region {
    start: usize,
    order: usize,
}

impl Drop for Region {
    fn drop(&mut self) {
        FRAME_ALLOCATOR.lock().dealloc(self.start, self.order);
    }
}

impl Region {
    pub fn as_ptr<T>(&self) -> *const T {
        self.start as *const T
    }

    pub fn as_mut_ptr<T>(&self) -> *mut T {
        self.start as *mut T
    }

    /// 返回该物理块的起始物理地址。
    pub fn as_phys_addr(&self) -> PhysicalAddr {
        PhysicalAddr::from(self.start)
    }

    /// 返回该物理块的字节大小（`1 << order << PAGE_SIZE_LOG_2`）。
    pub fn byte_size(&self) -> usize {
        order_size(self.order)
    }

    /// 返回该物理块在伙伴系统中的阶数。
    pub fn order(&self) -> usize {
        self.order
    }

    /// 返回该物理块的起始物理地址（字符串地址形式）。
    pub fn start(&self) -> usize {
        self.start
    }

    pub fn new(order: usize) -> Option<Self> {
        let start = FRAME_ALLOCATOR.lock().alloc(order)?;
        Some(Self { start, order })
    }
}
