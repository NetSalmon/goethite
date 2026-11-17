//! 从伙伴系统分配、用类型状态（typestate）保护的内存对象。
//!
//! [`Handle<T, S>`] 从 [`FRAME_ALLOCATOR`] 分配一块足以容纳 `T` 的页块，
//! 并把「是否已初始化」编码在类型参数 `S` 上：分配出来的是
//! [`Handle<T, Uninit>`]，必须调用 [`init`](Handle::init) 转成
//! [`Handle<T, Init>`] 之后才能安全访问。
//!
//! 分配得到的是伙伴系统给出的**高半区虚拟地址**（见
//! [`BuddyAllocator`](crate::mm::buddy_system::BuddyAllocator) 的地址语义），
//! 因此 [`get`](Handle::get) / `Deref` 解引用时无需再做转换；归还时把同一
//! 地址传回 [`FRAME_ALLOCATOR`] 即可。
//!
//! [`RefHandle<T, S>`] 在 [`Handle`] 的基础上增加引用计数：可以从
//! [`RefHandle<T, Init>`] 经 [`Clone`] 派生出多个共享同一页块的句柄，只有最后一个
//! 句柄析构时才 drop `T` 并归还页块。由于多个句柄共享同一对象，`RefHandle` 不实现
//! `DerefMut`，可变访问须经 [`get_mut`](RefHandle::get_mut) 在引用唯一时取得。
//!
//! # 安全性
//!
//! - 初始化状态在 **类型层面** 建模，因而未初始化的对象没有机会被读取：
//!   [`Deref`]/[`DerefMut`]/[`get`](Handle::get) 只为 [`Init`] 状态实现，
//!   `Uninit` 状态在编译期就无法调用它们，彻底避免 `assume_init_*` 读到未初始化
//!   内存造成未定义行为——不再需要运行时 `bool` 与 panic 判空。
//! - [`init`](Handle::init) 按值消费 `Uninit` 对象并返回 `Init` 对象，保证
//!   对象在「写入 `T` 之前」不可能被读取；重复写入（[`set`](Handle::set)）仍需 `&mut self`。
//! - 析构行为由状态 [`State::DROP_T`] 驱动：`Uninit` 只归还页，`Init` 先 drop
//!   `T` 再归还页；该常量在编译期折叠，运行期无额外分支。
//! - 分配时记录页块在伙伴系统中的阶数 `page_order`，析构时按阶归还，避免按字节
//!   大小重复换算与阶数截断不一致。
//! - `RefHandle` 的引用计数存放在同一页块内、`T` 之后，使用原子操作增减：
//!   克隆时 `Relaxed` 递增，析构时 `Release` 递减，最后一个句柄再以 `Acquire`
//!   栅栏取得其他线程的写入。只有计数归零才 drop `T` 并归还页块。

use core::{
    alloc::Layout,
    fmt::Debug,
    marker::PhantomData,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering, fence},
};

use goethite_config::{BUDDY_MAX_ORDER, PAGE_SIZE, align};

use crate::mm::{
    alloc::FRAME_ALLOCATOR,
    buddy_system::{order_size, size_to_order},
};

/// 从伙伴系统分配 [`Handle`] / [`RefHandle`] 时可能出现的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AllocError {
    /// `T` 的布局（大小或对齐）超出支持范围。
    #[error("unsupported layout")]
    UnsupportedLayout,
    /// 伙伴系统中没有足够大的空闲页块。
    #[error("out of memory")]
    OutOfMemory,
}

/// 类型状态协议：定义某种初始化状态在析构时是否要 drop 其内嵌对象。
///
/// 借助该关联常量，[`Handle`] 只需一个通用 [`Drop`] 实现即可区分两种状态，
/// 无需为每个状态写一个（Rust 不允许 `Drop` 按具体类型特化）。
pub trait State {
    /// 该状态下析构时是否需要 drop 内嵌的 `T`。
    const DROP_T: bool;
}

/// 类型状态标记：尚未初始化。
///
/// [`Handle<T, Uninit>`] 只允许 [`init`](Handle::init) 或直接析构归还页块，
/// 无法读取 `T`。
pub struct Uninit;

impl State for Uninit {
    const DROP_T: bool = false;
}

/// 类型状态标记：已初始化。
///
/// [`Handle<T, Init>`] 允许经 [`get`](Handle::get) / [`Deref`] 访问 `T`。
pub struct Init;

impl State for Init {
    const DROP_T: bool = true;
}

