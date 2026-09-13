//! SLUB 风格的对象分配器（全局堆）。
//!
//! 按对象大小分为多档 [`Cache`]；每档从伙伴系统取页并切成等大对象。
//! 大对象（超过 `SLUB_MAX_ORDER`）直接走伙伴系统。
use core::{alloc::Layout, cmp::max, fmt, marker::PhantomData, ptr::null_mut};

use goethite_config::*;
use goethite_intrusive_list::IntrusiveList;
use goethite_sync::{PerCpu, SpinLock};

use crate::mm::alloc::FRAME_ALLOCATOR;

/// 按 hart 分区的快速缓存：每个 hart 自己的一档 [`SlowCaches`]，避免锁竞争。
#[derive(Debug)]
pub struct CpuCache(pub PerCpu<SlowCaches, MAX_CPU>);

/// 一整套慢速对象缓存（按对象大小分档的 `[Cache; CACHES_MAX]`）。
#[derive(Debug)]
pub struct SlowCaches(pub(crate) [Cache; CACHES_MAX]);

/// 全局堆：包含 per-CPU 快速缓存与一把自旋锁保护的慢速缓存。
pub struct Caches {
    cpu_caches: CpuCache,
    slow_caches: SpinLock<SlowCaches>,
}

impl Caches {
    /// 构造：每个 hart 一份 `SlowCaches`，慢速缓存整体加锁。
    pub fn new() -> Self {
        Self {
            cpu_caches: CpuCache(PerCpu::new(core::array::from_fn(|_| SlowCaches::new()))),
            slow_caches: SpinLock::new(SlowCaches::new()),
        }
    }

    /// 分配 `layout` 大小的内存；先尝试当前 hart 的快速缓存，失败再走慢速缓存。
    pub fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = self.cpu_caches.0.current().alloc(layout);

        if !ptr.is_null() {
            return ptr;
        }

        self.slow_caches.lock().alloc(layout)
    }

    /// 释放 `layout` 大小的内存；先尝试归还到当前 hart 的快速缓存，失败再走慢速缓存。
    pub fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if !self.cpu_caches.0.current().dealloc(ptr, layout) {
            self.slow_caches.lock().dealloc(ptr, layout);
        }
    }
}

// SAFETY: `slow_caches` 由自旋锁串行化对 `SlowCaches` 的访问，
// `cpu_caches` 通过 `PerCpu` 让各 hart 只访问自己的槽位，互不相交，
// 因此 `Caches` 可安全地在执行流之间共享或转移。
unsafe impl Send for Caches {}
unsafe impl Sync for Caches {}

impl SlowCaches {
    pub fn new() -> Self {
        Self(core::array::from_fn(|i| {
            Cache::new(2usize.pow((i + SLUB_MIN_ORDER) as u32))
        }))
    }
}

impl SlowCaches {
    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let order = layout_order(&layout);

        if order >= SLUB_MAX_ORDER {
            return FRAME_ALLOCATOR
                .lock()
                .alloc_frame(1 << order)
                .map(|addr| addr as *mut _)
                .unwrap_or(null_mut());
        }

        let index = order - SLUB_MIN_ORDER;
        self.0[index]
            .alloc()
            .map(|p| p as *mut u8)
            .unwrap_or(null_mut())
    }

    pub fn dealloc(&mut self, ptr: *mut u8, layout: Layout) -> bool {
        let order = layout_order(&layout);

        if order >= SLUB_MAX_ORDER {
            FRAME_ALLOCATOR
                .lock()
                .dealloc_frame(ptr as usize, 1 << order);
            true
        } else {
            let index = order - SLUB_MIN_ORDER;

            self.0[index].dealloc(ptr as usize)
        }
    }
}

#[inline]
fn layout_order(layout: &Layout) -> usize {
    let adjusted_size = max(layout.size(), layout.align());
    let size = max(adjusted_size, 1 << SLUB_MIN_ORDER);
    size.next_power_of_two().trailing_zeros() as usize
}

