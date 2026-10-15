//! virtio 链式握手状态机。
//!
//! 把初始化过程拆成 `HandshakeBegin -> FeaturesNegotiated -> QueuesReady`
//! 三个阶段，每个阶段的方法名即下一步动作，保证寄存器操作顺序正确。
use alloc::vec::Vec;

use goethite_config::VIRTIO_VERSION_LEGACY;
use goethite_riscv64::addr::PhysicalAddr;

use crate::device::{
    DeviceError,
    virtio_mmio::{DeviceStatus, VirtQueueConfig, queue::VirtQueue},
};

/// 单个队列的配置：索引与期望大小。
pub struct QueueConfig {
    pub index: u32,
    pub size: u32,
}

#[must_use]
pub struct HandshakeBegin<'a> {
    cfg: &'a mut VirtQueueConfig,
}

#[must_use]
pub struct FeaturesNegotiated<'a> {
    cfg: &'a mut VirtQueueConfig,
}

#[must_use]
pub struct QueuesReady<'a> {
    cfg: &'a mut VirtQueueConfig,
    queues: Vec<VirtQueue>,
}

impl VirtQueueConfig {
    /// 复位设备并依次置位 ACKNOWLEDGE / DRIVER，进入特性协商阶段。
    pub fn handshake(&mut self) -> HandshakeBegin<'_> {
        let mut status: DeviceStatus = 0.into();
        self.write_status(status.into());

        status.set_acknowledge(true);
        self.write_status(status.into());

        status.set_driver(true);
        self.write_status(status.into());

        HandshakeBegin { cfg: self }
    }
}

impl<'a> HandshakeBegin<'a> {
    /// legacy（v1）特性协商：读取设备特性，由 `negotiate` 决定驱动接受哪些。
    pub fn legacy(
        self,
        negotiate: impl FnOnce(u32) -> u32,
    ) -> Result<FeaturesNegotiated<'a>, DeviceError> {
        self.cfg.write_device_features_sel(0);
        let features: u32 = self.cfg.device_features();

        let negotiated = negotiate(features);

        self.cfg.write_driver_features_sel(0);
        self.cfg.write_driver_features(negotiated);

        Self::commit_features(self.cfg)
    }

    /// modern（v2）特性协商：分别读取低 / 高 32 位设备特性并回写驱动特性。
    pub fn modern(
        self,
        negotiate: impl FnOnce(u32, u32) -> (u32, u32),
    ) -> Result<FeaturesNegotiated<'a>, DeviceError> {
        self.cfg.write_device_features_sel(0);
        let features_low: u32 = self.cfg.device_features();

        self.cfg.write_device_features_sel(1);
        let features_high: u32 = self.cfg.device_features();

        let (negotiated_low, negotiated_high) = negotiate(features_low, features_high);

        self.cfg.write_driver_features_sel(0);
        self.cfg.write_driver_features(negotiated_low);

        self.cfg.write_driver_features_sel(1);
        self.cfg.write_driver_features(negotiated_high);

        Self::commit_features(self.cfg)
    }

    fn commit_features(cfg: &mut VirtQueueConfig) -> Result<FeaturesNegotiated<'_>, DeviceError> {
        let mut status: DeviceStatus = cfg.status().into();
        status.set_features_ok(true);
        cfg.write_status(status.into());

        let got_status: DeviceStatus = cfg.status().into();
        if !got_status.features_ok() {
            let mut failed = got_status;
            failed.set_failed(true);
            cfg.write_status(failed.into());
            return Err(DeviceError::NotReady);
        }

        Ok(FeaturesNegotiated { cfg })
    }
}

impl<'a> FeaturesNegotiated<'a> {
    /// 配置单个队列。
    pub fn setup_queue(self, config: QueueConfig) -> Result<QueuesReady<'a>, DeviceError> {
        self.setup_queues(&[config])
    }

    /// 配置一组队列。
    pub fn setup_queues(self, configs: &[QueueConfig]) -> Result<QueuesReady<'a>, DeviceError> {
        let version = self.cfg.version();
        let mut queues = Vec::new();

        for qcfg in configs {
            self.cfg.write_queue_sel(qcfg.index);

            let num_max = self.cfg.queue_num_max();
            if qcfg.size > num_max {
                return Err(DeviceError::NotReady);
            }
            self.cfg.write_queue_num(qcfg.size);

            let virtq = VirtQueue::new()?;

            if version == VIRTIO_VERSION_LEGACY {
                self.cfg.write_guest_page_size(4096);
                // used 环按页对齐，与 Queue 布局保持一致。
                self.cfg.write_queue_align(4096);

                let pa = PhysicalAddr::from(virtq.address() as usize);
                self.cfg.write_queue_pfn(pa.ppn() as u32);
            } else {
                self.cfg.write_queue_desc_low(virtq.desc_addr() as u32);
                self.cfg
                    .write_queue_desc_high((virtq.desc_addr() >> 32) as u32);
                self.cfg.write_queue_driver_low(virtq.avail_addr() as u32);
                self.cfg
                    .write_queue_driver_high((virtq.avail_addr() >> 32) as u32);
                self.cfg.write_queue_device_low(virtq.used_addr() as u32);
                self.cfg
                    .write_queue_device_high((virtq.used_addr() >> 32) as u32);
                self.cfg.write_queue_ready(1);
            }

            queues.push(virtq);
        }

        Ok(QueuesReady {
            cfg: self.cfg,
            queues,
        })
    }
}

impl<'a> QueuesReady<'a> {
    /// 追加配置单个队列。
    pub fn setup_queue(mut self, config: QueueConfig) -> Result<Self, DeviceError> {
        let configs = [config];
        self.add_queues(&configs)?;
        Ok(self)
    }

    /// 追加配置一组队列。
    pub fn setup_queues(mut self, configs: &[QueueConfig]) -> Result<Self, DeviceError> {
        self.add_queues(configs)?;
        Ok(self)
    }

    fn add_queues(&mut self, configs: &[QueueConfig]) -> Result<(), DeviceError> {
        let version = self.cfg.version();

        for qcfg in configs {
            self.cfg.write_queue_sel(qcfg.index);

            let num_max = self.cfg.queue_num_max();
            if qcfg.size > num_max {
                return Err(DeviceError::NotReady);
            }
            self.cfg.write_queue_num(qcfg.size);

            let virtq = VirtQueue::new()?;

            if version == VIRTIO_VERSION_LEGACY {
                self.cfg.write_guest_page_size(4096);
                // used 环按页对齐，与 Queue 布局保持一致。
                self.cfg.write_queue_align(4096);

                let pa = PhysicalAddr::from(virtq.address() as usize);
                self.cfg.write_queue_pfn(pa.ppn() as u32);
            } else {
                self.cfg.write_queue_desc_low(virtq.desc_addr() as u32);
                self.cfg
                    .write_queue_desc_high((virtq.desc_addr() >> 32) as u32);
                self.cfg.write_queue_driver_low(virtq.avail_addr() as u32);
                self.cfg
                    .write_queue_driver_high((virtq.avail_addr() >> 32) as u32);
                self.cfg.write_queue_device_low(virtq.used_addr() as u32);
                self.cfg
                    .write_queue_device_high((virtq.used_addr() >> 32) as u32);
                self.cfg.write_queue_ready(1);
            }

            self.queues.push(virtq);
        }

        Ok(())
    }

    /// 置位 DRIVER_OK 并交出已配置的队列。
    pub fn finish(self) -> Vec<VirtQueue> {
        let mut status: DeviceStatus = self.cfg.status().into();
        status.set_driver_ok(true);
        self.cfg.write_status(status.into());
        self.queues
    }
}
