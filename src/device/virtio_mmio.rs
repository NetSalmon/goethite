//! virtio-mmio 传输层。
//!
//! 定义 MMIO 寄存器布局（[`VirtQueueConfig`]）、设备状态 / 类型枚举与虚拟队列
//! 结构；设备初始化流程见 [`handshake`]，队列实现见 [`queue`]。
//!
//! [`VirtioDevice`] 抽象出与具体设备类型无关的探测、特征协商与队列 0 配置，
//! 新设备驱动只需实现 [`VirtioDevice::DEVICE_TYPE`] 与 `device` / `queues`
//! 两个访问器（并按需覆写特性协商），即可复用完整的初始化与提交流程。
mod handshake;
pub(crate) mod queue;

use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};

use goethite_config::{RING_SIZE, VIRTIO_VERSION_LEGACY};
use goethite_macros::{bits, numeric};
use goethite_sync::{SpinLock, spin::SpinLockGuard};

use self::{
    handshake::QueueConfig,
    queue::{Queue, UsedElement, VirtQueue},
};
use crate::{
    device::{DeviceError, DeviceInfo},
    mmio_regs,
};

pub struct VirtQueueConfig {
    pub device: DeviceInfo,
}

/// 判断 `device` 是否为指定 virtio 设备类型（读取 `device_id` 寄存器）。
pub(crate) fn is_device(device: DeviceInfo, expected: DeviceType) -> bool {
    VirtQueueConfig { device }.device_id() == expected.0
}

numeric! {
    pub enum DeviceType : u32 {
        NET = 1,
        BLOCK = 2,
        CONSOLE = 3,
        ENTROPY_SOURCE = 4,
        MEMORY_BALLOONING = 5,
        IO_MEMORY = 6,
        RPMSG = 7,
        SCSI_HOST = 8,
        TRANSPORT = 9,
        MAC80211_WLAN = 10,
        RPROC_SERIAL = 11,
        VIRTIO_CAIF = 12,
    }
}

bits! {
    pub type DeviceStatus : u32 {
        acknowledge: 0,
        driver: 1,
        driver_ok: 2,
        features_ok: 3,
        device_needs_reset: 6,
        failed: 7,
    }
}

numeric! {
    pub enum VirtQueueVersion : u32 {
        LEGACY = 1,
        MODERN = 2,
    }
}

// 生成 magic_value() / write_magic_value() 等寄存器访问方法。
mmio_regs! {
    VirtQueueConfig: [
        magic_value: u32 => 0x000,
        version: u32 => 0x004,
        device_id: u32 => 0x008,
        vendor_id: u32 => 0x00C,
        device_features: u32 => 0x010,
        device_features_sel: u32 => 0x014,
        driver_features: u32 => 0x020,
        driver_features_sel: u32 => 0x024,
        queue_sel: u32 => 0x030,
        queue_num_max: u32 => 0x034,
        queue_num: u32 => 0x038,
        queue_align: u32 => 0x03C,   // legacy
        queue_pfn: u32 => 0x040,     // legacy
        queue_ready: u32 => 0x044,
        queue_notify: u32 => 0x050,
        guest_page_size: u32 => 0x028,
        interrupt_status: u32 => 0x060,
        interrupt_ack: u32 => 0x064,
        status: u32 => 0x070,
        queue_desc_low: u32 => 0x080,
        queue_desc_high: u32 => 0x084,
        queue_driver_low: u32 => 0x090,
        queue_driver_high: u32 => 0x094,
        queue_device_low: u32 => 0x0A0,
        queue_device_high: u32 => 0x0A4,
        config_generation: u32 => 0x0FC,
    ]
}

/// virtio-mmio 设备抽象。
///
/// 提供与具体设备类型无关的握手（[`Self::handshake`]）、队列 0 的收发原语
/// （[`Self::submit`]）以及通知（[`Self::notify`]）。新设备驱动只需实现
/// [`Self::DEVICE_TYPE`] 与 `device` / `queues` 两个访问器；需要多条队列时
/// 覆写 [`Self::queue_count`]，并在 [`Self::prepare_queues`] 里于 `DRIVER_OK`
/// 之前备好接收缓冲。
pub trait VirtioDevice: Sized {
    /// 设备类型（virtio 规范编号），探测时按此匹配 `device_id` 寄存器。
    const DEVICE_TYPE: DeviceType;