/// 一段从伙伴系统取回、被切成等大对象的物理页（SLUB 的 “slub page”）。
#[derive(Debug)]
pub struct SlubPage {
    page_size: usize,
    next: *mut SlubPage,
    free_list: IntrusiveList,
    inuse: usize,
    objects: usize,
    page_start: *mut u8,
}

/// 按对象大小分档的缓存：维护部分空闲与全满两组 slub page 链表。
#[derive(Clone, Copy, Debug)]
pub struct Cache {
    objects_size: usize,
    partial_slubs: SlubPageList,
    full_slubs: SlubPageList,
}

/// 指向 [`SlubPage`] 的单向链表（以 `head` 裸指针表达）。
#[derive(Clone, Copy)]
pub struct SlubPageList {
    head: *mut SlubPage,
}

impl SlubPageList {
    /// 构造一个空链表。
    pub const fn new() -> Self {
        Self { head: null_mut() }
    }

    /// 链表是否为空。
    pub fn is_empty(&self) -> bool {
        self.head.is_null()
    }

    /// 头插法压入一个节点。
    pub fn push(&mut self, node: *mut SlubPage) {
        unsafe {
            (*node).next = self.head;
        }
        self.head = node;
    }

    /// 弹出头节点。
    pub fn pop(&mut self) -> Option<*mut SlubPage> {
        if self.is_empty() {
            None
        } else {
            let node = self.head;
            self.head = unsafe { (*node).next };
            Some(node)
        }
    }

    /// 移除指定节点，返回是否成功。
    pub fn remove(&mut self, item: *mut SlubPage) -> bool {
        if self.head.is_null() {
            return false;
        }

        if self.head == item {
            self.head = unsafe { (*self.head).next };
            return true;
        }

        let mut current = self.head;
        while !current.is_null() {
            let next = unsafe { (*current).next };
            if next == item {
                unsafe {
                    (*current).next = (*next).next;
                }
                return true;
            }
            current = unsafe { (*current).next };
        }

        false
    }

    /// 只读迭代器。
    pub fn iter(&self) -> SlubPageListIter<'_> {
        SlubPageListIter {
            current: self.head,
            _marker: PhantomData,
        }
    }

    /// 可变迭代器。
    pub fn iter_mut(&mut self) -> SlubPageListIterMut<'_> {
        SlubPageListIterMut {
            current: self.head,
            _marker: PhantomData,
        }
    }
}

impl fmt::Debug for SlubPageList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.iter().map(|p| p as *const _ as usize))
            .finish()
    }
}

/// 只读迭代器，遍历链表中的 [`SlubPage`]。
pub struct SlubPageListIter<'a> {
    current: *mut SlubPage,
    _marker: PhantomData<&'a SlubPageList>,
}

impl<'a> Iterator for SlubPageListIter<'a> {
    type Item = &'a SlubPage;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            None
        } else {
            let node = unsafe { &*self.current };
            self.current = node.next;
            Some(node)
        }
    }
}

/// 可变迭代器，遍历链表中的 [`SlubPage`]（可变引用）。
pub struct SlubPageListIterMut<'a> {
    current: *mut SlubPage,
    _marker: PhantomData<&'a mut SlubPageList>,
}

impl<'a> Iterator for SlubPageListIterMut<'a> {
    type Item = &'a mut SlubPage;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            None
        } else {
            let node = unsafe { &mut *self.current };
            self.current = node.next;
            Some(node)
        }
    }
}

impl SlubPage {
    /// 是否已满（所有对象都被占用）。
    pub fn is_full(&self) -> bool {
        self.inuse == self.objects
    }

    /// 是否为空（没有任何对象被占用）。
    pub fn is_empty(&self) -> bool {
        self.inuse == 0
    }

