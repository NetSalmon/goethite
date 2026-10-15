//! virtio-rng 熵源设备驱动。
//!
//! 基于 virtio-mmio 传输层（见 [`crate::device::virtio_mmio`]）完成设备探测、
//! 特征协商与队列配置，并通过 [`Device::read`] 提供真随机字节。
//!
//! 协议：virtio-rng 只有一个 virtqueue（索引 0），请求就是一个可写缓冲区——
//! 设备把随机字节写入缓冲区后放回 used 环，used 元素长度即实际写入的字节数。
use alloc::{sync::Arc, vec::Vec};

use goethite_sync::SpinLock;

use crate::device::{
    Device, DeviceError, DeviceInfo, DeviceResult, FileType, Major, Mode,
    desc::DeviceDesc,
    driver::Driver,
    preset, virt_to_phys,
    virtio_mmio::{
        DeviceType, VirtioDevice, is_device,
        queue::{Flags, VirtQueue, VirtQueueDescriptor},
    },
};

/// virtio-rng 设备：MMIO 描述与队列 0。
pub struct VirtioRng {
    pub device: DeviceInfo,
    pub queues: SpinLock<Option<Vec<VirtQueue>>>,
}

impl Device for VirtioRng {
    /// 用随机字节填充 `buf`，返回写入的字节数。
    fn read(&self, buf: &mut [u8]) -> DeviceResult {
        let mut done = 0;
        while done < buf.len() {
            let written = self.request(&mut buf[done..])?;
            // 设备写入量不应超过请求长度，防御性截断。
            let written = written.min(buf.len() - done);
            if written == 0 {
                return Err(DeviceError::Io);
            }
            done += written;
        }
        Ok(done)
    }
}

impl VirtioDevice for VirtioRng {
    const DEVICE_TYPE: DeviceType = DeviceType::ENTROPY_SOURCE;

    fn device(&self) -> DeviceInfo {
        self.device
    }

    fn queues(&self) -> &SpinLock<Option<Vec<VirtQueue>>> {
        &self.queues
    }
}

impl From<DeviceInfo> for VirtioRng {
    fn from(dev: DeviceInfo) -> Self {
        VirtioRng {
            device: dev,
            queues: SpinLock::new(None),
        }
    }
}

impl VirtioRng {
    /// 从设备描述探测：要求 `device_id` 为熵源。
    pub fn from_desc(desc: &DeviceDesc) -> Option<Self> {
        let device = DeviceInfo::from_desc(desc)?;
        is_device(device, Self::DEVICE_TYPE).then(|| device.into())
    }

    /// 通过 virtqueue 0 请求一次熵数据，返回设备实际写入的字节数。
    ///
    /// 单个可写描述符（设备写入 `buf`），提交后轮询等待 used 元素。描述符 0
    /// 每次复用，同一时刻最多一个在途请求。
    fn request(&self, buf: &mut [u8]) -> Result<usize, DeviceError> {
        let elem = VirtioDevice::submit(self, |q| {
            let mut flags = Flags::new();
            flags.set_write(true);
            q.desc[0] = VirtQueueDescriptor {
                addr: virt_to_phys(buf.as_mut_ptr() as usize) as u64,
                len: buf.len() as u32,
                flags,
                next: 0,
            };
        })?;
        if elem.id != 0 {
            return Err(DeviceError::Io);
        }
        Ok(elem.len as usize)
    }
}

/// virtio-rng 的驱动：匹配 compatible `virtio,mmio` 且 `device_id` 为熵源，
/// 登记为只读字符设备。
pub struct VirtioRngDriver;

impl Driver for VirtioRngDriver {
    fn name(&self) -> &'static str {
        "virtio-mmio-rng"
    }

    fn compatibles(&self) -> &'static [&'static str] {
        &["virtio,mmio"]
    }

    fn major(&self) -> Major {
        Major::Static(preset::MISC)
    }

    fn file_type(&self) -> FileType {
        FileType::CHR
    }

    /// 熵源只读（属主 / 属组 / 其他均可读）。
    fn mode(&self) -> Mode {
        Mode::builder()
            .set_file_type(self.file_type())
            .set_user_read(true)
            .set_group_read(true)
            .set_other_read(true)
            .build()
    }

    fn probe(&self, desc: &Arc<DeviceDesc>) -> Result<Arc<dyn Device>, DeviceError> {
        VirtioRng::from_desc(desc)
            .map(|rng| Arc::new(rng) as Arc<dyn Device>)
            .ok_or(DeviceError::NotReady)
    }
}
