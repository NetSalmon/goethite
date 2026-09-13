//! 版本信息（uname）。
use goethite_macros::const_val;

/// 内核版本字符串（uname 的 `version` 字段）。
#[const_val]
pub const VERSION: &str = "0.1.0";
/// 内核发布标识（uname 的 `release` 字段）。
pub const RELEASE: &str = "none";
/// 操作系统名称（uname 的 `sysname` 字段）。
#[const_val]
pub const SYS: &str = "Chaahann";
/// 机器/架构标识（uname 的 `machine` 字段）。
#[const_val]
pub const ARCH: &str = "riscv64gc";
