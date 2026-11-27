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
//!   Header                                // magic + 各表偏移 + total_size
//!   StringTable[StringRef;string_count]   // 字符串引用表
//!   EnvVars[Pair;env_count]               // 环境变量表
//!   AuxValues[AuxEntry;auxv_count]        // 辅助向量表
//!   Args[StringId;argc]                   // 命令行参数表
//!   StringPool[u8;pool_len]               // 字节池，存所有 UTF-8 字符串数据
//! sp:
//!   .quad stack_top
//! ```
//!
//! 环境变量表与命令行参数表的偏移与大小均按 4 字节对齐，辅助向量表项为 16 字节（按 8 字节对齐）。
//! 只要栈映像基址按 8 字节对齐，各表均能保证天然对齐。

use core::mem::size_of;

/// goethite-stack 映像的魔数：`b"kosSTACK"`。
pub const GOETHITE_STACK_MAGIC: [u8; 8] = [b'k', b'o', b's', b'S', b'T', b'A', b'C', b'K'];

/// 单个 [`StringRef`] 的字节大小（两个 `u32`）。
pub const STRING_REF_SIZE: usize = 8;
/// 单个 [`Pair`] 的字节大小（两个 `u32`）。
pub const PAIR_SIZE: usize = 8;
/// 单个 [`AuxEntry`] 的字节大小（两个 `u64`）。
pub const AUX_ENTRY_SIZE: usize = 16;
/// 单个 [`StringId`] 的字节大小（一个 `u32`）。
pub const STRING_ID_SIZE: usize = 4;
/// [`Header`] 的字节大小。
pub const HEADER_SIZE: usize = size_of::<Header>();

// ===== 常用 ELF 辅助向量类型编号（AT_*） =====

/// 辅助向量结束标志。
pub const AT_NULL: u64 = 0;
/// 应被忽略的条目。
pub const AT_IGNORE: u64 = 1;
/// 可执行文件的文件描述符。
pub const AT_EXECFD: u64 = 2;
/// 程序的 Program Header 虚拟地址。
pub const AT_PHDR: u64 = 3;
/// Program Header 表项的大小（字节）。
pub const AT_PHENT: u64 = 4;
/// Program Header 表项的数量。
pub const AT_PHNUM: u64 = 5;
/// 系统页面大小（字节）。
pub const AT_PAGESZ: u64 = 6;
/// 解释器（动态链接器）基地址。
pub const AT_BASE: u64 = 7;
/// 标志位。
pub const AT_FLAGS: u64 = 8;
/// 程序入口地址。
pub const AT_ENTRY: u64 = 9;
/// 非 ELF 文件。
pub const AT_NOTELF: u64 = 10;
/// 真实用户 ID。
pub const AT_UID: u64 = 11;
/// 有效用户 ID。
pub const AT_EUID: u64 = 12;
/// 真实组 ID。
pub const AT_GID: u64 = 13;
/// 有效组 ID。
pub const AT_EGID: u64 = 14;
/// 平台标识字符串地址。
pub const AT_PLATFORM: u64 = 15;
/// 硬件能力位掩码。
pub const AT_HWCAP: u64 = 16;
/// 时钟频率（`sysconf(_SC_CLK_TCK)`）。
pub const AT_CLKTCK: u64 = 17;
/// 安全执行标志。
pub const AT_SECURE: u64 = 23;
/// 基础平台字符串地址。
pub const AT_BASE_PLATFORM: u64 = 24;
/// 指向 16 字节随机数的地址。
pub const AT_RANDOM: u64 = 25;
/// 硬件能力扩展位掩码。
pub const AT_HWCAP2: u64 = 26;
/// 可执行文件路径名字符串地址。
pub const AT_EXECFN: u64 = 31;
/// 系统调用入口地址。
pub const AT_SYSINFO: u64 = 32;
/// vDSO ELF 头地址。
pub const AT_SYSINFO_EHDR: u64 = 33;

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

/// 一个键值对（如环境变量项），键和值均为 [`StringTable`] 中的索引。
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

/// 一个辅助向量项（ELF auxiliary vector entry），键与值均为 64 位整数。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct AuxEntry {
    /// 辅助向量类型编号（如 [`AT_PHDR`]、[`AT_ENTRY`] 等）。
    pub key: u64,
    /// 辅助向量内容（指针、地址或数值）。
    pub value: u64,
}

impl AuxEntry {
    pub const fn new(key: u64, value: u64) -> Self {
        Self { key, value }
    }
}

impl From<(u64, u64)> for AuxEntry {
    fn from((key, value): (u64, u64)) -> Self {
        Self { key, value }
    }
}

impl From<AuxEntry> for (u64, u64) {
    fn from(entry: AuxEntry) -> Self {
        (entry.key, entry.value)
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

/// 辅助向量表（按序存放所有辅助向量项）。
#[derive(Clone, Copy, Debug)]
pub struct AuxValues<'a>(pub &'a [AuxEntry]);

impl<'a> AuxValues<'a> {
    pub fn as_slice(&self) -> &'a [AuxEntry] {
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

/// 从 `bytes` 的 `off` 处读取一个 `u64`（小端序）。
///
/// 调用方必须保证 `off + 8 <= bytes.len()`；此处直接切片，越界时 panic（用于已校验的区域）。
#[inline]
#[allow(dead_code)]
pub(crate) fn le64(bytes: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        bytes[off],
        bytes[off + 1],
        bytes[off + 2],
        bytes[off + 3],
        bytes[off + 4],
        bytes[off + 5],
        bytes[off + 6],
        bytes[off + 7],
    ])
}

/// 将 `value` 以小端序写入 `bytes` 的 `off` 处。
///
/// 调用方必须保证 `off + 8 <= bytes.len()`。
#[inline]
pub(crate) fn write_le64(bytes: &mut [u8], off: usize, value: u64) {
    bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
}
