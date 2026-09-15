//! 物理内存块（[`Region`]）：从伙伴系统按阶分配、析构时自动归还的页块。
//!
//! [`Region`] 持有伙伴系统中一个固定阶（order）的页块。它暴露起始**虚拟地址**
//! （伙伴系统返回的高半区地址）与字节大小，因此 [`as_ptr`](Region::as_ptr) 得到的
//! 指针可直接解引用；需要物理地址时用 [`as_phys_addr`](Region::as_phys_addr) 转换。
use goethite_riscv64::addr::{PhysicalAddr, VirtualAddr};

use crate::mm::{alloc::FRAME_ALLOCATOR, buddy_system::order_size};

/// 一段由伙伴系统按阶分配、析构时自动归还的物理内存块。
///
/// `start` 是伙伴系统返回的**高半区虚拟地址**，可直接作为指针使用。
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

    /// 返回该页块的起始物理地址（把伙伴系统给出的高半区虚拟地址转回物理地址）。
    pub fn as_phys_addr(&self) -> PhysicalAddr {
        VirtualAddr::from(self.start).as_paddr()
    }

    /// 返回该物理块的字节大小（`1 << order << PAGE_SIZE_LOG_2`）。
    pub fn byte_size(&self) -> usize {
        order_size(self.order)
    }

    /// 返回该页块在伙伴系统中的阶数。
    pub fn order(&self) -> usize {
        self.order
    }

    /// 返回该页块的起始高半区虚拟地址。
    pub fn start(&self) -> usize {
        self.start
    }

    pub fn new(order: usize) -> Option<Self> {
        let start = FRAME_ALLOCATOR.lock().alloc(order)?;
        Some(Self { start, order })
    }
}
