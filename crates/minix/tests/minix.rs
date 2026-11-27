//! 用内存中的最小 MINIX v1 镜像验证 `MinixFs` 的读取、写入与目录操作。

use std::sync::{Arc, Mutex};

use goethite_fs::{BlockDevice, IoError, IoResult, SECTOR_SIZE, Whence, path::Path};
use goethite_minix::{DiskInode, DiskSuperBlock, MinixFs, MinixFsMagic};
use goethite_sync::RwLock;

const ZONE: usize = 1024;
const ZONES: usize = 16;
const IMAGE_SIZE: usize = ZONE * ZONES;
const SUPERBLOCK_OFFSET: usize = 1024;
const IMAP_OFFSET: usize = 2 * ZONE;
const ZMAP_OFFSET: usize = 3 * ZONE;
const INODE_TABLE_OFFSET: usize = 4 * ZONE;
const ROOT_DIR_ZONE: usize = 5;
const HELLO_ZONE: usize = 6;
const HELLO_DATA: &[u8] = b"hello, minix\n";

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

fn write_bytes<T>(image: &mut [u8], offset: usize, value: &T) {
    // SAFETY: 测试里只写入 `#[repr(C)]` 的纯数据，长度由类型决定。
    let bytes =
        unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) };
    image[offset..offset + bytes.len()].copy_from_slice(bytes);
}

fn write_inode(image: &mut [u8], ino: u16, inode: &DiskInode) {
    let offset = INODE_TABLE_OFFSET + (ino as usize - 1) * size_of::<DiskInode>();
    write_bytes(image, offset, inode);
}

fn write_dir_entry(
    image: &mut [u8],
    zone: usize,
    slot: usize,
    entry_size: usize,
    ino: u16,
    name: &str,
) {
    let offset = zone * ZONE + slot * entry_size;
    image[offset..offset + 2].copy_from_slice(&ino.to_le_bytes());
    let name = name.as_bytes();
    assert!(name.len() < entry_size - 2);
    image[offset + 2..offset + 2 + name.len()].copy_from_slice(name);
}

fn build_image(magic: MinixFsMagic, name_len: usize) -> Vec<u8> {
    let entry_size = 2 + name_len;
    let mut image = vec![0u8; IMAGE_SIZE];

    let superblock = DiskSuperBlock {
        ninodes: 16,
        nzones: ZONES as u16,
        imap_blocks: 1,
        zmap_blocks: 1,
        first_data_zone: 5,
        log_zone_size: 0,
        max_size: 0x100_0000,
        magic,
        state: 0,
    };
    write_bytes(&mut image, SUPERBLOCK_OFFSET, &superblock);

    // inode 位图：bit 0 保留，bit 1 / bit 2 分别对应 inode 1 / 2。
    image[IMAP_OFFSET] = 0b0000_0110;
    // zone 位图：bit 0 → zone 4（inode 表），bit 1 / 2 → zone 5 / 6。
    image[ZMAP_OFFSET] = 0b0000_0111;

    let root = DiskInode {
        mode: ((0o4u16) << 12 | 0o755).into(),
        uid: 0,
        size: (3 * entry_size) as u32,
        mtime: 0,
        gid: 0,
        nlinks: 2,
        zone: [ROOT_DIR_ZONE as u16, 0, 0, 0, 0, 0, 0, 0, 0],
    };
    write_inode(&mut image, 1, &root);

    let hello = DiskInode {
        mode: ((0o10u16) << 12 | 0o644).into(),
        uid: 0,
        size: HELLO_DATA.len() as u32,
        mtime: 0,
        gid: 0,
        nlinks: 1,
        zone: [HELLO_ZONE as u16, 0, 0, 0, 0, 0, 0, 0, 0],
    };
    write_inode(&mut image, 2, &hello);

    write_dir_entry(&mut image, ROOT_DIR_ZONE, 0, entry_size, 1, ".");
    write_dir_entry(&mut image, ROOT_DIR_ZONE, 1, entry_size, 1, "..");
    write_dir_entry(&mut image, ROOT_DIR_ZONE, 2, entry_size, 2, "hello");

    image[HELLO_ZONE * ZONE..HELLO_ZONE * ZONE + HELLO_DATA.len()].copy_from_slice(HELLO_DATA);

    image
}

fn mount(image: Vec<u8>) -> (Arc<RwLock<MemDevice>>, MinixFs<MemDevice>) {
    let device = Arc::new(RwLock::new(MemDevice::new(image)));
    let fs = MinixFs::from_device(&device)
        .expect("device read should succeed")
        .expect("valid MINIX v1 image");
    (device, fs)
}

#[test]
fn recognizes_minix_magic() {
    for magic in [MinixFsMagic::V1_14, MinixFsMagic::V1_30] {
        let image = build_image(magic, if magic == MinixFsMagic::V1_30 { 30 } else { 14 });
        let (_device, fs) = mount(image);
        let file = fs
            .open(&Path::new("/hello"))
            .unwrap()
            .expect("/hello exists");
        assert_eq!(file.size(), HELLO_DATA.len() as u32);
    }
}

#[test]
fn rejects_non_minix_and_short_devices() {
    let mut image = build_image(MinixFsMagic::V1_14, 14);
    image[SUPERBLOCK_OFFSET + 16..SUPERBLOCK_OFFSET + 18].fill(0);
    let device = Arc::new(RwLock::new(MemDevice::new(image)));
    assert!(MinixFs::from_device(&device).unwrap().is_none());

    let short = vec![0u8; SUPERBLOCK_OFFSET + 4];
    let device = Arc::new(RwLock::new(MemDevice::new(short)));
    assert!(MinixFs::from_device(&device).is_err());
}

