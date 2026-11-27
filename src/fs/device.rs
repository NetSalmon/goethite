//! 设备文件系统（devfs）。
//!
//! [`DeviceFs`] 实现 [`goethite_fs::vfs::FileSystem`]，将 [`DEVICE_MANAGER`] 中所有已注册的
//! [`DeviceNode`] 实时映射为挂载点（通常为 `/dev`）下的设备节点：
//!
//! - 根目录 `/`：可打开并经 [`File::read_dir`](goethite_fs::vfs::File::read_dir) 遍历当前全部设备；
//! - `/<name>`：按设备名查找 [`DeviceNode`]，打开后通过 [`DeviceFileOps`] 把 VFS 读写转发到
//!   [`Device::read`] / [`Device::write`]（块设备按扇区转发到 [`Device::read_at`] / [`Device::write_at`]）。

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};

use goethite_config::SECTOR_SIZE;
use goethite_fs::{
    error::{FsError, FsResult},
    path::{Path, PathBuf},
    types::{FileType, Mode},
    vfs::{
        DirEntry, File, FileSystem, OpenFlags, Stat,
        file_ops::{FileOps, Whence},
    },
};

use crate::device::{
    DEVICE_MANAGER, DeviceError, DeviceNode, FileType as DevFileType, Mode as DevMode,
    desc::DeviceId,
};

/// devfs 根目录固定 inode 号；设备节点 inode 号从 `2` 起按 `DeviceId` 映射。
const ROOT_INO: u64 = 1;

fn device_ino(id: DeviceId) -> u64 {
    (*id) as u64 + 2
}

fn convert_mode(mode: DevMode) -> Mode {
    Mode::from(*mode)
}

fn convert_file_type(ft: DevFileType) -> FileType {
    FileType::from(ft.0)
}

fn root_mode() -> Mode {
    Mode::builder()
        .set_file_type(FileType::DIR)
        .set_user_read(true)
        .set_user_write(true)
        .set_user_execute(true)
        .set_group_read(true)
        .set_group_execute(true)
        .set_other_read(true)
        .set_other_execute(true)
        .build()
}

fn map_device_err(err: DeviceError) -> FsError {
    match err {
        DeviceError::Unsupported => FsError::Unsupported,
        DeviceError::NotFound => FsError::NotFound,
        DeviceError::AllocFailed => FsError::OutOfMemory,
        DeviceError::InvalidMajor | DeviceError::MajorInUse => FsError::Invalid,
        DeviceError::Io | DeviceError::NotReady => FsError::Io,
    }
}

/// 设备文件系统：动态反映 [`DEVICE_MANAGER`] 中的已注册设备。
#[derive(Debug, Default, Clone, Copy)]
pub struct DeviceFs;

impl DeviceFs {
    pub const fn new() -> Self {
        Self
    }

    /// 解析相对 devfs 根目录的路径：
    /// - `/` → `Ok(None)`（根目录本身）
    /// - `/<name>` → `Ok(Some(name))`（单级设备名）
    /// - 多级路径（如 `/a/b`）→ `Err(FsError::NotFound)` 或 `Err(FsError::NotDir)`
    fn parse_path(path: &Path) -> FsResult<Option<&str>> {
        let mut parts = path.as_str().split('/').filter(|s| !s.is_empty());
        let Some(first) = parts.next() else {
            return Ok(None);
        };
        if parts.next().is_some() {
            // 第一级若存在则是设备文件（非目录），返回 ENOTDIR；否则 ENOENT。
            if !DEVICE_MANAGER.read().by_name(first).is_empty() {
                return Err(FsError::NotDir);
            }
            return Err(FsError::NotFound);
        }
        Ok(Some(first))
    }

    fn lookup_node(name: &str) -> FsResult<Arc<DeviceNode>> {
        DEVICE_MANAGER
            .read()
            .by_name(name)
            .into_iter()
            .next()
            .ok_or(FsError::NotFound)
    }
}

impl FileSystem for DeviceFs {
    fn open(&self, path: &Path, flags: OpenFlags, _mode: Mode) -> FsResult<File> {
        let Some(name) = Self::parse_path(path)? else {
            if flags.can_write() {
                return Err(FsError::IsDir);
            }
            let ops: Arc<dyn FileOps> = Arc::new(DevDirOps::snapshot());
            return Ok(File::from_ops("/", ROOT_INO, root_mode(), ops, flags));
        };

        if flags.directory() {
            return Err(FsError::NotDir);
        }

        let node = Self::lookup_node(name)?;
        if flags.create() && flags.exclusive() {
            return Err(FsError::AlreadyExists);
        }

        let mode = convert_mode(node.mode);
        let ino = device_ino(node.id);
        let node_name = node.name.clone();
        let ops: Arc<dyn FileOps> = Arc::new(DeviceFileOps { node, flags });
        Ok(File::from_ops(&node_name, ino, mode, ops, flags))
    }

    fn stat(&self, path: &Path) -> FsResult<Stat> {
        let Some(name) = Self::parse_path(path)? else {
            return Ok(Stat {
                ino: ROOT_INO,
                mode: root_mode(),
                size: 0,
                nlinks: 2,
                mtime: 0,
                uid: 0,
                gid: 0,
            });
        };

        let node = Self::lookup_node(name)?;
        Ok(Stat {
            ino: device_ino(node.id),
            mode: convert_mode(node.mode),
            size: 0,
            nlinks: 1,
            mtime: 0,
            uid: 0,
            gid: 0,
        })
    }

