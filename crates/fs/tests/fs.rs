use std::sync::{Arc, Mutex};

use goethite_fs::{
    BlockDevice, FsError, IoError, IoResult, SECTOR_SIZE, Whence,
    path::{Component, Path, PathBuf},
    types::{FileType, Mode},
    vfs::{AccessMode, File, FileSystem, OpenFlags, Stat, Vfs, file_ops::FileOps},
};

// ===== 路径 =====

#[test]
fn path_queries() {
    let path = Path::new("/usr/bin/rustc");
    assert!(path.is_absolute());
    assert!(!path.is_relative());
    assert_eq!(path.parent(), Some("/usr/bin"));
    assert_eq!(path.file_name(), Some("rustc"));
    assert_eq!(path.extension(), None);

    assert_eq!(Path::new("/a/b.tar.gz").extension(), Some("gz"));
    assert_eq!(Path::new("/a/").file_name(), Some("a"));
    assert_eq!(Path::new("/").file_name(), None);
    assert_eq!(Path::new("/").parent(), None);
    assert_eq!(Path::new("a").parent(), Some(""));
    assert_eq!(Path::new("a/b").parent(), Some("a"));

    assert!(path.starts_with(Path::new("/usr")));
    assert!(!path.starts_with(Path::new("/usr/bin/rust")));
    assert!(path.ends_with(Path::new("bin/rustc")));
    assert!(!path.ends_with(Path::new("usr/bin/rustc/extra")));
}

#[test]
fn path_components() {
    let path = Path::new("/a/./b/../c");
    let components: Vec<_> = path.components().collect();
    assert_eq!(
        components,
        vec![
            Component::RootDir,
            Component::Normal("a"),
            Component::CurDir,
            Component::Normal("b"),
            Component::ParentDir,
            Component::Normal("c"),
        ]
    );
    assert_eq!(Component::RootDir.as_str(), "");
    assert_eq!(Component::CurDir.as_str(), ".");
    assert_eq!(Component::ParentDir.as_str(), "..");
    assert_eq!(Component::Normal("x").as_str(), "x");

    assert_eq!(
        Path::new("//a///b/").components().collect::<Vec<_>>(),
        vec![
            Component::RootDir,
            Component::Normal("a"),
            Component::Normal("b")
        ]
    );
}

#[test]
fn pathbuf_push_pop_join() {
    let mut buf = PathBuf::from("/a/b");
    buf.push(Path::new("c"));
    assert_eq!(buf.as_str(), "/a/b/c");
    buf.push(Path::new("/absolute"));
    assert_eq!(buf.as_str(), "/absolute");

    assert!(buf.pop());
    assert_eq!(buf.as_str(), "/");
    assert!(!buf.pop());

    let mut buf = PathBuf::new();
    buf.push(Path::new("a"));
    buf.push(Path::new("b"));
    assert_eq!(buf.as_str(), "a/b");
    assert_eq!(buf.join(Path::new("c")).as_str(), "a/b/c");

    let mut buf = PathBuf::from("/a/b");
    buf.set_file_name("c");
    assert_eq!(buf.as_str(), "/a/c");
    assert_eq!(Path::new("/a/b").join(Path::new("c")).as_str(), "/a/b/c");
}

// ===== 类型与错误 =====

#[test]
fn mode_bits_and_file_types() {
    let mode = Mode::from(0o100644u16);
    assert_eq!(mode.file_type(), FileType::REG);
    assert!(mode.user_read());
    assert!(mode.user_write());
    assert!(!mode.user_execute());
    assert!(mode.group_read());
    assert!(mode.other_read());
    assert!(!mode.setuid());

    let mut mode = Mode::from(0o40755u16);
    assert_eq!(mode.file_type(), FileType::DIR);
    assert!(mode.user_execute());
    assert!(mode.group_execute());
    assert!(mode.other_execute());
    mode.set_setuid(true);
    assert!(mode.setuid());
    assert_eq!(u16::from(mode) & 0o4000, 0o4000);

    assert_eq!(u16::from(FileType::REG), 0o10);
    assert_eq!(u16::from(FileType::DIR), 0o4);
    assert_eq!(FileType::from(0o12u16), FileType::LNK);
}

