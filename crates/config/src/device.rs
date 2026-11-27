//! 设备常量。
//!
//! [`DEVICE_ID_MINOR_BITS`] 决定每个主设备号下的从设备号数量，
//! [`DEVICE_ID_MAJOR_MAX`] 为主设备号数量上限，[`DEVICE_ID_MAX`] 是两者合成的
//! 设备 id 空间上界。
use goethite_macros::const_val;

/// 设备号中从设备号（minor）占用的比特数：每个主设备号下有
/// `2^DEVICE_ID_MINOR_BITS` 个从设备号。
#[const_val]
pub const DEVICE_ID_MINOR_BITS: usize = 8;

/// 主设备号（major）数量上限。
#[const_val]
pub const DEVICE_ID_MAJOR_MAX: usize = 16;

/// 设备 id 分配器的最大可分配值（`major` 空间左移 minor 位数）。
pub const DEVICE_ID_MAX: usize = DEVICE_ID_MAJOR_MAX << DEVICE_ID_MINOR_BITS;
