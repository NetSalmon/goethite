//! 文件系统子系统。
//!
//! 把 `goethite-fs` 的 VFS 与 `goethite-minix` 的 MINIX v1 驱动接入内核：
//!
//! - [`DeviceBlock`]：把内核块设备（[`crate::device::Device`]）适配为文件系统层
//!   的 [`BlockDevice`]，使 MINIX 驱动无需感知设备管理器的内部结构；
//! - [`VFS`]：全内核唯一的挂载表，按路径前缀把请求分发到各文件系统；
//! - [`init`]：从 [`DEVICE_MANAGER`] 取出第一台块设备，识别 MINIX v1 并挂载到
//!   根目录 `/`。
//!
//! 挂载完成后即可通过 [`VFS`] 按路径打开 / 读写文件，例如读取根目录下的
//! `hello` 文件。

use alloc::{sync::Arc, vec::Vec};

use goethite_fs::{
    device::{BlockDevice, IoError, IoResult},
    error::FsResult,
    path::Path,
    types::Mode,
    vfs::{FileSystem, OpenFlags, Vfs},
};
use goethite_minix::minix::{MinixFs, MinixVfs};
use goethite_sync::RwLock;

use crate::{
    device::{DEVICE_MANAGER, Device, FileType},
    error::{KernelError, KernelResult},
};

/// 打开文件描述符（占位：后续与 `goethite_fs::vfs::File` 对接）。
pub struct FileDesc {}

/// 内核全局 VFS：所有挂载点与路径分发的中枢。
///
/// 以 `const` 构造（内部读写锁懒保护挂载表），首次 [`init`] 时挂载根文件系统。
pub static VFS: Vfs = Vfs::new();

/// 把内核 [`Device`]（块设备）适配为文件系统层的 [`BlockDevice`]。
///
/// 两者都以扇区为单位寻址（`read_at` / `write_at` 返回扇区数），因此适配器
/// 只需把 [`DeviceError`](crate::device::DeviceError) 映射为文件系统层的
/// [`IoError`]。
struct DeviceBlock(Arc<dyn Device>);

impl BlockDevice for DeviceBlock {
    fn read_at(&self, sector: usize, buf: &mut [u8]) -> IoResult<usize> {
        self.0.read_at(sector, buf).map_err(|_| IoError::Request)
    }

    fn write_at(&self, sector: usize, buf: &[u8]) -> IoResult<usize> {
        self.0.write_at(sector, buf).map_err(|_| IoError::Request)
    }
}

/// 初始化文件系统：找到块设备、识别 MINIX v1 并挂载到根目录 `/`。
///
/// # Errors
///
/// - [`KernelError::NoBlockDevice`]：设备管理器中没有块设备。
/// - [`KernelError::FsIoError`]：读取超级块失败。
/// - [`KernelError::NotMinixFs`]：块设备上的 magic 不是 MINIX v1。
/// - [`KernelError::FsError`]：挂载失败（如根路径已被占用）。
pub fn init() -> KernelResult<()> {
    let device = find_block_device().ok_or(KernelError::NoBlockDevice)?;
    let block = Arc::new(RwLock::new(DeviceBlock(device)));

    let minix = MinixFs::from_device(&block)?.ok_or(KernelError::NotMinixFs)?;

    let fs: Arc<dyn FileSystem> = Arc::new(MinixVfs::new(Arc::new(minix)));
    VFS.mount(&Path::new("/"), fs)?;

    Ok(())
}

/// 读取文件全部内容。
///
/// 按路径经 [`VFS`] 打开文件并顺序读完；文件不存在等错误由 [`FsError`] 返回。
///
/// [`FsError`]: goethite_fs::error::FsError
pub fn read_file(path: &str) -> FsResult<Vec<u8>> {
    let file = VFS.open(&Path::new(path), OpenFlags::read_only(), Mode::from(0))?;

    let mut data = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
    }
    Ok(data)
}

/// 从设备管理器中取出第一台块设备（`mode` 的文件类型为 [`FileType::BLK`]）。
fn find_block_device() -> Option<Arc<dyn Device>> {
    DEVICE_MANAGER
        .read()
        .all()
        .into_iter()
        .find(|node| node.mode.file_type() == FileType::BLK)
        .map(|node| node.device.clone())
}
