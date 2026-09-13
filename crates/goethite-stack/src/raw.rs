//! goethite-stack 的底层二进制布局类型与读写辅助。
//!
//! 该模块定义的是紧凑的、自包含的栈映像（process stack image）的磁盘/内存格式，
//! 供 [`crate::ser`]（序列化）与 [`crate::view`]（零拷贝反序列化）共用。
//!
//! # 布局
//!
//! 整个映像是一段连续字节，按如下顺序存放（小端序）：
//!
//! ```text
//! stack_top:
//!   Header                // magic + 各表偏移 + total_size
//!   StringTable[StringRef;string_count]   // 字符串引用表
//!   EnvVars[Pair;env_count]               // 环境变量表
//!   AuxValues[Pair;auxv_count]            // 辅助向量表
//!   Args[StringId;argc]                   // 命令行参数表
//!   StringPool[u8;pool_len]               // 字节池，存所有 UTF-8 字符串数据
//! sp:
//!   .quad stack_top
//! ```
//!
//! 除 `pool_offset` 之外，所有偏移都以字节为单位、相对于映像起始地址，并且 4 字节对齐。

use core::mem::size_of;

/// goethite-stack 映像的魔数：`b"kosSTACK"`。
pub const GOETHITE_STACK_MAGIC: [u8; 8] = [b'k', b'o', b's', b'S', b'T', b'A', b'C', b'K'];

/// 单个 [`StringRef`] 的字节大小（两个 `u32`）。
pub const STRING_REF_SIZE: usize = 8;
/// 单个 [`Pair`] 的字节大小（两个 `u32`）。
pub const PAIR_SIZE: usize = 8;
/// 单个 [`StringId`] 的字节大小（一个 `u32`）。
pub const STRING_ID_SIZE: usize = 4;
/// [`Header`] 的字节大小。
pub const HEADER_SIZE: usize = size_of::<Header>();

/// 映像头部：记录魔数、总大小以及各表的字节偏移。
///
/// 各偏移字段遵循 [`Header::empty`] 的约定：一个“空”映像（无任何表）的所有偏移都等于
/// [`HEADER_SIZE`]，`total_size` 也等于 [`HEADER_SIZE`]。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub(crate) magic: [u8; 8], // b"kosSTACK", goethite stack
    /// 映像总字节数（含头部、所有表与字符串池）。
    pub total_size: u32,
    /// `StringTable` 的字节偏移。
    pub index_offset: u32,
    /// `EnvVars` 的字节偏移。
    pub envp_offset: u32,
    /// `AuxValues` 的字节偏移。
    pub auxv_offset: u32,
    /// `Args` 的字节偏移。
    pub argv_offset: u32,
    /// `StringPool` 的字节偏移。
    pub pool_offset: u32,
}

impl Header {
    /// 一个“空”映像的头部：只有头部本身，各表偏移均指向 [`HEADER_SIZE`]（即无表）。
    pub const fn empty() -> Self {
        Self {
            magic: GOETHITE_STACK_MAGIC,
            total_size: size_of::<Self>() as u32,
            index_offset: size_of::<Self>() as u32,
            envp_offset: size_of::<Self>() as u32,
            auxv_offset: size_of::<Self>() as u32,
            argv_offset: size_of::<Self>() as u32,
            pool_offset: size_of::<Self>() as u32,
        }
    }

    /// 头部字节大小（恒等于 [`HEADER_SIZE`]）。
    pub const fn size() -> usize {
        size_of::<Self>()
    }

    /// 魔数。
    pub const fn magic(&self) -> &[u8; 8] {
        &self.magic
    }

    /// 魔数是否匹配且 `total_size` 至少能容纳头部本身。
    pub fn validate(&self) -> bool {
        self.magic == GOETHITE_STACK_MAGIC && self.total_size >= size_of::<Self>() as u32
    }
}

/// 指向字符串池中一段连续字节的引用（偏移量 + 长度）。
///
/// 字段均为相对映像起始地址的池内偏移（非绝对地址）。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StringRef {
    /// 在字符串池内的字节偏移。
    pub offset: u32,
    /// 字节长度。
    pub len: u32,
}

impl StringRef {
    pub const fn new(offset: u32, len: u32) -> Self {
        Self { offset, len }
    }

    /// 在池内对应的字节区间。
    pub fn range(&self) -> core::ops::Range<usize> {
        self.offset as usize..(self.offset as usize + self.len as usize)
    }
}

/// 字符串池中的索引标识符（用于快速定位字符串）。
///
/// 该值是对 [`StringTable`] 的索引。
pub type StringId = u32;

/// 一个键值对（如环境变量、辅助向量项），键和值均为 [`StringTable`] 中的索引。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    /// 键在 [`StringTable`] 中的索引。
    pub key: StringId,
    /// 值在 [`StringTable`] 中的索引。
    pub value: StringId,
}

impl Pair {
    pub const fn new(key: StringId, value: StringId) -> Self {
        Self { key, value }
    }
}

/// 字符串引用表（按序存放所有字符串的引用信息），按 [`StringId`] 索引。
#[derive(Clone, Copy, Debug)]
pub struct StringTable<'a>(pub &'a [StringRef]);

impl<'a> StringTable<'a> {
    pub fn as_slice(&self) -> &'a [StringRef] {
        self.0
    }
}

/// 环境变量表（按序存放所有环境变量）。
#[derive(Clone, Copy, Debug)]
pub struct EnvVars<'a>(pub &'a [Pair]);

impl<'a> EnvVars<'a> {
    pub fn as_slice(&self) -> &'a [Pair] {
        self.0
    }
}

/// 辅助向量表（按序存放所有辅助向量的键值对）。
#[derive(Clone, Copy, Debug)]
pub struct AuxValues<'a>(pub &'a [Pair]);

impl<'a> AuxValues<'a> {
    pub fn as_slice(&self) -> &'a [Pair] {
        self.0
    }
}

/// 命令行参数表（按序存放所有参数字符串对应的 `StringId`）。
#[derive(Clone, Copy, Debug)]
pub struct Args<'a>(pub &'a [StringId]);

impl<'a> Args<'a> {
    pub fn as_slice(&self) -> &'a [StringId] {
        self.0
    }
}

/// 字节池，存放所有实际字符串数据（UTF-8 字节序列）。
#[derive(Clone, Copy, Debug)]
pub struct StringPool<'a>(pub &'a [u8]);

impl<'a> StringPool<'a> {
    pub fn as_slice(&self) -> &'a [u8] {
        self.0
    }
}

// ===== 小端读写辅助（避免对齐依赖，可用于任意字节缓冲） =====

/// 从 `bytes` 的 `off` 处读取一个 `u32`（小端序）。
///
/// 调用方必须保证 `off + 4 <= bytes.len()`；此处直接切片，越界时 panic（用于已校验的区域）。
#[inline]
pub(crate) fn le32(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

/// 将 `value` 以小端序写入 `bytes` 的 `off` 处。
///
/// 调用方必须保证 `off + 4 <= bytes.len()`。
#[inline]
pub(crate) fn write_le32(bytes: &mut [u8], off: usize, value: u32) {
    bytes[off..off + 4].copy_from_slice(&value.to_le_bytes());
}
