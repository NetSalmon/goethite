#![no_std]

//! MINIX V1 文件系统驱动（`#![no_std]`，依赖 `alloc`）。
//!
//! 见 [`minix`] 模块文档。

extern crate alloc;

pub mod minix;