/// 从伙伴系统分配、用类型状态保护的内存对象。
///
/// `S` 是初始化状态：[`Uninit`] 表示尚未初始化，[`Init`] 表示已初始化。
pub struct Handle<T, S: State> {
    ptr: NonNull<MaybeUninit<T>>,
    /// 伙伴系统分配该页块时对应的阶数；页块字节数为 `PAGE_SIZE << page_order`。
    page_order: usize,
    /// 把状态标记 `S` 挂在类型上（零占用）。
    state: PhantomData<S>,
}

impl<T: Debug> Debug for Handle<T, Init> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Safety: type state已保证数据得到初始化
        unsafe { self.ptr.as_ref().assume_init_ref() }.fmt(f)
    }
}

impl<T, S: State> Handle<T, S> {
    /// 归还伙伴系统页块（不 drop `T`）。
    ///
    /// # Safety
    ///
    /// 仅用于归还物理页；调用方须保证 `ptr`/`page_order` 与分配时一致。
    fn dealloc(&mut self) {
        FRAME_ALLOCATOR
            .lock()
            .dealloc(self.ptr.as_ptr() as usize, self.page_order);
    }

    /// 获取地址
    ///
    /// # Safety
    ///
    /// 仅用于获取地址；调用方需保证不读写指针。
    pub unsafe fn as_ptr(&self) -> *const () {
        self.ptr.as_ptr() as *const ()
    }
}

/// 从伙伴系统分配一块满足 `layout` 的页块。
///
/// 返回页块起始地址与对应阶数；请求至少占一页，并按页向上取整。
fn alloc_pages(layout: Layout) -> Result<(NonNull<u8>, usize), AllocError> {
    // 对齐超出一页无法由伙伴系统保证，拒绝。
    if layout.align() > PAGE_SIZE {
        return Err(AllocError::UnsupportedLayout);
    }

    // 至少分配一页，并把请求大小按页向上对齐。
    let byte_size = align(layout.size().max(1), PAGE_SIZE);

    // 按字节大小换算为对应阶数，并校验单块上限，避免 `size_to_order` 截断后
    // 返回过小的块。
    let order = size_to_order(byte_size, BUDDY_MAX_ORDER);
    if order_size(order) < byte_size {
        return Err(AllocError::UnsupportedLayout);
    }

    let addr = FRAME_ALLOCATOR
        .lock()
        .alloc(order)
        .ok_or(AllocError::OutOfMemory)?;
    let ptr = NonNull::new(addr as *mut u8).ok_or(AllocError::OutOfMemory)?;
    Ok((ptr, order))
}

impl<T> Handle<T, Uninit> {
    /// 从伙伴系统分配一块足以容纳 `T` 的页，返回一个 **未初始化** 的
    /// [`Handle<T, Uninit>`]。
    ///
    /// # Errors
    ///
    /// - [`AllocError::UnsupportedLayout`]：`T` 的对齐超过页大小，或所需大小超过
    ///   伙伴系统单个最大块。
    /// - [`AllocError::OutOfMemory`]：伙伴系统没有足够大的空闲块。
    pub fn alloc() -> Result<Self, AllocError> {
        let (ptr, page_order) = alloc_pages(Layout::new::<T>())?;

        Ok(Handle {
            ptr: ptr.cast(),
            page_order,
            state: PhantomData,
        })
    }

    /// 把 `value` 写入页起始处，返回一个 **已初始化** 的 [`Handle<T, Init>`]。
    ///
    /// 按值消费 `self`，因此在「写入 `T` 之前」对象不可能被读取；一旦返回，`T`
    /// 保证已写入，后续读取都不再有未初始化内存的问题。
    pub fn init(mut self, value: T) -> Handle<T, Init> {
        unsafe { self.ptr.as_mut().write(value) };
        let initialized = Handle {
            ptr: self.ptr,
            page_order: self.page_order,
            state: PhantomData,
        };
        // `initialized` 已接手掌管该页块，丢弃旧的 `Uninit` 外壳，避免其析构重复归还。
        core::mem::forget(self);
        initialized
    }
}

impl<T> Handle<T, Init> {
    /// 以不可变引用读取；`Init` 状态保证 `T` 已写入。
    #[inline]
    pub fn get(&self) -> &T {
        unsafe { self.ptr.as_ref().assume_init_ref() }
    }

    /// 以可变引用读取；`Init` 状态保证 `T` 已写入。
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        unsafe { self.ptr.as_mut().assume_init_mut() }
    }

    /// 替换为新的 `value`：先 drop 旧的 `T` 再写入，避免泄漏。
    pub fn set(&mut self, value: T) {
        unsafe { core::ptr::drop_in_place(self.ptr.cast::<T>().as_ptr()) };
        unsafe { self.ptr.as_mut().write(value) };
    }
}

impl<T> Deref for Handle<T, Init> {
    type Target = T;