    /// 从 [`FRAME_ALLOCATOR`] 取一页并
    /// 初始化成对象大小为 `object_size` 的空 slub page；失败返回 `None`。
    pub fn new<'a>(object_size: usize) -> Option<&'a mut SlubPage> {
        let header_size = size_of::<SlubPage>();

        let start_offset = align(header_size, object_size);

        let min_page_size = start_offset + object_size;
        let mut page_size = PAGE_SIZE;
        while page_size < min_page_size {
            page_size *= 2;
        }

        let desired_objects = page_size / object_size;
        if desired_objects < SLUB_MIN_OBJECTS && page_size < SLUB_MAX_PAGE_SIZE {
            let new_page_size = page_size * 2;
            if new_page_size / object_size >= 2 {
                page_size = new_page_size;
            }
        }

        let page = FRAME_ALLOCATOR.lock().alloc_frame(page_size)?;

        let page_ptr = page as *mut SlubPage;

        unsafe {
            page_ptr.write_volatile(SlubPage {
                page_size,
                inuse: 0,
                next: null_mut(),
                free_list: IntrusiveList::new(),
                objects: 0,
                page_start: page as *mut u8,
            });
        }

        let ptr = unsafe { &mut *page_ptr };

        let mut count = 0;
        let mut pos = page + start_offset;
        while pos + object_size <= page + page_size {
            ptr.free_list.push(pos as *mut usize);
            pos += object_size;
            count += 1;
        }

        ptr.objects = count;
        Some(ptr)
    }

    /// 从本页的空闲链表取出一个对象（返回其地址），无空闲时返回 `None`。
    pub fn alloc_obj(&mut self) -> Option<usize> {
        if self.free_list.is_empty() {
            None
        } else {
            self.free_list.pop().map(|p| {
                self.inuse += 1;
                p as usize
            })
        }
    }

    /// 把地址为 `ptr` 的对象归还到本页的空闲链表。
    pub fn dealloc_obj(&mut self, ptr: usize) {
        self.free_list.push(ptr as *mut usize);
        self.inuse -= 1;
    }
}

impl Cache {
    /// 构造一个对象大小为 `objects_size` 的缓存（空链表）。
    pub fn new(objects_size: usize) -> Self {
        Self {
            objects_size,
            partial_slubs: SlubPageList::new(),
            full_slubs: SlubPageList::new(),
        }
    }

    /// 是否已满（没有部分空闲的 slub page 可供再分配）。
    pub fn is_full(&self) -> bool {
        self.partial_slubs.is_empty()
    }

    /// 分配一个对象：优先从部分空闲的 slub page 取，否则新建一页。
    pub fn alloc(&mut self) -> Option<usize> {
        if !self.partial_slubs.is_empty() {
            let page = self.partial_slubs.iter_mut().next()?;

            let ptr = page.alloc_obj()?;

            if page.is_full() {
                self.full_slubs.push(self.partial_slubs.pop()?);
            }

            return Some(ptr);
        }

        let page = SlubPage::new(self.objects_size)?;

        let ptr = page.alloc_obj()?;

        self.partial_slubs.push(page);

        Some(ptr)
    }

    /// 把对象 `ptr` 归还到所属的 slub page；对象所在页变空时回收该页。
    /// 返回是否成功归还（未找到所属页则返回 `false`）。
    pub fn dealloc(&mut self, ptr: usize) -> bool {
        let mut found_page: Option<*mut SlubPage> = None;
        for i in self.full_slubs.iter_mut() {
            let page_start = i.page_start as usize;
            if ptr >= page_start && ptr < page_start + i.page_size {
                i.dealloc_obj(ptr);

                found_page = Some(i as *mut SlubPage);
                break;
            }
        }

        if let Some(page_ptr) = found_page {
            self.full_slubs.remove(page_ptr);
            self.partial_slubs.push(page_ptr);

            return true;
        }

        let mut found_page = None;
        for i in self.partial_slubs.iter_mut() {
            let page_start = i.page_start as usize;
            if ptr >= page_start && ptr < page_start + i.page_size {
                i.dealloc_obj(ptr);

                if i.is_empty() {
                    found_page = Some(i as *mut SlubPage);
                    break;
                }

                return true;
            }
        }

        if let Some(page_ptr) = found_page {
            self.partial_slubs.remove(page_ptr);
            let page_size = unsafe { (*page_ptr).page_size };
            FRAME_ALLOCATOR
                .lock()
                .dealloc_frame(page_ptr as usize, page_size);

            return true;
        }

        false
    }
}
