//! virtio 设备常量。
//!
//! 汇总 virtio-mmio 传输层与具体设备（如 virtio-blk）用到的魔数、版本、
//! 虚拟队列环大小、请求类型与完成状态等编译期常量。
#![allow(dead_code)]

use goethite_macros::const_val;

/// virtio-mmio 魔数寄存器（`magic_value`）的期望值（小端 `"virt"`）。
pub const MAGIC_VALUE: u32 = 0x74726976;

/// legacy virtio-mmio 传输版本（`version` 寄存器值）。
pub const VIRTIO_VERSION_LEGACY: u32 = 1;

/// 虚拟队列环的描述符数量（desc / avail / used 三环共用）。
#[const_val(multiple_of = 2, max = 1024, min = 64)]
pub const RING_SIZE: usize = 256;

/// 单个块设备逻辑扇区的字节数。
pub const SECTOR_SIZE: usize = 512;

/// virtio-blk 请求类型：读（设备向数据缓冲区写入）。
pub const VIRTIO_BLK_T_IN: u32 = 0;
/// virtio-blk 请求类型：写（设备从数据缓冲区读取）。
pub const VIRTIO_BLK_T_OUT: u32 = 1;
/// virtio-blk 请求完成状态：成功。
pub const VIRTIO_BLK_S_OK: u8 = 0;