#[test]
fn fs_error_errno_and_display() {
    assert_eq!(FsError::NotFound.errno(), 2);
    assert_eq!(FsError::Io.errno(), 5);
    assert_eq!(FsError::NotDir.errno(), 20);
    assert_eq!(FsError::IsDir.errno(), 21);
    assert_eq!(FsError::Invalid.errno(), 22);
    assert_eq!(FsError::NoSpace.errno(), 28);
    assert_eq!(FsError::Unsupported.errno(), 95);

    assert_eq!(format!("{}", FsError::NotFound), "not found");
    assert_eq!(FsError::from(IoError::Request), FsError::Io);
    assert_eq!(FsError::from(IoError::NotReady), FsError::Io);
}

#[test]
fn open_flags() {
    assert!(OpenFlags::read_only().can_read());
    assert!(!OpenFlags::read_only().can_write());
    assert!(!OpenFlags::write_only().can_read());
    assert!(OpenFlags::write_only().can_write());
    assert!(OpenFlags::read_write().can_read());
    assert!(OpenFlags::read_write().can_write());

    assert_eq!(OpenFlags::read_write().accmode(), AccessMode::READ_WRITE);
    let flags = OpenFlags::builder()
        .set_create(true)
        .set_truncate(true)
        .set_append(true)
        .build();
    assert!(flags.create());
    assert!(flags.truncate());
    assert!(flags.append());
    assert!(!flags.exclusive());
    assert_eq!(u32::from(flags), (1 << 6) | (1 << 9) | (1 << 10));
}

// ===== 块设备 =====

struct MemDevice {
    data: Mutex<Vec<u8>>,
}

impl MemDevice {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data: Mutex::new(data),
        }
    }
}

impl BlockDevice for MemDevice {
    fn read_at(&self, sector: usize, buf: &mut [u8]) -> IoResult<usize> {
        let data = self.data.lock().unwrap();
        let start = sector * SECTOR_SIZE;
        let end = start + buf.len();
        if end > data.len() {
            return Err(IoError::Request);
        }
        buf.copy_from_slice(&data[start..end]);
        Ok(buf.len() / SECTOR_SIZE)
    }

    fn write_at(&self, sector: usize, buf: &[u8]) -> IoResult<usize> {
        let mut data = self.data.lock().unwrap();
        let start = sector * SECTOR_SIZE;
        let end = start + buf.len();
        if end > data.len() {
            return Err(IoError::Request);
        }
        data[start..end].copy_from_slice(buf);
        Ok(buf.len() / SECTOR_SIZE)
    }
}

#[test]
fn block_device_byte_access_across_sectors() {
    let mut image = vec![0u8; SECTOR_SIZE * 4];
    for (i, byte) in image.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
    let device = MemDevice::new(image.clone());

    let mut buffer = [0u8; 40];
    device.read_bytes_at(SECTOR_SIZE - 20, &mut buffer).unwrap();
    assert_eq!(&buffer[..], &image[SECTOR_SIZE - 20..SECTOR_SIZE + 20]);

    device
        .write_bytes_at(SECTOR_SIZE - 10, b"0123456789abcdefghij")
        .unwrap();
    let mut raw = [0u8; SECTOR_SIZE * 2];
    device.read_at(0, &mut raw).unwrap();
    assert_eq!(&raw[SECTOR_SIZE - 10..SECTOR_SIZE], b"0123456789");
    assert_eq!(&raw[SECTOR_SIZE..SECTOR_SIZE + 10], b"abcdefghij");

    // 越界读取返回设备错误。
    assert!(matches!(
        device.read_bytes_at(SECTOR_SIZE * 4 - 1, &mut buffer),
        Err(IoError::Request)
    ));
}

#[test]
fn whence_values() {
    assert_eq!(Whence::SEEK_SET.0, 0);
    assert_eq!(Whence::SEEK_CUR.0, 1);
    assert_eq!(Whence::SEEK_END.0, 2);
}

// ===== VFS =====

struct FakeOps;

impl FileOps for FakeOps {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> goethite_fs::FsResult<usize> {
        let data = b"fake file contents";
        let offset = offset as usize;
        if offset >= data.len() {
            return Ok(0);
        }
        let n = (data.len() - offset).min(buf.len());
        buf[..n].copy_from_slice(&data[offset..offset + n]);
        Ok(n)
    }