    /// 返回设备的 MMIO 描述。
    fn device(&self) -> DeviceInfo;

    /// 已配置虚拟队列的可变访问（未握手时为 `None`）。
    fn queues(&self) -> &SpinLock<Option<Vec<VirtQueue>>>;

    /// modern 特性协商：输入设备特性（低 / 高 32 位），返回驱动协商值。
    ///
    /// 默认不协商任何特性（`(0, 0)`）。
    fn negotiate(&self, _features_low: u32, _features_high: u32) -> (u32, u32) {
        (0, 0)
    }

    /// legacy 特性协商：输入设备特性，返回驱动协商值。默认 `0`。
    fn negotiate_legacy(&self, _features: u32) -> u32 {
        0
    }

    /// 要配置的 virtqueue 个数。默认只有队列 0；网卡等设备覆写为收发两队列。
    fn queue_count(&self) -> u32 {
        1
    }

    /// 在置 `DRIVER_OK` 之前调整已配置的队列。
    ///
    /// 网卡在这一步把接收缓冲放进 avail 环：设备从 `DRIVER_OK` 起就可以往这些
    /// 缓冲里写包。
    fn prepare_queues(&self, _queues: &mut [VirtQueue]) -> Result<(), DeviceError> {
        Ok(())
    }

    /// `DRIVER_OK` 已经置位、队列已经保存后的收尾（例如通知接收队列）。
    fn activate(&self) -> Result<(), DeviceError> {
        Ok(())
    }

    /// 完成 ACK / DRIVER 状态、特性协商与队列配置的完整握手。
    ///
    /// 队列尚未配置时，[`Self::submit`] 会自动补一次握手。已经握过手则直接返回。
    fn handshake(&self) -> Result<(), DeviceError> {
        if self.queues().lock().is_some() {
            return Ok(());
        }

        let mut cfg = VirtQueueConfig {
            device: self.device(),
        };
        let is_modern = cfg.version() != VIRTIO_VERSION_LEGACY;
        let queue_count = self.queue_count();
        if queue_count == 0 {
            return Err(DeviceError::NotReady);
        }
        let configs: Vec<QueueConfig> = (0..queue_count)
            .map(|index| QueueConfig {
                index,
                size: RING_SIZE as u32,
            })
            .collect();

        let mut ready = if is_modern {
            cfg.handshake()
                .modern(|low, high| self.negotiate(low, high))?
                .setup_queues(&configs)?
        } else {
            cfg.handshake()
                .legacy(|features| self.negotiate_legacy(features))?
                .setup_queues(&configs)?
        };

        self.prepare_queues(ready.queues_mut())?;
        let queues = ready.finish();
        *self.queues().lock() = Some(queues);
        self.activate()
    }

    /// 队列 0 的加锁访问；未配置时先补一次握手。
    fn queue(&self) -> Result<SpinLockGuard<'_, Option<Vec<VirtQueue>>>, DeviceError> {
        if self.queues().lock().is_none() {
            self.handshake()?;
        }
        let queues = self.queues().lock();
        if queues.as_ref().and_then(|queues| queues.first()).is_some() {
            Ok(queues)
        } else {
            Err(DeviceError::NotReady)
        }
    }

    /// 提交一个队列请求并等待设备完成。
    ///
    /// 描述符配置、发布、通知和等待必须由同一个队列锁保护。virtio 驱动
    /// 只负责填写协议相关的描述符，避免各驱动重复实现容易出错的锁边界。
    fn submit<F>(&self, configure: F) -> Result<UsedElement, DeviceError>
    where
        F: FnOnce(&mut Queue),
    {
        let mut queues = self.queue()?;
        let queue = queues
            .as_mut()
            .and_then(|queues| queues.first_mut())
            .ok_or(DeviceError::NotReady)?;

        configure(queue.as_mut());
        let last_used = queue.post_avail(0);
        self.notify();
        queue.wait_used(last_used)
    }

    /// 通知设备处理队列 `index`。
    fn notify_queue(&self, index: u32) {
        // `QUEUE_NOTIFY_OFFSET` 由 `mmio_regs!` 根据 queue_notify 寄存器生成。
        self.device().mmio.write::<u32>(QUEUE_NOTIFY_OFFSET, index);
        fence(Ordering::SeqCst);
    }

    /// 通知设备处理队列 0。
    fn notify(&self) {
        self.notify_queue(0);
    }
}
