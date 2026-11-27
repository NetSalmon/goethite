//! goethite 用户态程序默认入口库：直接重新导出 Rust 原生运行时 [`rs`]（`goethite-librs`）。
//!
//! 另一套面向 C / POSIX 程序的兼容库位于 `userland/libc`（crate `goethite-libc`，产出 `libc.a`）。

#![no_std]

pub use rs::*;
