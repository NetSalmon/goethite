//! virtio 设备常量。
//!
//! 汇总 virtio-mmio 传输层与具体设备（如 virtio-blk、virtio-net）用到的魔数、版本、
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

/// legacy virtio-net 包头长度。没有 `num_buffers` 字段。
pub const VIRTIO_NET_HDR_LEGACY: usize = 10;

/// modern virtio-net 包头长度。协商 `VIRTIO_F_VERSION_1` 之后 `num_buffers`
/// 总是存在；未协商 `VIRTIO_NET_F_MRG_RXBUF` 时设备把它填成 1。
pub const VIRTIO_NET_HDR_MODERN: usize = 12;

/// `VIRTIO_NET_F_MAC`：设备在配置空间给出 MAC 地址。
pub const VIRTIO_NET_F_MAC: u32 = 1 << 5;

/// modern 特性高 32 位中的 `VIRTIO_F_VERSION_1`（特性位 32）。
pub const VIRTIO_F_VERSION_1: u32 = 1 << 0;

/// avail 环 `flags`：驱动自己轮询完成状态，不要求设备发中断。
pub const VIRTQ_AVAIL_F_NO_INTERRUPT: u16 = 1;
