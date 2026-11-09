//! 设备描述：从设备树（FDT）解析并索引设备。
//!
//! [`DeviceDesc`] 是从一个 FDT 节点提取出的设备元数据（兼容表、`reg` 资源
//! 窗口、中断号、原始属性）。[`DeviceDescManager`] 汇总 FDT 中的全部设备，
//! 并建立按名字与兼容串的索引，[`DEVICE_DESC_MANAGER`] 是全内核唯一的实例。
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};

use fdt::{
    nodes::Node,
    parsing::{Panic, aligned::AlignedParser},
    properties::Compatible,
};
use goethite_id_alloc::Id;
use goethite_macros::{bits, lazy};

use crate::{FDT_ADDR, device::Resource};

/// 从单个设备树（FDT）节点提取出的设备元数据。
pub struct DeviceDesc {
    /// 节点名（如 `"uart0"`）。
    pub name: String,
    /// 兼容串列表（`compatible` 属性，从最具体到最通用排序）。
    pub compatible: Vec<String>,
    /// 设备占据的地址资源窗口（`reg` 属性）。
    pub resources: Vec<Resource>,
    /// 设备用到的中断号列表（`interrupts` 属性）。
    pub irq: Vec<usize>,
    /// 其余原始属性（属性名 → 原始字节），供驱动按需读取。
    pub props: BTreeMap<String, Vec<u8>>,
}

impl<'a> From<Node<'a, (AlignedParser<'a>, Panic)>> for DeviceDesc {
    fn from(value: Node<'a, (AlignedParser<'a>, Panic)>) -> Self {
        let name = value.name().name.to_string();

        let compatible = value
            .property::<Compatible>()
            .map(|compatible| compatible.all().map(str::to_string).collect())
            .unwrap_or_default();

        let mut resources = Vec::new();
        if let Some(reg) = value.reg() {
            for entry in reg.iter::<usize, usize>() {
                if let Ok(entry) = entry {
                    resources.push(Resource::new(entry.address, entry.len));
                }
            }
        }

        let irq = value
            .raw_property("interrupts")
            .map(|prop| {
                prop.value
                    .chunks_exact(4)
                    .map(|cells| u32::from_be_bytes(cells.try_into().unwrap()) as usize)
                    .collect()
            })
            .unwrap_or_default();

        let props = value
            .properties()
            .iter()
            .map(|prop| (prop.name.to_string(), prop.value.to_vec()))
            .collect();

        Self {
            name,
            compatible,
            resources,
            irq,
            props,
        }
    }
}

// 设备 id：在 [`DeviceDescManager`] 中唯一标识一台设备。
//
// 采用类 `dev_t` 的两级编码：低 `DEVICE_ID_MINOR_BITS` 位是从设备号（minor），
// 其余高位是主设备号（major）。字段范围由 `bits!` 生成，因此该类型同时带有
// `Debug` / `Deref` / 位运算等实现；`Id` 则由下方手工实现（与 `#[derive(Id)]`
// 生成的一组 trait 冲突）。
bits! {
    pub type DeviceId : usize {
        // 从设备号（minor）：低 8 位，宽度须与 goethite_config::DEVICE_ID_MINOR_BITS 一致。
        minor: 0 => 7,
        // 主设备号（major）：其余高位。
        major: 8 => 63,
    }
}

// 保证 bits! 中 minor 字段宽度与配置常量一致。
const _: () = assert!(
    goethite_config::DEVICE_ID_MINOR_BITS == 8,
    "DEVICE_ID_MINOR_BITS 必须与 DeviceId 的 minor 字段宽度一致",
);

impl DeviceId {
    /// 由主设备号与从设备号组合出设备 id。
    pub fn from_parts(major: usize, minor: usize) -> Self {
        Self::builder().set_major(major).set_minor(minor).build()
    }
}

/// 手工实现 [`Id`]：`bits!` 已提供 `Debug` / `Copy` / `Ord` 等前置能力。
impl Id for DeviceId {
    const BITS: u32 = usize::BITS;
    const MAX: Self = Self(usize::MAX);
    const MIN: Self = Self(usize::MIN);

    fn next(&self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }

    fn prev(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }

    fn distance_to(&self, other: &Self) -> usize {
        other.0 - self.0
    }

    fn to_bits(&self) -> u128 {
        self.0 as u128
    }

    fn from_bits(bits: u128) -> Self {
        Self(bits as usize)
    }
}

/// 一台设备的描述与其「兼容匹配优先级」。
pub struct RankedDesc {
    /// 设备描述。
    pub desc: Arc<DeviceDesc>,
    /// 兼容级别：兼容串越靠前（越具体）优先级越高。
    pub rank: usize,
}

impl RankedDesc {
    /// 构造一个带优先级的设备描述。
    pub fn new(desc: Arc<DeviceDesc>, rank: usize) -> Self {
        Self { desc, rank }
    }
}

/// 设备描述管理器：汇总所有设备描述，并按名字 / 兼容串建立索引。
pub struct DeviceDescManager {
    pub all: Vec<Arc<DeviceDesc>>,
    pub by_compatible: BTreeMap<String, Vec<RankedDesc>>,
    pub by_name: BTreeMap<String, Vec<Arc<DeviceDesc>>>,
}

impl From<*const u32> for DeviceDescManager {
    fn from(value: *const u32) -> Self {
        let mut result = Self::new();

        let fdt = match unsafe { fdt::Fdt::from_ptr(value) } {
            Ok(fdt) => fdt,
            Err(err) => {
                panic!("fdt parse error: {}", err);
            }
        };

        for (_, node) in fdt.all_nodes() {
            result.register(node.into())
        }

        result
    }
}

impl DeviceDescManager {
    pub fn new() -> Self {
        Self {
            all: Vec::new(),
            by_name: BTreeMap::new(),
            by_compatible: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, desc: DeviceDesc) {
        let desc = Arc::new(desc);

        self.all.push(desc.clone());

        let name = desc.name.clone();
        let compatible = desc.compatible.clone();

        if let Some(devices) = self.by_name.get_mut(&desc.name) {
            devices.push(desc.clone());
        } else {
            self.by_name.insert(name, vec![desc.clone()]);
        }

        let mut rank = compatible.len();

        for component in compatible {
            if let Some(devices) = self.by_compatible.get_mut(&component) {
                devices.push(RankedDesc::new(desc.clone(), rank));
            } else {
                self.by_compatible
                    .insert(component, vec![RankedDesc::new(desc.clone(), rank)]);
            }

            rank -= 1;
        }
    }
}

#[lazy(rwlock)]
pub static DEVICE_DESC_MANAGER: DeviceDescManager =
    DeviceDescManager::from(unsafe { FDT_ADDR }.as_ptr());
