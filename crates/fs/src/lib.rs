#![no_std]

//! 文件系统通用层：与具体文件系统解耦的路径、类型、错误、设备抽象与 VFS。
//!
//! - [`path`]：仿 `std::path` 的 [`Path`] / [`PathBuf`]；
//! - [`types`]：POSIX `st_mode` 的 [`FileType`] / [`Mode`]；
//! - [`error`]：统一错误 [`FsError`] / [`FsResult`]；
//! - [`device`]：块设备抽象 [`BlockDevice`] 与 I/O 错误 [`IoError`]；
//! - [`vfs`]：VFS 接口（[`FileSystem`] / [`FileOps`]）与内存对象
//!   （[`Stat`] / [`OpenFlags`] / [`File`] / [`SuperBlock`] / [`Vfs`]）。
//!
//! 本 crate 不依赖标准库，只依赖 `alloc`。

extern crate alloc;

pub mod device;
pub mod error;
pub mod path;
pub mod types;
pub mod vfs;

pub use device::{BlockDevice, IoError, IoResult, SECTOR_SIZE, Whence};
pub use error::{FsError, FsResult};
pub use path::{Path, PathBuf};
pub use types::{FileType, Mode, S_IFMT};
pub use vfs::{DirEntry, FileSystem, OpenFlags, Stat};
