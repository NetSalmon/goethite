//! virtio-blk 块设备驱动。
//!
//! 基于 virtio-mmio 传输层（见 [`crate::device::virtio_mmio`]）完成设备探测、
//! 特征协商与队列配置，并通过 [`Device`] 的 `read_at` / `write_at` 提供按扇区
//! 寻址的读写。探测、握手与队列收发原语复用 [`VirtioDevice`] 通用抽象，本文件
//! 只描述 blk 特有的请求格式。
use alloc::{sync::Arc, vec::Vec};
use core::{
    mem::size_of,
    ptr::{addr_of, addr_of_mut},
};

use goethite_config::{SECTOR_SIZE, VIRTIO_BLK_S_OK, VIRTIO_BLK_T_IN, VIRTIO_BLK_T_OUT};
use goethite_macros::bits;
use goethite_sync::SpinLock;

use crate::device::{
    Device, DeviceError, DeviceInfo, DeviceNaming, DeviceResult, FileType, Major,
    desc::DeviceDesc,
    driver::Driver,
    preset, virt_to_phys,
    virtio_mmio::{
        DeviceType, VirtioDevice, is_device,
        queue::{Flags, VirtQueue, VirtQueueDescriptor},
    },
};

/// virtio-blk 设备：MMIO 描述与队列 0。
pub struct VirtioBlk {
    pub device: DeviceInfo,
    pub queues: SpinLock<Option<Vec<VirtQueue>>>,
}

bits! {
    pub type Status: u32 {
        acknowledge: 0,
        driver: 1,
        driver_ok: 2,
        features_ok: 3,
        failed: 7,
    }
}

bits! {
    pub type VirtioBlkFeaturesLow: u32 {
        geometry: 4,
        readonly: 6,
        scsi: 7,
        flush: 9,
        any_layout: 11,
        write_zeroes: 14,
        blk_size: 24,
        flush_cmd: 28,
        reserved_transport: 0 => 23,
        reserved_device: 24 => 31,
    }
}

bits! {
    pub type VirtioBlkFeaturesHigh: u32 {
        version_1: 0,
        access_platform: 1,
        ring_packed: 2,
        in_order: 3,
        order_platform: 4,
        sr_iov: 5,
        notification_data: 6,
        notif_config_data: 7,
        ring_reset: 8,
    }
}

impl Device for VirtioBlk {
    /// 从第 `sector` 个扇区起读取 `buf.len()` 字节，返回读取的扇区数。
    fn read_at(&self, sector: usize, buf: &mut [u8]) -> DeviceResult {
        self.read_bytes(sector * SECTOR_SIZE, buf)?;
        Ok(buf.len().div_ceil(SECTOR_SIZE))
    }

    /// 从第 `sector` 个扇区起写入 `buf`，返回写入的扇区数。
    fn write_at(&self, sector: usize, buf: &[u8]) -> DeviceResult {
        self.write_bytes(sector * SECTOR_SIZE, buf)?;
        Ok(buf.len().div_ceil(SECTOR_SIZE))
    }
}

impl VirtioDevice for VirtioBlk {
    const DEVICE_TYPE: DeviceType = DeviceType::BLOCK;

    fn device(&self) -> DeviceInfo {
        self.device
    }

    fn queues(&self) -> &SpinLock<Option<Vec<VirtQueue>>> {
        &self.queues
    }

    /// modern 特性协商：接受 `VIRTIO_F_VERSION_1`（high 位 0）。
    fn negotiate(&self, _features_low: u32, features_high: u32) -> (u32, u32) {
        if features_high & 1 != 0 {
            (0, 1)
        } else {
            (0, 0)
        }
    }
}

impl From<DeviceInfo> for VirtioBlk {
    fn from(dev: DeviceInfo) -> Self {
        VirtioBlk {
            device: dev,
            queues: SpinLock::new(None),
        }
    }
}

impl VirtioBlk {
    /// 从设备描述探测：要求 `device_id` 为块设备。
    pub fn from_desc(desc: &DeviceDesc) -> Option<Self> {
        let device = DeviceInfo::from_desc(desc)?;
        is_device(device, Self::DEVICE_TYPE).then(|| device.into())
    }

