#![no_std]

//! 编译期常量汇总。
//!
//! 聚合 CPU（`cpu`）、设备（`device`）、文件系统（`fs`）、内存布局（`memory`）、
//! 链接器符号（`symbols`）、任务（`task`）与版本/架构（`uname`）
//! 常量，并在本模块统一 `pub use` 导出。
pub(crate) mod cpu;
pub(crate) mod device;
pub(crate) mod fs;
pub(crate) mod memory;
pub(crate) mod symbols;
pub(crate) mod task;
pub(crate) mod uname;

pub use cpu::*;
pub use device::*;
pub use fs::*;
pub use memory::*;
pub use symbols::*;
pub use task::*;
pub use uname::*;