    fn mkdir(&self, _path: &Path, _mode: Mode) -> FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn unlink(&self, _path: &Path) -> FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn rmdir(&self, _path: &Path) -> FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn rename(&self, _old: &Path, _new: &Path) -> FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn link(&self, _old: &Path, _new: &Path) -> FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn symlink(&self, _target: &str, _linkpath: &Path) -> FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn readlink(&self, _path: &Path) -> FsResult<PathBuf> {
        Err(FsError::Unsupported)
    }

    fn sync(&self) -> FsResult<()> {
        Ok(())
    }
}

/// devfs 根目录的目录遍历操作：打开时对 [`DEVICE_MANAGER`] 做一次快照。
struct DevDirOps {
    entries: Vec<DirEntry>,
    cursor: AtomicUsize,
}

impl DevDirOps {
    fn snapshot() -> Self {
        let entries = DEVICE_MANAGER
            .read()
            .all()
            .into_iter()
            .map(|node| DirEntry {
                ino: device_ino(node.id),
                name: node.name.clone(),
                file_type: convert_file_type(node.mode.file_type()),
            })
            .collect();
        Self {
            entries,
            cursor: AtomicUsize::new(0),
        }
    }
}

impl FileOps for DevDirOps {
    fn read_dir(&self) -> FsResult<Option<DirEntry>> {
        let idx = self.cursor.fetch_add(1, Ordering::Relaxed);
        Ok(self.entries.get(idx).cloned())
    }
}

/// 单个打开设备节点的文件操作，将 VFS I/O 转发到底层 [`Device`](crate::device::Device)。
struct DeviceFileOps {
    node: Arc<DeviceNode>,
    flags: OpenFlags,
}

impl FileOps for DeviceFileOps {
    fn read(&self, buf: &mut [u8]) -> FsResult<usize> {
        if !self.flags.can_read() {
            return Err(FsError::PermissionDenied);
        }
        self.node.device.read(buf).map_err(map_device_err)
    }

    fn write(&self, buf: &[u8]) -> FsResult<usize> {
        if !self.flags.can_write() {
            return Err(FsError::PermissionDenied);
        }
        self.node.device.write(buf).map_err(map_device_err)
    }

    fn read_at(&self, buf: &mut [u8], offset: u64) -> FsResult<usize> {
        if !self.flags.can_read() {
            return Err(FsError::PermissionDenied);
        }
        if self.node.mode.file_type() == DevFileType::BLK {
            return read_block_bytes(&self.node, offset, buf);
        }
        self.node.device.read(buf).map_err(map_device_err)
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> FsResult<usize> {
        if !self.flags.can_write() {
            return Err(FsError::PermissionDenied);
        }
        if self.node.mode.file_type() == DevFileType::BLK {
            return write_block_bytes(&self.node, offset, buf);
        }
        self.node.device.write(buf).map_err(map_device_err)
    }

    fn seek(&self, current: u64, offset: i64, whence: Whence) -> FsResult<i64> {
        if self.node.mode.file_type() != DevFileType::BLK {
            return Err(FsError::Unsupported);
        }
        let base = match whence {
            Whence::SEEK_SET => 0i64,
            Whence::SEEK_CUR => i64::try_from(current).map_err(|_| FsError::Invalid)?,
            Whence::SEEK_END => return Err(FsError::Unsupported),
            _ => return Err(FsError::Invalid),
        };
        let target = base.checked_add(offset).ok_or(FsError::Invalid)?;
        if target < 0 {
            return Err(FsError::Invalid);
        }
        Ok(target)
    }
}

/// 按字节偏移从块设备读取数据（内部按 `SECTOR_SIZE` 扇区缓冲）。
fn read_block_bytes(node: &DeviceNode, offset: u64, buf: &mut [u8]) -> FsResult<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    let mut sector_buf = [0u8; SECTOR_SIZE];
    let mut done = 0usize;
    let mut pos = offset as usize;

    while done < buf.len() {
        let sector = pos / SECTOR_SIZE;
        let in_sector = pos % SECTOR_SIZE;
        let n = (SECTOR_SIZE - in_sector).min(buf.len() - done);

        node.device
            .read_at(sector, &mut sector_buf)
            .map_err(map_device_err)?;
        buf[done..done + n].copy_from_slice(&sector_buf[in_sector..in_sector + n]);

        done += n;
        pos += n;
    }
    Ok(done)
}

/// 按字节偏移向块设备写入数据（非整扇区写入采用 read-modify-write）。
fn write_block_bytes(node: &DeviceNode, offset: u64, buf: &[u8]) -> FsResult<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    let mut sector_buf = [0u8; SECTOR_SIZE];
    let mut done = 0usize;
    let mut pos = offset as usize;

    while done < buf.len() {
        let sector = pos / SECTOR_SIZE;
        let in_sector = pos % SECTOR_SIZE;
        let n = (SECTOR_SIZE - in_sector).min(buf.len() - done);

        if in_sector != 0 || n < SECTOR_SIZE {
            node.device
                .read_at(sector, &mut sector_buf)
                .map_err(map_device_err)?;
        }
        sector_buf[in_sector..in_sector + n].copy_from_slice(&buf[done..done + n]);
        node.device
            .write_at(sector, &sector_buf)
            .map_err(map_device_err)?;

        done += n;
        pos += n;
    }
    Ok(done)
}
