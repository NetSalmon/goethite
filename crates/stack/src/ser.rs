//! goethite-stack 序列化：把结构化的 argv / env / auxv 写成紧凑的栈映像字节。
//!
//! 本模块不依赖 `alloc`（除 [`crate::Stack`] 的 `to_bytes` 包装外），核心入口是：
//! - [`required_len`]：计算序列化结果所需的字节数（无需分配，可据此预分配缓冲区）；
//! - [`write`]：把 [`Input`] 写入由调用方提供的 `&mut [u8]`，返回实际写入的字节数。
//!
//! 为避免分配，字符串去重采用 O(n²) 的逐字节比较（对内核侧的 argc/env 规模而言足够快），
//! 代价是相对于基于哈希表的去重更慢，但完全零分配。

use crate::{
    error::Error,
    raw::{
        AUX_ENTRY_SIZE, GOETHITE_STACK_MAGIC, HEADER_SIZE, PAIR_SIZE, STRING_ID_SIZE,
        STRING_REF_SIZE, write_le32, write_le64,
    },
};

/// 序列化的输入：全部为借用切片，便于与 `#![no_std]` / 无分配环境配合。
///
/// `env` 为按序排列的环境变量键值对切片；`auxv` 为按序排列的辅助向量（编号与数值）切片；`argv` 为按序排列的参数字符串切片。
/// 注意：`env` 出现重复键是允许的，序列化时各自独立去重字节内容，但在还原为
/// [`crate::Stack`]（使用 `BTreeMap`）时重复键会被后者折叠。
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// 命令行参数（`argv`）。
    pub argv: &'a [&'a str],
    /// 环境变量（`env`），键值对。
    pub env: &'a [(&'a str, &'a str)],
    /// 辅助向量（`auxv`），编号与内容。
    pub auxv: &'a [(u64, u64)],
}

impl<'a> Input<'a> {
    /// 构造一个空的 [`Input`]（无参数、无环境、无辅助向量）。
    pub fn empty() -> Self {
        Self {
            argv: &[],
            env: &[],
            auxv: &[],
        }
    }
}

/// 规范序字符串总数：先按对交替列出 `env` 的键与值，最后是 `argv`。
fn canonical_len(input: &Input<'_>) -> usize {
    input.env.len() * 2 + input.argv.len()
}

/// 规范序中第 `j` 个字符串。
fn canonical_str<'a>(input: &Input<'a>, j: usize) -> &'a str {
    let envn = input.env.len();
    if j < envn * 2 {
        let pair = input.env[j / 2];
        if j % 2 == 0 { pair.0 } else { pair.1 }
    } else {
        input.argv[j - envn * 2]
    }
}

/// 第 `j` 个字符串是否为“新字符串”（规范序中首个与其字节相同的串）。
fn is_new(input: &Input<'_>, j: usize) -> bool {
    let s = canonical_str(input, j);
    for k in 0..j {
        if canonical_str(input, k) == s {
            return false;
        }
    }
    true
}

/// `[0, j)` 区间内新字符串的个数（也即第 `j` 个字符串若是新串时的 `StringId`）。
fn new_count_before(input: &Input<'_>, j: usize) -> usize {
    let mut c = 0;
    for k in 0..j {
        if is_new(input, k) {
            c += 1;
        }
    }
    c
}

/// 规范序中第 `j` 个字符串对应的 `StringId`。
///
/// 若是新串，则为新串计数；若是重复串，则为首次出现处对应新串的 `StringId`。
fn id_of(input: &Input<'_>, j: usize) -> usize {
    let s = canonical_str(input, j);
    for k in 0..j {
        if canonical_str(input, k) == s {
            return new_count_before(input, k);
        }
    }
    new_count_before(input, j)
}

/// 字符串表长度（去重后唯一的字符串个数）。
fn string_count(input: &Input<'_>) -> usize {
    new_count_before(input, canonical_len(input))
}

