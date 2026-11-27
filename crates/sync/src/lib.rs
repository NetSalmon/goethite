#![no_std]

//! goethite 内核的同步原语。
//!
//! 为 `#![no_std]` 的 RISC-V 内核提供不依赖操作系统支持的同步工具：
//!
//! - [`LazyLock`]：延迟到首次访问才执行初始化闭包、之后缓存结果的懒加载静态值；
//! - [`OnceLock`]：只初始化一次的值容器；
//! - [`PerCpu`]：按 hart 复制一份的数据，借由 `tp` 寄存器索引当前 hart，避免
//!   不同 hart 竞争同一把锁；
//! - [`RwLock`]：写优先的读写自旋锁；
//! - [`SpinLock`]：关中断的自旋锁。
//!
//! 所有原语都基于 `UnsafeCell` 与/或原子操作实现，可用于内核的 `const` 静态
//! 初始化。它们以 `unsafe impl Sync` / `Send` 提供并发安全，其正确性依赖调用方
//! 遵守各模块文档中的借用与访问约定（例如 [`PerCpu`] 要求同一 hart 不会同时
//! 持有同一槽位的两个可变引用，[`RwLock`] / [`SpinLock`] 要求在锁内不依赖中断）。
//!
//! # 安全性
//!
//! 这些类型对外暴露的并发安全建立在「调用方遵循模块约定」的前提之上，使用前
//! 请阅读对应模块文档的 `# 安全性` 小节。

pub mod lazy;
pub mod once;
pub mod per_cpu;
#[cfg(feature = "qemu-tests")]
pub mod qemu_tests;
pub mod rwlock;
pub mod spin;

pub use lazy::LazyLock;
pub use once::OnceLock;
pub use per_cpu::PerCpu;
pub use rwlock::RwLock;
pub use spin::SpinLock;
