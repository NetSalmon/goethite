#![no_std]
#![recursion_limit = "256"]

//! ELF64 解析。
//!
//! 用于加载用户程序：提供 ELF 64 位文件头 [`ElfHeader`] 与程序头
//! `ProgramHeader` 的最小结构集合，以及相关的 ELF 常量类型（如
//! [`ElfMachine`]、[`ElfType`]）。
//!
//! 本 crate 为 `no_std`，不依赖标准库。

mod types;

pub use types::*;
