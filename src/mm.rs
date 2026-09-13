//! 内存管理子系统。
//!
//! - [`alloc`]：全局堆分配器（实现 [`alloc::GlobalAlloc`]），
//!   底层是懒加载的 [`buddy_system::BuddyAllocator`] 与 [`slub::Caches`]。
//! - [`buddy_system`]：按 2 的幂次阶维护物理页帧空闲链表的伙伴系统。
//! - [`slub`]：SLUB 风格的对象缓存，作为内核堆（`#[global_allocator]`）。
//! - [`page_table`]：Sv39 页表项（[`page_table::PageTableEntry`]）与页表。
//! - [`address_space`]：由一个根页表 + 多个子页表组成的内存地址空间。
//! - [`object`]：堆上分配、自动释放的内存对象（[`object::PageObject`]）。

pub mod address_space;
pub mod alloc;
pub mod buddy_system;
pub mod object;
pub mod page_table;
pub mod region;
pub mod slub;