/// 字符串池总字节数（去重后所有唯一字符串的字节长度之和）。
fn pool_len(input: &Input<'_>) -> usize {
    let mut n = 0;
    for j in 0..canonical_len(input) {
        if is_new(input, j) {
            n += canonical_str(input, j).len();
        }
    }
    n
}

/// 计算把 [`Input`] 序列化后所需的字节数。
///
/// 该方法只读输入，不写任何缓冲，可在分配前调用以确定缓冲区大小。
pub fn required_len(input: &Input<'_>) -> usize {
    let count = string_count(input);
    HEADER_SIZE
        + count * STRING_REF_SIZE
        + input.env.len() * PAIR_SIZE
        + input.auxv.len() * AUX_ENTRY_SIZE
        + input.argv.len() * STRING_ID_SIZE
        + pool_len(input)
}

/// 把 [`Input`] 序列化并写入 `buf`，返回实际写入的字节数（与 [`required_len`] 一致）。
///
/// 当 `buf` 小于所需长度时返回 [`Error::BufferTooSmall`]。
pub fn write(buf: &mut [u8], input: &Input<'_>) -> Result<usize, Error> {
    let total = required_len(input);
    if buf.len() < total {
        return Err(Error::BufferTooSmall {
            needed: total,
            got: buf.len(),
        });
    }

    let count = string_count(input);
    let envn = input.env.len();
    let auxn = input.auxv.len();
    let argc = input.argv.len();

    let index_offset = HEADER_SIZE;
    let envp_offset = index_offset + count * STRING_REF_SIZE;
    let auxv_offset = envp_offset + envn * PAIR_SIZE;
    let argv_offset = auxv_offset + auxn * AUX_ENTRY_SIZE;
    let pool_offset = argv_offset + argc * STRING_ID_SIZE;
    let total_size = total as u32;

    // 头部
    buf[0..8].copy_from_slice(&GOETHITE_STACK_MAGIC);
    write_le32(buf, 8, total_size);
    write_le32(buf, 12, index_offset as u32);
    write_le32(buf, 16, envp_offset as u32);
    write_le32(buf, 20, auxv_offset as u32);
    write_le32(buf, 24, argv_offset as u32);
    write_le32(buf, 28, pool_offset as u32);

    // 字符串表 + 字符串池（按规范序，只对“新串”写入表项并拷贝池字节）
    let mut count_written = 0usize;
    let mut pool_pos = 0usize;
    for j in 0..canonical_len(input) {
        let s = canonical_str(input, j);
        if is_new(input, j) {
            let ref_off = index_offset + count_written * STRING_REF_SIZE;
            buf[ref_off..ref_off + 4].copy_from_slice(&(pool_pos as u32).to_le_bytes());
            buf[ref_off + 4..ref_off + 8].copy_from_slice(&(s.len() as u32).to_le_bytes());

            let pool_at = pool_offset + pool_pos;
            buf[pool_at..pool_at + s.len()].copy_from_slice(s.as_bytes());
            pool_pos += s.len();
            count_written += 1;
        }
    }
    debug_assert_eq!(count_written, count);

    // 环境变量表
    for i in 0..input.env.len() {
        let off = envp_offset + i * PAIR_SIZE;
        write_le32(buf, off, id_of(input, i * 2) as u32);
        write_le32(buf, off + 4, id_of(input, i * 2 + 1) as u32);
    }

    // 辅助向量表
    for (i, &(key, value)) in input.auxv.iter().enumerate() {
        let off = auxv_offset + i * AUX_ENTRY_SIZE;
        write_le64(buf, off, key);
        write_le64(buf, off + 8, value);
    }

    // 参数表
    let arg_base = envn * 2;
    for (i, _arg) in input.argv.iter().enumerate() {
        let off = argv_offset + i * STRING_ID_SIZE;
        write_le32(buf, off, id_of(input, arg_base + i) as u32);
    }

    Ok(total)
}