#[test]
fn opens_and_reads_files() {
    let (_device, fs) = mount(build_image(MinixFsMagic::V1_14, 14));

    let mut file = fs
        .open(&Path::new("/hello"))
        .unwrap()
        .expect("/hello exists");
    assert_eq!(file.ino(), 2);
    assert_eq!(file.size(), HELLO_DATA.len() as u32);
    assert_eq!(file.name(), Some("hello"));
    assert_eq!(file.file_type().0, 0o10);
    assert_eq!(u16::from(file.mode()) & 0o777, 0o644);

    let mut buffer = [0u8; 64];
    assert_eq!(file.read(&mut buffer).unwrap(), HELLO_DATA.len());
    assert_eq!(&buffer[..HELLO_DATA.len()], HELLO_DATA);
    // 读到末尾后继续读返回 0。
    assert_eq!(file.read(&mut buffer).unwrap(), 0);

    assert_eq!(file.seek(7, Whence::SEEK_SET).unwrap(), 7);
    let mut part = [0u8; 16];
    let n = file.read(&mut part).unwrap();
    assert_eq!(&part[..n], b"minix\n");

    assert_eq!(file.seek(-6, Whence::SEEK_END).unwrap(), 7);
    assert_eq!(file.offset(), 7);
    // seek 超过末尾会被钳制。
    assert_eq!(
        file.seek(1000, Whence::SEEK_SET).unwrap(),
        HELLO_DATA.len() as u64
    );

    let root = fs.open(&Path::new("/")).unwrap().expect("root exists");
    assert_eq!(root.ino(), 1);
    assert_eq!(root.file_type().0, 0o4);

    assert!(fs.open(&Path::new("/missing")).unwrap().is_none());
    assert!(fs.open(&Path::new("/hello/child")).unwrap().is_none());
    assert!(fs.open(&Path::new("/hello/../hello")).unwrap().is_none());
}

#[test]
fn lists_directory_entries() {
    let (device, fs) = mount(build_image(MinixFsMagic::V1_14, 14));
    let device = device.read();
    let root = fs.read_inode(1, &device).unwrap();
    assert_eq!(root.mode.file_type().0, 0o4);

    let names: Vec<_> = fs
        .dir_entries(&root, &device)
        .unwrap()
        .into_iter()
        .map(|entry| (entry.ino, entry.name))
        .collect();
    assert_eq!(
        names,
        vec![
            (1, ".".to_string()),
            (1, "..".to_string()),
            (2, "hello".to_string()),
        ]
    );

    let count = fs
        .dir_entries_iter(&root, &device)
        .unwrap()
        .filter_map(Result::ok)
        .count();
    assert_eq!(count, 3);
}

#[test]
fn writes_extend_files_and_allocate_zones() {
    let (_device, fs) = mount(build_image(MinixFsMagic::V1_14, 14));

    let path = Path::new("/hello").to_path_buf();
    let mut file = fs.open(&path).unwrap().unwrap();
    assert_eq!(file.write_at(b"xyz", HELLO_DATA.len()).unwrap(), 3);
    assert_eq!(file.size(), 16);
    assert_eq!(file.offset(), 0, "write_at 不改变读写位置");

    file.seek(0, Whence::SEEK_SET).unwrap();
    let mut contents = [0u8; 32];
    let n = file.read(&mut contents).unwrap();
    assert_eq!(&contents[..n], b"hello, minix\nxyz");
    drop(file);

    // 重新打开能看到写回磁盘的大小与内容。
    let mut reopened = fs.open(&path).unwrap().unwrap();
    assert_eq!(reopened.size(), 16);
    let mut contents = [0u8; 32];
    let n = reopened.read(&mut contents).unwrap();
    assert_eq!(&contents[..n], b"hello, minix\nxyz");
}

#[test]
fn creates_files_and_extends_directory() {
    let (device, fs) = mount(build_image(MinixFsMagic::V1_14, 14));

    let ino = fs.create_file(1, "new").unwrap().expect("space available");
    assert_eq!(ino, 3);

    let mut file = fs.open(&Path::new("/new")).unwrap().unwrap();
    assert_eq!(file.ino(), ino);
    assert_eq!(file.size(), 0);
    assert_eq!(file.write(b"abc").unwrap(), 3);
    file.seek(0, Whence::SEEK_SET).unwrap();
    let mut buffer = [0u8; 8];
    let n = file.read(&mut buffer).unwrap();
    assert_eq!(&buffer[..n], b"abc");

    let device = device.read();
    let root = fs.read_inode(1, &device).unwrap();
    let names: Vec<_> = fs
        .dir_entries(&root, &device)
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert!(names.contains(&"new".to_string()));
}

#[test]
fn unlink_removes_file_and_frees_inode() {
    let (device, fs) = mount(build_image(MinixFsMagic::V1_14, 14));

    assert!(fs.unlink(1, "hello").unwrap());
    assert!(!fs.unlink(1, "hello").unwrap());
    assert!(fs.open(&Path::new("/hello")).unwrap().is_none());

    let device = device.read();
    let inode = fs.read_inode(2, &device).unwrap();
    assert_eq!(inode.nlinks, 0);
    assert_eq!(inode.size, 0);
}

#[test]
fn supports_30_byte_names() {
    let (_device, fs) = mount(build_image(MinixFsMagic::V1_30, 30));

    let name = "a-very-long-file-name-12345678";
    assert_eq!(name.len(), 30);
    let ino = fs.create_file(1, name).unwrap().expect("space available");

    let path = format!("/{name}");
    let mut file = fs.open(&Path::new(path)).unwrap().unwrap();
    assert_eq!(file.ino(), ino);
    assert_eq!(file.write(b"long").unwrap(), 4);
}