    /// 通过 virtqueue 0 提交一次 virtio-blk 请求并等待完成。
    ///
    /// `data` 为数据缓冲区；`data_write` 为真表示设备向 `data` 写入
    /// （读请求），为假表示设备从 `data` 读取（写请求）。描述符 0/1/2
    /// 分别对应请求头、数据与状态字节，每次请求复用同一组描述符（同一
    /// 时刻最多一个在途请求）。
    fn submit_request(
        &self,
        req_type: u32,
        sector: u64,
        data: *const u8,
        data_len: usize,
        data_write: bool,
    ) -> Result<(), DeviceError> {
        let req = VirtioBlkReq {
            request_type: req_type,
            reserved: 0,
            sector,
        };
        let mut status: u8 = !VIRTIO_BLK_S_OK;

        let elem = VirtioDevice::submit(self, |q| {
            let mut flags = Flags::new();
            flags.set_next(true);
            q.desc[0] = VirtQueueDescriptor {
                addr: virt_to_phys(addr_of!(req) as usize) as u64,
                len: size_of::<VirtioBlkReq>() as u32,
                flags,
                next: 1,
            };

            let mut flags = Flags::new();
            flags.set_next(true);
            flags.set_write(data_write);
            q.desc[1] = VirtQueueDescriptor {
                addr: virt_to_phys(data as usize) as u64,
                len: data_len as u32,
                flags,
                next: 2,
            };

            let mut flags = Flags::new();
            flags.set_write(true);
            q.desc[2] = VirtQueueDescriptor {
                addr: virt_to_phys(addr_of_mut!(status) as usize) as u64,
                len: size_of::<u8>() as u32,
                flags,
                next: 0,
            };
        })
        .map_err(|_| DeviceError::NotReady)?;

        if elem.id != 0 {
            return Err(DeviceError::Io);
        }
        if unsafe { addr_of!(status).read_volatile() } != VIRTIO_BLK_S_OK {
            return Err(DeviceError::Io);
        }

        Ok(())
    }

    /// 按字节偏移读取（内部使用，支持非扇区对齐）。
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<(), DeviceError> {
        let mut done = 0;
        while done < buf.len() {
            let pos = offset + done;
            let sector = (pos / SECTOR_SIZE) as u64;
            let shift = pos % SECTOR_SIZE;

            let mut sector_buf = [0u8; SECTOR_SIZE];
            self.submit_request(
                VIRTIO_BLK_T_IN,
                sector,
                sector_buf.as_mut_ptr(),
                sector_buf.len(),
                true,
            )?;

            let n = core::cmp::min(SECTOR_SIZE - shift, buf.len() - done);
            buf[done..done + n].copy_from_slice(&sector_buf[shift..shift + n]);
            done += n;
        }
        Ok(())
    }

    /// 按字节偏移写入（内部使用，非整扇区时读-改-写）。
    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<(), DeviceError> {
        let mut done = 0;
        while done < buf.len() {
            let pos = offset + done;
            let sector = (pos / SECTOR_SIZE) as u64;
            let shift = pos % SECTOR_SIZE;
            let n = core::cmp::min(SECTOR_SIZE - shift, buf.len() - done);

            if shift == 0 && n == SECTOR_SIZE {
                self.submit_request(
                    VIRTIO_BLK_T_OUT,
                    sector,
                    buf[done..done + n].as_ptr(),
                    n,
                    false,
                )?;
            } else {
                // 非整扇区写入先读回原扇区，再原地修改（read-modify-write）。
                let mut sector_buf = [0u8; SECTOR_SIZE];
                self.submit_request(
                    VIRTIO_BLK_T_IN,
                    sector,
                    sector_buf.as_mut_ptr(),
                    sector_buf.len(),
                    true,
                )?;
                sector_buf[shift..shift + n].copy_from_slice(&buf[done..done + n]);
                self.submit_request(
                    VIRTIO_BLK_T_OUT,
                    sector,
                    sector_buf.as_ptr(),
                    sector_buf.len(),
                    false,
                )?;
            }
            done += n;
        }
        Ok(())
    }
}

/// virtio-blk 的驱动：匹配 compatible `virtio,mmio` 且 `device_id` 为块设备，
/// 登记为块设备（[`FileType::BLK`]）。
pub struct VirtioBlkDriver;

impl Driver for VirtioBlkDriver {
    fn name(&self) -> &'static str {
        "virtio-mmio-blk"
    }

    fn compatibles(&self) -> &'static [&'static str] {
        &["virtio,mmio"]
    }

    fn major(&self) -> Major {
        Major::Static(preset::BLOCK)
    }

    fn file_type(&self) -> FileType {
        FileType::BLK
    }

    fn naming(&self) -> DeviceNaming {
        DeviceNaming::Alpha("vd")
    }

    fn probe(&self, desc: &Arc<DeviceDesc>) -> Result<Arc<dyn Device>, DeviceError> {
        VirtioBlk::from_desc(desc)
            .map(|blk| Arc::new(blk) as Arc<dyn Device>)
            .ok_or(DeviceError::NotReady)
    }
}

#[repr(C)]
#[derive(Default)]
struct VirtioBlkReq {
    request_type: u32,
    reserved: u32,
    sector: u64,
}