    fn seek(&self, current: u64, offset: i64, whence: Whence) -> goethite_fs::FsResult<i64> {
        let size = 18i64;
        let new = match whence {
            Whence::SEEK_SET => offset,
            Whence::SEEK_CUR => current as i64 + offset,
            Whence::SEEK_END => size + offset,
            _ => return Err(FsError::Invalid),
        };
        if new < 0 {
            return Err(FsError::Invalid);
        }
        Ok(new)
    }
}

struct FakeFs;

impl FileSystem for FakeFs {
    fn open(&self, path: &Path, flags: OpenFlags, _mode: Mode) -> goethite_fs::FsResult<File> {
        Ok(File::from_ops(
            path.file_name().unwrap_or("/"),
            42,
            Mode::from(0o100644u16),
            Arc::new(FakeOps),
            flags,
        ))
    }

    fn stat(&self, _path: &Path) -> goethite_fs::FsResult<Stat> {
        Ok(Stat {
            ino: 1,
            mode: Mode::from(0o40755u16),
            size: 0,
            nlinks: 2,
            mtime: 0,
            uid: 0,
            gid: 0,
        })
    }

    fn mkdir(&self, _: &Path, _: Mode) -> goethite_fs::FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn unlink(&self, _: &Path) -> goethite_fs::FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn rmdir(&self, _: &Path) -> goethite_fs::FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn rename(&self, _: &Path, _: &Path) -> goethite_fs::FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn link(&self, _: &Path, _: &Path) -> goethite_fs::FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn symlink(&self, _: &str, _: &Path) -> goethite_fs::FsResult<()> {
        Err(FsError::Unsupported)
    }

    fn readlink(&self, _: &Path) -> goethite_fs::FsResult<PathBuf> {
        Err(FsError::Unsupported)
    }

    fn sync(&self) -> goethite_fs::FsResult<()> {
        Ok(())
    }
}

#[test]
fn vfs_mount_and_resolve() {
    let vfs = Vfs::new();

    assert_eq!(vfs.stat(&Path::new("/x")).unwrap_err(), FsError::NotFound);
    assert_eq!(
        vfs.mount(&Path::new("relative"), Arc::new(FakeFs)),
        Err(FsError::Invalid)
    );

    vfs.mount(&Path::new("/"), Arc::new(FakeFs)).unwrap();
    assert_eq!(
        vfs.mount(&Path::new("/"), Arc::new(FakeFs)),
        Err(FsError::AlreadyExists)
    );

    let stat = vfs.stat(&Path::new("/anything")).unwrap();
    assert_eq!(stat.ino, 1);
    assert_eq!(stat.mode.file_type(), FileType::DIR);

    let file = vfs
        .open(&Path::new("/hello"), OpenFlags::read_only(), Mode::from(0))
        .unwrap();
    assert_eq!(file.stat().unwrap().ino, 42);
    assert_eq!(vfs.sync(), Ok(()));
}

#[test]
fn vfs_rename_across_mounts_is_cross_device() {
    let vfs = Vfs::new();
    vfs.mount(&Path::new("/a"), Arc::new(FakeFs)).unwrap();
    vfs.mount(&Path::new("/b"), Arc::new(FakeFs)).unwrap();

    assert_eq!(
        vfs.rename(&Path::new("/a/x"), &Path::new("/b/y")),
        Err(FsError::CrossDevice)
    );
    assert_eq!(
        vfs.rename(&Path::new("relative"), &Path::new("/b/y")),
        Err(FsError::Invalid)
    );
}

#[test]
fn vfs_file_operations_use_shared_offset() {
    let file = File::from_ops(
        "fake",
        42,
        Mode::from(0o100644u16),
        Arc::new(FakeOps),
        OpenFlags::read_only(),
    );

    let mut buffer = [0u8; 4];
    assert_eq!(file.read(&mut buffer).unwrap(), 4);
    assert_eq!(&buffer, b"fake");
    assert_eq!(file.offset(), 4);

    assert_eq!(file.seek(0, Whence::SEEK_SET).unwrap(), 0);
    assert_eq!(file.seek(2, Whence::SEEK_END).unwrap(), 20);
    assert_eq!(file.seek(-4, Whence::SEEK_CUR).unwrap(), 16);
    assert_eq!(file.seek(-100, Whence::SEEK_SET), Err(FsError::Invalid));
    assert_eq!(file.seek(0, Whence::from(99i32)), Err(FsError::Invalid));

    // 克隆的文件句柄共享同一偏移。
    let clone = file.clone();
    assert_eq!(clone.offset(), file.offset());
}
