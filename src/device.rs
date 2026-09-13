//! 设备子系统。
//!
//! - [`Device`]：块 / 字符设备的统一读写抽象；
//! - [`DeviceNode`]：已注册的设备节点（设备 id + 实例 + 元数据）；
//! - [`Resource`]：设备在系统总线上的地址资源窗口（设备树 `reg` 区间）；
//! - [`FileType`] / [`Mode`]：UNIX `st_mode` 的文件类型与权限位（供文件系统复用）；
//! - [`desc`]：从设备树（FDT）解析设备描述并建立索引；
//! - [`driver`]：设备驱动注册表。

pub mod desc;
pub mod driver;

use alloc::{string::String, sync::Arc};
use core::ops::Range;

use desc::{DeviceDesc, DeviceId};
use goethite_macros::{bits, numeric};

/// 设备操作可能出现的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    /// 目标设备不支持所请求的操作（如对只读设备执行写入）。
    #[error("operation not supported by the device")]
    Unsupported,
}

/// 设备操作的结果：
///
/// - `Ok(bytes)`：成功完成，携带本次传输/synchronize 的**单元数**（对
///   [`Device::read`] / [`Device::write`] 是字节数，对 [`Device::read_at`] /
///   [`Device::write_at`] 是扇区数）。
/// - `Err(e)`：底层驱动返回 [`DeviceError`]。
pub type DeviceResult = Result<usize, DeviceError>;

/// 块 / 字符设备的上层抽象：以一致的读写接口屏蔽幕后驱动差异。
#[allow(unused)]
pub trait Device {
    /// 从设备顺序读取数据到 `buf`，返回实际读到的字节数。
    fn read(&self, buf: &mut [u8]) -> DeviceResult {
        Err(DeviceError::Unsupported)
    }

    /// 把 `buf` 的数据顺序写入设备，返回实际写入的字节数。
    fn write(&self, buf: &[u8]) -> DeviceResult {
        Err(DeviceError::Unsupported)
    }

    /// 从设备第 `sector` 个扇区处读取数据到 `buf`，返回实际读到的扇区数。
    fn read_at(&self, sector: usize, buf: &mut [u8]) -> DeviceResult {
        Err(DeviceError::Unsupported)
    }

    /// 把 `buf` 的数据写入设备第 `sector` 个扇区处，返回实际写入的扇区数。
    fn write_at(&self, sector: usize, buf: &[u8]) -> DeviceResult {
        Err(DeviceError::Unsupported)
    }
}

// 文件类型（mode 高 4 位，bits 15..12 的取值），与 POSIX `S_IF*` 一致。
numeric! {
    pub enum FileType : u16 {
        FIFO = 0o1,   // 命名管道  S_IFIFO
        CHR = 0o2,    // 字符设备  S_IFCHR
        DIR = 0o4,    // 目录      S_IFDIR
        BLK = 0o6,    // 块设备    S_IFBLK
        REG = 0o10,   // 普通文件  S_IFREG
        LNK = 0o12,   // 符号链接  S_IFLNK
        SOCK = 0o14,  // 套接字    S_IFSOCK
    }
}

// 文件 mode 位，布局与 Unix `st_mode` 一致：
//
// ```text
// 15..12      11      10     9    8 7 6   5 4 3   2 1 0
// 文件类型  setuid setgid sticky  属主    属组    其他
//                                rwx     rwx     rwx
// ```
bits! {
    pub type Mode : u16 {
        // 文件类型位（bits 15..12），取值见下方 `FileType` 枚举。
        file_type: FileType : 12 => 15,

        // 特殊权限位。
        setuid: 11,
        setgid: 10,
        sticky: 9,

        // 属主（owner）rwx 权限位。
        user_execute: 6,
        user_write: 7,
        user_read: 8,
        user: 6 => 8,

        // 属组（group）rwx 权限位。
        group_execute: 3,
        group_write: 4,
        group_read: 5,
        group: 3 => 5,

        // 其他用户（other）rwx 权限位。
        other_execute: 0,
        other_write: 1,
        other_read: 2,
        other: 0 => 2,
    }
}

/// 一个向内核注册的设备节点：能唯一标识设备、持有驱动实例与其元数据。
pub struct DeviceNode {
    /// 设备在 [`desc::DeviceDescManager`] 中的唯一 id。
    pub id: DeviceId,
    /// 设备的驱动实现（trait object）。
    pub device: Arc<dyn Device>,

    /// 设备在文件系统 / 命名空间中的访问权限位。
    pub mode: Mode,
    /// 设备的名字（来自设备树的节点名）。
    pub name: String,

    /// 由设备树解析出的设备描述。
    pub desc: Arc<DeviceDesc>,
}

/// 设备在系统总线上的一个地址资源窗口（对应设备树的 `reg` 区间）。
#[derive(Debug)]
pub struct Resource {
    /// 窗口的起始物理地址 / 偏移。
    pub start: usize,
    /// 窗口的大小（字节）。
    pub size: usize,
}

impl Resource {
    /// 窗口的结束地址（不包含），即 `start + size`。
    pub fn end(&self) -> usize {
        self.start + self.size
    }

    /// 构造一个地址资源窗口。
    pub fn new(start: usize, size: usize) -> Self {
        Self { start, size }
    }
}

impl From<Range<usize>> for Resource {
    fn from(range: Range<usize>) -> Resource {
        let start = range.start;
        let size = range.end - start;

        Resource { start, size }
    }
}
