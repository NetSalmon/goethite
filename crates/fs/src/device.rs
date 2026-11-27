//! 块设备抽象与统一的 I/O 结果类型。
//!
//! [`BlockDevice`] 以扇区（sector）为单位寻址，与内核 `Device::read_at` /
//! `Device::write_at` 的语义一致；文件系统内部按字节偏移访问时，使用
//! [`BlockDevice::read_bytes_at`] / [`BlockDevice::write_bytes_at`]（默认实现
//! 负责扇区与字节偏移之间的换算，并对不足一个扇区的读写做读-改-写）。
//!
//! 读写统一返回 [`IoResult`]，错误类型 [`IoError`] 自持，不依赖具体驱动。

#![allow(dead_code)]

use alloc::vec;

use goethite_macros::numeric;

/// 一个扇区的字节数（与内核块设备的逻辑扇区大小一致）。
pub const SECTOR_SIZE: usize = 512;

/// I/O 错误：设备读写失败的统一错误类型。
#[derive(Debug, Clone, thiserror::Error)]
pub enum IoError {
    /// 设备请求执行失败（如 virtio-blk 请求返回错误状态）。
    #[error("device request failed")]
    Request,
    /// 设备尚未就绪（握手或特性协商失败）。
    #[error("device not ready")]
    NotReady,
}

/// 设备层统一结果类型。进入 VFS 后通过 `From<IoError>` 映射为 `FsError::Io`。
pub type IoResult<T> = core::result::Result<T, IoError>;

/// 块设备：以扇区为单位随机读写。
///
/// 需要 `Send + Sync`，因为设备通常以共享句柄（如 `Arc<RwLock<T>>`）被多个
/// 打开的文件复用。`read_at` / `write_at` 的返回值是本次实际读写的**扇区数**，
/// 与内核 `Device` trait 一致。
pub trait BlockDevice: Send + Sync {
    /// 从第 `sector` 个扇区开始读取数据到 `buf`，返回实际读取的扇区数。
    fn read_at(&self, sector: usize, buf: &mut [u8]) -> IoResult<usize>;

    /// 把 `buf` 的数据写入第 `sector` 个扇区，返回实际写入的扇区数。
    fn write_at(&self, sector: usize, buf: &[u8]) -> IoResult<usize>;

    /// 按字节偏移 `offset` 读取 `buf.len()` 字节。
    ///
    /// 落在扇区边界内的部分通过临时扇区缓冲读取后截取，因此支持任意字节
    /// 偏移（如 32 字节的 MINIX inode）。
    fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> IoResult<()> {
        let mut done = 0;
        while done < buf.len() {
            let pos = offset + done;
            let sector = pos / SECTOR_SIZE;
            let in_sector = pos % SECTOR_SIZE;
            let n = (SECTOR_SIZE - in_sector).min(buf.len() - done);

            if in_sector == 0 && n == SECTOR_SIZE {
                self.read_at(sector, &mut buf[done..done + n])?;
            } else {
                let mut tmp = vec![0u8; SECTOR_SIZE];
                self.read_at(sector, &mut tmp)?;
                buf[done..done + n].copy_from_slice(&tmp[in_sector..in_sector + n]);
            }
            done += n;
        }
        Ok(())
    }

    /// 按字节偏移 `offset` 写入 `buf`。
    ///
    /// 不足一个扇区的部分采用读-改-写，因此支持任意字节偏移（如 32 字节的
    /// MINIX inode）。
    fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> IoResult<()> {
        let mut done = 0;
        while done < buf.len() {
            let pos = offset + done;
            let sector = pos / SECTOR_SIZE;
            let in_sector = pos % SECTOR_SIZE;
            let n = (SECTOR_SIZE - in_sector).min(buf.len() - done);

            if in_sector == 0 && n == SECTOR_SIZE {
                self.write_at(sector, &buf[done..done + n])?;
            } else {
                let mut tmp = vec![0u8; SECTOR_SIZE];
                self.read_at(sector, &mut tmp)?;
                tmp[in_sector..in_sector + n].copy_from_slice(&buf[done..done + n]);
                self.write_at(sector, &tmp)?;
            }
            done += n;
        }
        Ok(())
    }
}

numeric! {
    pub enum Whence : i32 {
        SEEK_SET = 0,
        SEEK_CUR = 1,
        SEEK_END = 2,
    }
}
