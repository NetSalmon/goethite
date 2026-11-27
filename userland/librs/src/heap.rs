//! 用户态堆分配器封装：基于第三方 `linked_list_allocator` crate + 静态 `.bss` 堆区。
//!
//! 堆缓冲区位于 `.bss` 段（不占用磁盘 ELF 空间，加载时由内核自动清零映射），
//! 通过 [`goethite_sync::SpinLock`] 保护 [`linked_list_allocator::Heap`] 实现 `GlobalAlloc`。

use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    ptr::{self, NonNull},
};

use crate::sync::UserSpinLock;
pub use linked_list_allocator::Heap;

/// 默认用户态堆大小（256 KiB，位于 `.bss`）。
pub const DEFAULT_HEAP_SIZE: usize = 256 * 1024;

#[repr(C, align(16))]
struct AlignedHeap<const N: usize>([u8; N]);

/// 带用户态自旋锁与静态 `.bss` 缓冲区的全局堆分配器（底层为 `linked_list_allocator::Heap`）。
pub struct LockedHeap<const N: usize = DEFAULT_HEAP_SIZE> {
    inner: UserSpinLock<Heap>,
    storage: UnsafeCell<AlignedHeap<N>>,
}

unsafe impl<const N: usize> Sync for LockedHeap<N> {}

impl<const N: usize> LockedHeap<N> {
    /// 构造一个编译期零初始化的静态堆分配器（放入 `.bss`）。
    pub const fn new() -> Self {
        Self {
            inner: UserSpinLock::new(Heap::empty()),
            storage: UnsafeCell::new(AlignedHeap([0; N])),
        }
    }

    fn ensure_init(&self, heap: &mut Heap) {
        if heap.size() == 0 {
            let base = unsafe { (*self.storage.get()).0.as_mut_ptr() };
            unsafe { heap.init(base, N) };
        }
    }

    /// 返回当前堆的总容量（字节）。
    pub fn size(&self) -> usize {
        let mut guard = self.inner.lock();
        self.ensure_init(&mut guard);
        guard.size()
    }

    /// 返回当前已使用字节数。
    pub fn used(&self) -> usize {
        let mut guard = self.inner.lock();
        self.ensure_init(&mut guard);
        guard.used()
    }

    /// 返回当前剩余空闲字节数。
    pub fn free(&self) -> usize {
        let mut guard = self.inner.lock();
        self.ensure_init(&mut guard);
        guard.free()
    }
}

unsafe impl<const N: usize> GlobalAlloc for LockedHeap<N> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut guard = self.inner.lock();
        self.ensure_init(&mut guard);
        guard
            .allocate_first_fit(layout)
            .map(NonNull::as_ptr)
            .unwrap_or(ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(nn) = NonNull::new(ptr) {
            let mut guard = self.inner.lock();
            self.ensure_init(&mut guard);
            unsafe { guard.deallocate(nn, layout) };
        }
    }
}
