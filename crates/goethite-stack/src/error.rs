//! goethite-stack 序列化 / 反序列化的错误类型。

use core::fmt;

/// 序列化 / 反序列化过程中可能出现的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// 提供的缓冲区不足以容纳序列化结果。
    BufferTooSmall {
        /// 需要的字节数。
        needed: usize,
        /// 实际提供的字节数。
        got: usize,
    },
    /// 字节流在某个区域被截断。
    Truncated {
        /// 截断的区域名（如 `"header"`、`"pool"`）。
        region: &'static str,
    },
    /// 魔数不匹配，不是合法的 goethite-stack 映像。
    BadMagic,
    /// 头部字段布局不合规：偏移越界、倒退，或总大小与偏移不一致。
    InvalidLayout,
    /// 某表区域未按元素大小对齐，无法进行零拷贝视图。
    Misaligned {
        /// 未对齐的区域名。
        region: &'static str,
    },
    /// 字符串池中的字节不是合法 UTF-8。
    NotUtf8 {
        /// 非法字节在字符串池中的偏移。
        offset: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BufferTooSmall { needed, got } => {
                write!(f, "buffer too small: need {needed} bytes, got {got} bytes")
            }
            Error::Truncated { region } => write!(f, "truncated region: {region}"),
            Error::BadMagic => write!(f, "bad goethite-stack magic"),
            Error::InvalidLayout => write!(f, "invalid goethite-stack layout"),
            Error::Misaligned { region } => write!(f, "misaligned region: {region}"),
            Error::NotUtf8 { offset } => {
                write!(f, "string pool is not valid utf-8 at offset {offset}")
            }
        }
    }
}