    /// 以不可变引用解引用；`Init` 状态保证 `T` 已写入。
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> DerefMut for Handle<T, Init> {
    /// 以可变引用解引用；`Init` 状态保证 `T` 已写入。
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

impl<T, S: State> Drop for Handle<T, S> {
    /// 按状态决定是否先 drop `T`，再归还物理页。
    ///
    /// `S::DROP_T` 为编译期常量，落到各状态上时条件分支会被消除。
    fn drop(&mut self) {
        if S::DROP_T {
            unsafe { core::ptr::drop_in_place(self.ptr.cast::<T>().as_ptr()) };
        }
        self.dealloc();
    }
}

// SAFETY: `PageObject` 只在初始化后的 `Init` 状态下暴露 `&T`/`&mut T`，且写入
// （`init` / `set`）仍需独占访问，故 `T` 的写入与读取不可能并发。`Uninit` 状态
// 根本不含 `T`，其析构只是归还页块。跨线程共享（`Sync`）各方只经 `&self` 读取已
// 初始化部分。状态标记 `S` 是零大小的类型标签，不携带数据。
unsafe impl<T: Send, S: State + Send> Send for Handle<T, S> {}
unsafe impl<T: Send + Sync, S: State + Sync> Sync for Handle<T, S> {}

/// 从伙伴系统分配、带引用计数的内存对象。
///
/// 与 [`Handle`] 一样，`S` 是初始化状态：[`Uninit`] 表示尚未初始化，[`Init`]
/// 表示已初始化。区别在于：
///
/// - 页块内除 `T` 之外还存放一个 [`AtomicUsize`] 引用计数，分配时初始化为 1；
/// - [`RefHandle<T, Init>`] 实现 [`Clone`]，克隆只增加计数并共享同一 `T`；
/// - [`Uninit`] 状态不实现 [`Clone`]，避免多个未初始化句柄对同一对象重复
///   [`init`](RefHandle::init) 造成重复写入；
/// - 只有最后一个句柄析构时才 drop `T`（仅 [`Init`] 状态）并归还页块；
/// - 由于多个句柄可能同时共享 `T`，不实现 [`DerefMut`]，可变引用只能通过
///   [`get_mut`](RefHandle::get_mut) 在引用唯一时取得。
pub struct RefHandle<T, S: State> {
    /// `T` 的地址，同时也是伙伴系统页块的起始地址。
    ptr: NonNull<MaybeUninit<T>>,
    /// 位于 `T` 之后、同一页块内的引用计数。
    count: NonNull<AtomicUsize>,
    /// 伙伴系统分配该页块时对应的阶数；页块字节数为 `PAGE_SIZE << page_order`。
    page_order: usize,
    /// 把状态标记 `S` 挂在类型上（零占用）。
    state: PhantomData<S>,
}

impl<T: Debug> Debug for RefHandle<T, Init> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Safety: type state已保证数据得到初始化
        unsafe { self.ptr.as_ref().assume_init_ref() }.fmt(f)
    }
}

impl<T, S: State> RefHandle<T, S> {
    /// 引用计数；分配时与 `ptr` 指向同一页块并已写入，故始终有效。
    #[inline]
    fn ref_count(&self) -> &AtomicUsize {
        unsafe { self.count.as_ref() }
    }

    /// 归还伙伴系统页块（不 drop `T`）。
    fn dealloc(&mut self) {
        FRAME_ALLOCATOR
            .lock()
            .dealloc(self.ptr.as_ptr() as usize, self.page_order);
    }

    /// 获取 `T` 的地址（也是页块起始地址）。
    ///
    /// # Safety
    ///
    /// 仅用于获取地址；调用方需保证不读写指针。
    pub unsafe fn as_ptr(&self) -> *const () {
        self.ptr.as_ptr() as *const ()
    }
}

impl<T> RefHandle<T, Uninit> {
    /// 从伙伴系统分配一块足以容纳 `T` 与引用计数的页，返回一个 **未初始化** 的
    /// [`RefHandle<T, Uninit>`]。
    ///
    /// 引用计数初始化为 1，因此返回的句柄独占该页块。
    ///
    /// # Errors
    ///
    /// - [`AllocError::UnsupportedLayout`]：`T` 的对齐超过页大小，或所需大小超过
    ///   伙伴系统单个最大块。
    /// - [`AllocError::OutOfMemory`]：伙伴系统没有足够大的空闲块。
    pub fn alloc() -> Result<Self, AllocError> {
        // 把计数放在 `T` 之后，保证 `T` 仍从页块起始处开始，与 `Handle::as_ptr`
        // 的地址语义一致。
        let (layout, count_offset) = Layout::new::<T>()
            .extend(Layout::new::<AtomicUsize>())
            .map_err(|_| AllocError::UnsupportedLayout)?;
        let layout = layout.pad_to_align();

        let (base, page_order) = alloc_pages(layout)?;
        let ptr = base.cast::<MaybeUninit<T>>();
        // Safety: `count_offset` 由 `Layout::extend` 计算，落在已分配的页块内；
        // `base` 页对齐，满足 `AtomicUsize` 的对齐要求。
        let count = unsafe {
            let count = base.as_ptr().add(count_offset).cast::<AtomicUsize>();
            count.write(AtomicUsize::new(1));
            NonNull::new_unchecked(count)
        };

        Ok(RefHandle {
            ptr,
            count,
            page_order,
            state: PhantomData,
        })
    }

