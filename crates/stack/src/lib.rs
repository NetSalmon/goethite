//! goethite-stack：为 RISC-V 内核构建/解析进程栈映像（argv / env / auxv）的 crate。
//!
//! 该 crate 是 `#![no_std]` 的，提供两种工作模式：
//!
//! ## 无分配 / 零拷贝（`StackRef`）
//!
//! [`StackRef`] 直接借用一段连续的栈映像字节，在不拷贝任何字符串数据的前提下暴露
//! `argv` / `env` / `auxv` 的迭代视图。字符串数据引用缓冲内字符串池的字节，从而做到
//! 零拷贝反序列化。序列化入口 [`write`] / [`required_len`] 也完全无分配，由调用方提供
//! 输出缓冲区。
//!
//! ## 分配版（`Stack`，需 `alloc` feature）
//!
//! [`Stack`] 把 argv 还原为 [`alloc::vec::Vec<String>`]，把 env 键值对还原为
//! [`alloc::collections::BTreeMap<String, String>`]，把 auxv 编号-内容还原为
//! [`alloc::collections::BTreeMap<u64, u64>`]。提供 [`Stack::parse`]（分配反序列化）与
//! [`Stack::to_bytes`]（分配序列化，返回 `Vec<u8>`）。
//!
//! # 布局
//!
//! 参见 [`raw`] 模块的布局说明。格式固定、小端序，头部的各偏移字段指明各表位置。
//! 辅助向量条目采用标准的编号（如 [`AT_PHDR`]）与数值（如虚拟地址）结构。

#![no_std]
#![warn(missing_debug_implementations)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod error;
pub mod raw;
mod ser;
mod view;

#[cfg(feature = "alloc")]
mod owned;

pub use error::Error;
#[cfg(feature = "alloc")]
pub use owned::Stack;
pub use raw::{
    AT_BASE, AT_BASE_PLATFORM, AT_CLKTCK, AT_EGID, AT_ENTRY, AT_EUID, AT_EXECFD, AT_EXECFN,
    AT_FLAGS, AT_GID, AT_HWCAP, AT_HWCAP2, AT_IGNORE, AT_NOTELF, AT_NULL, AT_PAGESZ, AT_PHDR,
    AT_PHENT, AT_PHNUM, AT_PLATFORM, AT_RANDOM, AT_SECURE, AT_SYSINFO, AT_SYSINFO_EHDR, AT_UID,
    AUX_ENTRY_SIZE, Args, AuxEntry, AuxValues, EnvVars, GOETHITE_STACK_MAGIC, HEADER_SIZE, Header,
    PAIR_SIZE, Pair, STRING_ID_SIZE, STRING_REF_SIZE, StringId, StringPool, StringRef,
    StringTable,
};
pub use ser::{Input, required_len, write};
pub use view::{ArgsIter, AuxvIter, EnvIter, StackRef, TryArgsIter, TryAuxvIter, TryEnvIter};
