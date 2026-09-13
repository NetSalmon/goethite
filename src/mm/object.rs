//! 从伙伴系统分配、用类型状态（typestate）保护的内存对象。
//!
//! [`PageObject<T, S>`] 从 [`FRAME_ALLOCATOR`] 分配一块足以容纳 `T` 的物理页，
//! 并把「是否已初始化」编码在类型参数 `S` 上：分配出来的是
//! [`PageObject<T, Uninit>`]，必须调用 [`init`](PageObject::init) 转成
//! [`PageObject<T, Init>`] 之后才能安全访问。
//!
//! # 安全性
//!
//! - 初始化状态在 **类型层面** 建模，因而未初始化的对象没有机会被读取：
//!   [`Deref`]/[`DerefMut`]/[`get`](PageObject::get) 只为 [`Init`] 状态实现，
//!   `Uninit` 状态在编译期就无法调用它们，彻底避免 `assume_init_*` 读到未初始化
//!   内存造成未定义行为——不再需要运行时 `bool` 与 panic 判空。
//! - [`init`](PageObject::init) 按值消费 `Uninit` 对象并返回 `Init` 对象，保证
//!   对象在「写入 `T` 之前」不可能被读取；重复写入（[`set`](PageObject::set)）仍需 `&mut self`。
//! - 析构行为由状态 [`State::DROP_T`] 驱动：`Uninit` 只归还页，`Init` 先 drop
//!   `T` 再归还页；该常量在编译期折叠，运行期无额外分支。
//! - 分配时记录页块在伙伴系统中的阶数 `page_order`，析构时按阶归还，避免按字节
//!   大小重复换算与阶数截断不一致。

use core::{
    alloc::Layout,
    fmt::Debug,
    marker::PhantomData,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

use goethite_config::{BUDDY_MAX_ORDER, PAGE_SIZE, align};

use crate::mm::{
    alloc::FRAME_ALLOCATOR,
    buddy_system::{order_size, size_to_order},
};

/// 从伙伴系统分配 [`PageObject`] 时可能出现的错误。
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
/// 借助该关联常量，[`PageObject`] 只需一个通用 [`Drop`] 实现即可区分两种状态，
/// 无需为每个状态写一个（Rust 不允许 `Drop` 按具体类型特化）。
pub trait State {
    /// 该状态下析构时是否需要 drop 内嵌的 `T`。
    const DROP_T: bool;
}

/// 类型状态标记：尚未初始化。
///
/// [`PageObject<T, Uninit>`] 只允许 [`init`](PageObject::init) 或直接析构归还页块，
/// 无法读取 `T`。
pub struct Uninit;

impl State for Uninit {
    const DROP_T: bool = false;
}

/// 类型状态标记：已初始化。
///
/// [`PageObject<T, Init>`] 允许经 [`get`](PageObject::get) / [`Deref`] 访问 `T`。
pub struct Init;

impl State for Init {
    const DROP_T: bool = true;
}

/// 从伙伴系统分配、用类型状态保护的内存对象。
///
/// `S` 是初始化状态：[`Uninit`] 表示尚未初始化，[`Init`] 表示已初始化。
pub struct PageObject<T, S: State> {
    ptr: NonNull<MaybeUninit<T>>,
    /// 伙伴系统分配该页块时对应的阶数；页块字节数为 `PAGE_SIZE << page_order`。
    page_order: usize,
    /// 把状态标记 `S` 挂在类型上（零占用）。
    state: PhantomData<S>,
}

impl<T: Debug> Debug for PageObject<T, Init> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Safety: type state已保证数据得到初始化
        unsafe { self.ptr.as_ref().assume_init_ref() }.fmt(f)
    }
}

impl<T, S: State> PageObject<T, S> {
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

impl<T> PageObject<T, Uninit> {
    /// 从伙伴系统分配一块足以容纳 `T` 的页，返回一个 **未初始化** 的
    /// [`PageObject<T, Uninit>`]。
    ///
    /// # Errors
    ///
    /// - [`AllocError::UnsupportedLayout`]：`T` 的对齐超过页大小，或所需大小超过
    ///   伙伴系统单个最大块。
    /// - [`AllocError::OutOfMemory`]：伙伴系统没有足够大的空闲块。
    pub fn alloc() -> Result<Self, AllocError> {
        let layout = Layout::new::<T>();

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
        let ptr = NonNull::new(addr as *mut MaybeUninit<T>).ok_or(AllocError::OutOfMemory)?;

        Ok(PageObject {
            ptr,
            page_order: order,
            state: PhantomData,
        })
    }

    /// 把 `value` 写入页起始处，返回一个 **已初始化** 的 [`PageObject<T, Init>`]。
    ///
    /// 按值消费 `self`，因此在「写入 `T` 之前」对象不可能被读取；一旦返回，`T`
    /// 保证已写入，后续读取都不再有未初始化内存的问题。
    pub fn init(mut self, value: T) -> PageObject<T, Init> {
        unsafe { self.ptr.as_mut().write(value) };
        let initialized = PageObject {
            ptr: self.ptr,
            page_order: self.page_order,
            state: PhantomData,
        };
        // `initialized` 已接手掌管该页块，丢弃旧的 `Uninit` 外壳，避免其析构重复归还。
        core::mem::forget(self);
        initialized
    }
}

impl<T> PageObject<T, Init> {
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

impl<T> Deref for PageObject<T, Init> {
    type Target = T;

    /// 以不可变引用解引用；`Init` 状态保证 `T` 已写入。
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> DerefMut for PageObject<T, Init> {
    /// 以可变引用解引用；`Init` 状态保证 `T` 已写入。
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

impl<T, S: State> Drop for PageObject<T, S> {
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
unsafe impl<T: Send, S: State + Send> Send for PageObject<T, S> {}
unsafe impl<T: Send + Sync, S: State + Sync> Sync for PageObject<T, S> {}