    /// 把 `value` 写入页起始处，返回一个 **已初始化** 的 [`RefHandle<T, Init>`]。
    ///
    /// 按值消费 `self`，因此「写入 `T` 之前」对象不可能被读取。
    pub fn init(mut self, value: T) -> RefHandle<T, Init> {
        unsafe { self.ptr.as_mut().write(value) };
        let initialized = RefHandle {
            ptr: self.ptr,
            count: self.count,
            page_order: self.page_order,
            state: PhantomData,
        };
        // `initialized` 已接手掌管该页块，丢弃旧的 `Uninit` 外壳，避免其析构重复归还。
        core::mem::forget(self);
        initialized
    }
}

impl<T> RefHandle<T, Init> {
    /// 当前强引用计数。
    #[inline]
    pub fn strong_count(&self) -> usize {
        self.ref_count().load(Ordering::Relaxed)
    }

    /// 引用计数为 1（没有其他共享句柄）时返回 `true`。
    ///
    /// 使用 `Acquire` 载入，与最后一个克隆析构时的 `Release` 递减配对。
    #[inline]
    pub fn is_unique(&self) -> bool {
        self.ref_count().load(Ordering::Acquire) == 1
    }

    /// 以不可变引用读取；`Init` 状态保证 `T` 已写入。
    #[inline]
    pub fn get(&self) -> &T {
        unsafe { self.ptr.as_ref().assume_init_ref() }
    }

    /// 仅在引用唯一时返回 `&mut T`，否则返回 `None`。
    ///
    /// 多个句柄共享同一 `T` 时给出可变引用会造成数据竞争，因此必须先确认没有
    /// 其他句柄。
    #[inline]
    pub fn get_mut(&mut self) -> Option<&mut T> {
        if self.is_unique() {
            Some(unsafe { self.ptr.as_mut().assume_init_mut() })
        } else {
            None
        }
    }
}

impl<T> Clone for RefHandle<T, Init> {
    /// 克隆只增加引用计数，共享同一页块与 `T`。
    fn clone(&self) -> Self {
        // Relaxed 足够：原句柄仍然存在，保证 `T` 在克隆期间不会被释放。
        let previous = self.ref_count().fetch_add(1, Ordering::Relaxed);
        // 计数溢出说明已有 `usize::MAX` 个句柄，远大于正常使用范围。
        assert!(
            previous != usize::MAX,
            "RefHandle: reference count overflow"
        );

        RefHandle {
            ptr: self.ptr,
            count: self.count,
            page_order: self.page_order,
            state: PhantomData,
        }
    }
}

impl<T> Deref for RefHandle<T, Init> {
    type Target = T;

    /// 以不可变引用解引用；`Init` 状态保证 `T` 已写入。
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T, S: State> Drop for RefHandle<T, S> {
    /// 递减引用计数；只有最后一个句柄才 drop `T` 并归还物理页。
    fn drop(&mut self) {
        // Release：让此前对该页块的所有访问对最后一个析构者可见。
        if self.ref_count().fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        // 最后一个引用：获取其他线程释放引用时对应的写入。
        fence(Ordering::Acquire);

        if S::DROP_T {
            unsafe { core::ptr::drop_in_place(self.ptr.cast::<T>().as_ptr()) };
        }
        self.dealloc();
    }
}

// SAFETY: 页块内的引用计数是原子量，可被多个执行流并发增减。克隆出的句柄可能在
// 其他线程析构并 drop `T`，而其他句柄仍可通过 `&self` 读取 `T`，因此 `T` 必须同时
// 满足 `Send + Sync`。`Init` 下只在引用唯一时暴露 `&mut T`，不会与其他读写并发。
// 状态标记 `S` 是零大小的类型标签，不携带数据。
unsafe impl<T: Send + Sync, S: State + Send> Send for RefHandle<T, S> {}
unsafe impl<T: Send + Sync, S: State + Sync> Sync for RefHandle<T, S> {}
