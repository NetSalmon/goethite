//! 设备驱动注册表与设备-驱动双向匹配。
//!
//! [`DRIVERS`] 持有所有已注册的驱动器（[`RegisteredDriver`]）。[`Driver`]
//! 通过 compatible 串与设备描述（[`DeviceDesc`]）做**双向匹配**：
//!
//! - 设备 → 驱动：[`find_driver`] 为一份设备描述挑选优先级最高的驱动；
//! - 驱动 → 设备：[`match_devices`] 列出某驱动可处理的所有设备描述。
//!
//! 驱动在 [`register`] 时通过 [`Driver::major`] 申请主设备号（预设静态值或
//! 动态分配）；[`probe_all`] 遍历设备树中的全部设备，匹配驱动并探测（probe），
//! 把探测成功的设备登记进 [`DEVICE_MANAGER`](crate::device::DEVICE_MANAGER)
//! 中对应主设备号下的从设备号。
use alloc::{sync::Arc, vec::Vec};

use goethite_macros::lazy;

use crate::device::{
    DEVICE_MANAGER, Device, DeviceError, FileType, Major, Mode,
    desc::{DEVICE_DESC_MANAGER, DeviceDesc, RankedDesc},
};

/// 设备驱动：负责与设备描述匹配，并把匹配到的硬件探测（probe）为 [`Device`] 实例。
///
/// 需要 `Send + Sync`，因为驱动以 [`Arc`] 形式存放于全局 [`DRIVERS`]。
pub trait Driver: Send + Sync {
    /// 驱动名（用于日志 / 调试）。
    fn name(&self) -> &'static str;

    /// 该驱动支持的 compatible 串（从最具体到最通用）。
    fn compatibles(&self) -> &'static [&'static str];

    /// 该驱动申请主设备号的方式（默认动态分配）。
    ///
    /// 在 [`register`] 时解析一次，之后该驱动的所有设备共享同一主设备号、
    /// 各分配不同的从设备号。
    fn major(&self) -> Major {
        Major::Dynamic
    }

    /// 该驱动对设备描述 `desc` 的匹配优先级；`None` 表示不匹配。
    ///
    /// 设备 `compatible` 列表越靠前（越具体）的串被匹配到，rank 越大；
    /// 默认实现基于 [`compatibles`](Self::compatibles)。
    fn rank(&self, desc: &DeviceDesc) -> Option<usize> {
        let total = desc.compatible.len();
        desc.compatible
            .iter()
            .enumerate()
            .filter_map(|(index, compatible)| {
                self.compatibles()
                    .iter()
                    .any(|supported| *supported == compatible.as_str())
                    .then_some(total - index)
            })
            .max()
    }

    /// 该驱动是否能处理 `desc`。默认由 [`rank`](Self::rank) 派生。
    fn matches(&self, desc: &DeviceDesc) -> bool {
        self.rank(desc).is_some()
    }

    /// 该驱动所提供设备的文件类型（块设备 / 字符设备等）。
    fn file_type(&self) -> FileType {
        FileType::CHR
    }

    /// 该驱动所提供设备的默认访问权限位（默认属主 / 属组 / 其他均可读写）。
    fn mode(&self) -> Mode {
        Mode::builder()
            .set_file_type(self.file_type())
            .set_user_read(true)
            .set_user_write(true)
            .set_group_read(true)
            .set_group_write(true)
            .set_other_read(true)
            .set_other_write(true)
            .build()
    }

    /// 探测 / 绑定 `desc` 描述的硬件，成功时返回设备实例。
    fn probe(&self, desc: &Arc<DeviceDesc>) -> Result<Arc<dyn Device>, DeviceError>;

    /// 解绑设备实例（默认无操作）。
    fn remove(&self, _device: &Arc<dyn Device>) {}
}

/// 一个已注册的驱动及其申请到的主设备号。
#[derive(Clone)]
pub struct RegisteredDriver {
    /// 驱动实现。
    pub driver: Arc<dyn Driver>,
    /// 驱动申请到的主设备号（major）。
    pub major: usize,
}

/// 所有已注册的驱动。
#[lazy(rwlock)]
pub static DRIVERS: Vec<RegisteredDriver> = Vec::new();

/// 注册一个驱动，并按 [`Driver::major`] 申请主设备号。
///
/// 返回申请到的主设备号；之后该驱动 [`probe`](Driver::probe) 出的设备都会登记
/// 在该主设备号下，各自分配不同的从设备号。
///
/// # Errors
///
/// - [`DeviceError::InvalidMajor`]：驱动申请的静态主设备号不是内核预设。
/// - [`DeviceError::MajorInUse`]：该静态主设备号已被占用。
/// - [`DeviceError::AllocFailed`]：动态主设备号空间已耗尽。
pub fn register(driver: Arc<dyn Driver>) -> Result<usize, DeviceError> {
    let major = DEVICE_MANAGER.write().request_major(driver.major())?;
    DRIVERS.write().push(RegisteredDriver { driver, major });
    Ok(major)
}

/// 设备 → 驱动：为设备描述 `desc` 选出 rank 最高的驱动。
pub fn find_driver(desc: &DeviceDesc) -> Option<Arc<dyn Driver>> {
    DRIVERS
        .read()
        .iter()
        .filter_map(|entry| {
            entry
                .driver
                .rank(desc)
                .map(|rank| (rank, entry.driver.clone()))
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, driver)| driver)
}

/// 驱动 → 设备：列出驱动 `driver` 能处理的所有设备描述，按优先级从高到低排序。
pub fn match_devices(driver: &dyn Driver) -> Vec<RankedDesc> {
    let mut matched: Vec<RankedDesc> = DEVICE_DESC_MANAGER
        .read()
        .all
        .iter()
        .filter_map(|desc| {
            driver
                .rank(desc)
                .map(|rank| RankedDesc::new(desc.clone(), rank))
        })
        .collect();
    matched.sort_by(|a, b| b.rank.cmp(&a.rank));
    matched
}

/// 探测全部设备：对设备树中的每个设备描述，按 rank 从高到低依次尝试匹配的驱动
/// （`probe` 成功即绑定），把成功者登记进
/// [`DEVICE_MANAGER`](crate::device::DEVICE_MANAGER) 中该驱动的主设备号下。
///
/// 之所以要「依次尝试」而非只挑 rank 最高者：多个驱动可能共享同一 compatible
/// 串（如 virtio-blk、virtio-net 与 virtio-rng 都以 `virtio,mmio` 匹配），需要靠 `probe`
/// 读取设备类型寄存器才能区分。
///
/// 返回成功登记的设备数。先快照驱动与设备描述再逐个 probe，避免在 probe
/// 期间持锁。
pub fn probe_all() -> usize {
    let drivers: Vec<RegisteredDriver> = DRIVERS.read().clone();
    let descs: Vec<Arc<DeviceDesc>> = DEVICE_DESC_MANAGER.read().all.clone();

    let mut bound = 0;
    for desc in descs {
        let mut candidates: Vec<(usize, &RegisteredDriver)> = drivers
            .iter()
            .filter_map(|entry| entry.driver.rank(&desc).map(|rank| (rank, entry)))
            .collect();
        candidates.sort_by(|a, b| b.0.cmp(&a.0));

        for (_, entry) in candidates {
            if let Ok(device) = entry.driver.probe(&desc) {
                let mode = entry.driver.mode();
                if DEVICE_MANAGER
                    .write()
                    .register(
                        entry.major,
                        desc.clone(),
                        device,
                        entry.driver.clone(),
                        mode,
                    )
                    .is_ok()
                {
                    bound += 1;
                }
                break;
            }
        }
    }
    bound
}
