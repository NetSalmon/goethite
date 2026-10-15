//! 设备子系统。
//!
//! - [`Device`]：块 / 字符设备的统一读写抽象；
//! - [`DeviceNode`]：已注册的设备节点（设备 id + 实例 + 驱动 + 元数据）；
//! - [`DeviceManager`]：设备管理器，负责申请 / 回收主设备号、注册设备、
//!   分配两级设备 id（major / minor）并建立名字索引；
//! - [`Major`] / [`preset`] / [`PRESET_MAJORS`]：主设备号的申请方式与内核预设值；
//! - [`Resource`]：设备在系统总线上的地址资源窗口（设备树 `reg` 区间），
//!   并提供易失的 MMIO 读写；[`DeviceInfo`] 在此之上附加中断号；
//! - [`mmio_regs`]：按寄存器偏移生成 MMIO 访问方法的宏；
//! - [`FileType`] / [`Mode`]：UNIX `st_mode` 的文件类型与权限位（供文件系统复用）；
//! - [`desc`]：从设备树（FDT）解析设备描述并建立索引；
//! - [`driver`]：设备驱动注册表与设备-驱动双向匹配；
//! - 具体驱动：`ns16550a`（UART）、`virtio_mmio`（传输层）、`virtio_blk`、
//!   `virtio_rng`、`fw_cfg`（QEMU firmware config）、`ramfb`（QEMU 显示）；
//! - 平台辅助：`plic`（外部中断控制器）、`reboot`（平台复位）。

pub mod desc;
pub mod driver;
pub mod fw_cfg;
pub mod ns16550a;
pub mod plic;
pub mod ramfb;
pub mod reboot;
pub mod virtio_blk;
pub mod virtio_mmio;
pub mod virtio_rng;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    sync::Arc,
    vec::Vec,
};
use core::ops::Range;

use desc::{DeviceDesc, DeviceId};
use driver::Driver;
use goethite_config::DEVICE_ID_MINOR_BITS;
use goethite_id_alloc::IdAlloc;
use goethite_macros::{bits, lazy, numeric};
use goethite_riscv64::addr::{PhysicalAddr, VirtualAddr};

/// 设备操作可能出现的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    /// 目标设备不支持所请求的操作（如对只读设备执行写入）。
    #[error("operation not supported by the device")]
    Unsupported,
    /// 设备 id / 主设备号 / 从设备号分配失败（空间已耗尽）。
    #[error("device id allocation failed")]
    AllocFailed,
    /// 指定的设备 id 不存在。
    #[error("device not found")]
    NotFound,
    /// 主设备号不是内核预设，或尚未被申请。
    #[error("invalid major device number")]
    InvalidMajor,
    /// 指定的静态主设备号已被申请。
    #[error("major device number already in use")]
    MajorInUse,
    /// 设备请求执行失败（如 virtio-blk 请求返回错误状态）。
    #[error("device request failed")]
    Io,
    /// 设备尚未就绪（握手、特性协商或队列配置失败）。
    #[error("device not ready")]
    NotReady,
}

/// 设备操作的结果：
///
/// - `Ok(bytes)`：成功完成，携带本次传输/synchronize 的**单元数**（对
///   [`Device::read`] / [`Device::write`] 是字节数，对 [`Device::read_at`] /
///   [`Device::write_at`] 是扇区数）。
/// - `Err(e)`：底层驱动返回 [`DeviceError`]。
pub type DeviceResult = Result<usize, DeviceError>;

/// 块 / 字符设备的上层抽象：以一致的读写接口屏蔽幕后驱动差异。
///
/// 需要 `Send + Sync`，因为设备以 [`Arc`] 形式存放于全局
/// [`DeviceManager`]。
#[allow(unused)]
pub trait Device: Send + Sync {
    /// 从设备顺序读取数据到 `buf`，返回实际读到的字节数。
    fn read(&self, buf: &mut [u8]) -> DeviceResult {
        Err(DeviceError::Unsupported)
    }

    /// 把 `buf` 的数据顺序写入设备，返回实际写入的字节数。
    fn write(&self, buf: &[u8]) -> DeviceResult {
        Err(DeviceError::Unsupported)
    }

    /// 从设备第 `sector` 个扇区处读取数据到 `buf`，返回实际读到的扇区数。
    fn read_at(&self, sector: usize, buf: &mut [u8]) -> DeviceResult {
        Err(DeviceError::Unsupported)
    }

    /// 把 `buf` 的数据写入设备第 `sector` 个扇区处，返回实际写入的扇区数。
    fn write_at(&self, sector: usize, buf: &[u8]) -> DeviceResult {
        Err(DeviceError::Unsupported)
    }
}

// 文件类型（mode 高 4 位，bits 15..12 的取值），与 POSIX `S_IF*` 一致。
numeric! {
    pub enum FileType : u16 {
        FIFO = 0o1,   // 命名管道  S_IFIFO
        CHR = 0o2,    // 字符设备  S_IFCHR
        DIR = 0o4,    // 目录      S_IFDIR
        BLK = 0o6,    // 块设备    S_IFBLK
        REG = 0o10,   // 普通文件  S_IFREG
        LNK = 0o12,   // 符号链接  S_IFLNK
        SOCK = 0o14,  // 套接字    S_IFSOCK
    }
}

// 文件 mode 位，布局与 Unix `st_mode` 一致：
//
// ```text
// 15..12      11      10     9    8 7 6   5 4 3   2 1 0
// 文件类型  setuid setgid sticky  属主    属组    其他
//                                rwx     rwx     rwx
// ```
bits! {
    pub type Mode : u16 {
        // 文件类型位（bits 15..12），取值见下方 `FileType` 枚举。
        file_type: FileType : 12 => 15,

        // 特殊权限位。
        setuid: 11,
        setgid: 10,
        sticky: 9,

        // 属主（owner）rwx 权限位。
        user_execute: 6,
        user_write: 7,
        user_read: 8,
        user: 6 => 8,

        // 属组（group）rwx 权限位。
        group_execute: 3,
        group_write: 4,
        group_read: 5,
        group: 3 => 5,

        // 其他用户（other）rwx 权限位。
        other_execute: 0,
        other_write: 1,
        other_read: 2,
        other: 0 => 2,
    }
}

/// 一个向内核注册的设备节点：能唯一标识设备、持有驱动实例与其元数据。
pub struct DeviceNode {
    /// 设备在 [`DeviceManager`] 中的唯一 id。
    pub id: DeviceId,
    /// 设备的驱动实现（trait object）。
    pub device: Arc<dyn Device>,
    /// 绑定该设备的驱动（用于解绑 / 回溯）。
    pub driver: Arc<dyn Driver>,

    /// 设备在文件系统 / 命名空间中的访问权限位。
    pub mode: Mode,
    /// 设备的名字（来自设备树的节点名）。
    pub name: String,

    /// 由设备树解析出的设备描述。
    pub desc: Arc<DeviceDesc>,
}

/// 设备在系统总线上的一个地址资源窗口（对应设备树的 `reg` 区间）。
///
/// `start` 是设备树给出的**物理**地址；内核在 [`init_page_table`](crate::init::init_page_table)
/// 中把该窗口线性映射到高半区（`虚拟地址 = 物理地址 + KERNEL_OFFSET`），
/// 因此 [`read`](Self::read) / [`write`](Self::write) 访问前会做物理→虚拟转换。
#[derive(Debug, Clone, Copy)]
pub struct Resource {
    /// 窗口的起始物理地址 / 偏移。
    pub start: usize,
    /// 窗口的大小（字节）。
    pub size: usize,
}

impl Resource {
    /// 窗口的结束地址（不包含），即 `start + size`。
    pub fn end(&self) -> usize {
        self.start + self.size
    }

    /// 构造一个地址资源窗口。
    pub const fn new(start: usize, size: usize) -> Self {
        Self { start, size }
    }

    /// 窗口起始物理地址对应的内核虚拟地址。
    #[inline]
    pub fn as_vaddr(&self) -> VirtualAddr {
        PhysicalAddr::from(self.start).as_vaddr()
    }

    /// 从 `start + offset` 处读取一个 `T`（易失读）。
    ///
    /// # Safety
    ///
    /// 调用方须保证 `offset..offset + size_of::<T>()` 落在合法 MMIO 窗口内，
    /// 且 `T` 是可从 MMIO 读取的普通类型。
    #[inline]
    pub fn read<T>(&self, offset: usize) -> T {
        let ptr = PhysicalAddr::from(self.start + offset)
            .as_vaddr()
            .as_ptr::<T>();
        unsafe { ptr.read_volatile() }
    }

    /// 向 `start + offset` 处写入一个 `T`（易失写）。
    ///
    /// # Safety
    ///
    /// 同 [`read`](Self::read)，`offset..offset + size_of::<T>()` 必须落在合法
    /// MMIO 窗口内。
    #[inline]
    pub fn write<T>(&self, offset: usize, value: T) {
        let ptr = PhysicalAddr::from(self.start + offset)
            .as_vaddr()
            .as_mut_ptr::<T>();
        unsafe { ptr.write_volatile(value) }
    }
}

impl From<Range<usize>> for Resource {
    fn from(range: Range<usize>) -> Resource {
        let start = range.start;
        let size = range.end - start;

        Resource { start, size }
    }
}

/// 一台设备的硬件描述：一段 MMIO 资源窗口与可选的中断号。
///
/// 从 [`DeviceDesc`] 解析而来，供驱动通过 `mmio_regs!` 生成的寄存器访问方法
/// 使用（访问 `device.mmio`）。
#[derive(Debug, Clone, Copy)]
pub struct DeviceInfo {
    /// 设备寄存器所在的 MMIO 窗口。
    pub mmio: Resource,
    /// 设备使用的中断号（无中断设备为 `None`）。
    pub irq: Option<usize>,
}

impl DeviceInfo {
    /// 构造一份设备硬件描述。
    pub const fn new(mmio: Resource, irq: Option<usize>) -> Self {
        Self { mmio, irq }
    }

    /// 从设备树描述中取出首个 MMIO 窗口与首个中断号。
    ///
    /// 设备描述不含 `reg` 时返回 `None`。
    pub fn from_desc(desc: &DeviceDesc) -> Option<Self> {
        let mmio = *desc.resources.first()?;
        Some(Self::new(mmio, desc.irq.first().copied()))
    }
}

/// 把内核虚拟地址转换为设备可用的物理地址。
///
/// 内核堆 / 栈 / 镜像位于高半区（`>= KERNEL_OFFSET`），减去偏移即为物理地址；
/// 启动早期（尚未切换页表）的低地址本身即物理地址，原样返回。用于把
/// virtio 描述符、fw_cfg DMA 缓冲等交给设备直接访问。
#[inline]
pub fn virt_to_phys(addr: usize) -> usize {
    if addr >= goethite_config::KERNEL_OFFSET {
        *VirtualAddr::from(addr).as_paddr()
    } else {
        addr
    }
}

/// 按寄存器偏移为设备生成易失 MMIO 访问方法。
///
/// 语法：`mmio_regs!(Device: [reg[: Ty] => offset, ...])`。为每个 `reg` 生成
/// `reg()` / `write_reg(val)`：写成 `reg: Ty` 时以固定宽度 `Ty` 访问，省略时
/// 生成泛型方法由调用处推断宽度。同时在模块内生成常量 `REG_OFFSET`。设备须有
/// 一个 `device: DeviceInfo` 字段。
#[macro_export]
macro_rules! mmio_regs {
    ($device:ident: [ $( $reg:ident $( : $t:ty )? => $offset:expr ),+ $(,)? ]) => {
        paste::paste! {
            $( #[allow(dead_code)] const [<$reg:upper _OFFSET>]: usize = $offset as usize; )+

            #[allow(dead_code)]
            impl $device {
                $(
                    $crate::mmio_regs!(@helper &self, $reg, $($t)?, $offset);
                )+
            }
        }
    };

    (@helper &self, $reg:ident, $t:ty, $offset:expr) => {
        paste::paste! {
            #[inline]
            pub fn [< $reg:snake >](&self) -> $t {
                self.device.mmio.read::<$t>($offset)
            }

            #[inline]
            pub fn [< write_ $reg:snake >](&self, val: $t) {
                self.device.mmio.write::<$t>($offset, val);
            }
        }
    };

    (@helper &self, $reg:ident, , $offset:expr) => {
        paste::paste! {
            #[inline]
            pub fn [< $reg:snake >]<T>(&self) -> T {
                self.device.mmio.read::<T>($offset)
            }

            #[inline]
            pub fn [< write_ $reg:snake >]<T>(&self, val: T) {
                self.device.mmio.write::<T>($offset, val);
            }
        }
    };
}

/// 内核预设的主设备号。
pub mod preset {
    /// 内存类设备（`/dev/mem`、`/dev/null`、`/dev/zero` 等）。
    pub const MEM: usize = 1;
    /// 终端 / 串口设备。
    pub const TTY: usize = 2;
    /// 块设备（virtio-blk 等）。
    pub const BLOCK: usize = 3;
    /// 杂项设备。
    pub const MISC: usize = 4;
}

/// 全部内核预设主设备号，在 [`DeviceManager::new`] 时预留。
pub const PRESET_MAJORS: [usize; 4] = [preset::MEM, preset::TTY, preset::BLOCK, preset::MISC];

/// 驱动对主设备号的申请方式，由 [`Driver::major`](Driver::major) 声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Major {
    /// 申请一个内核预设的静态主设备号。
    Static(usize),
    /// 由内核动态分配一个主设备号。
    Dynamic,
}

/// 设备管理器：集中持有所有已注册设备，负责分配 / 回收设备 id 并维护名字索引。
///
/// 设备号采用类 `dev_t` 的两级编码：主设备号（major）由驱动通过
/// [`Driver::major`] 申请——可以是内核预设的静态主设备号，也可以由内核动态
/// 分配；从设备号（minor）在 [`register`](Self::register) 时从该 major 的块内
/// 分配。最终 id = `major << DEVICE_ID_MINOR_BITS | minor`。
///
/// `by_name` 允许按设备树节点名反查（同名设备可能有多台）。
pub struct DeviceManager {
    /// 已注册设备，按 id 索引。
    devices: BTreeMap<DeviceId, Arc<DeviceNode>>,
    /// 名字索引：设备名 → 设备 id 列表。
    by_name: BTreeMap<String, Vec<DeviceId>>,
    /// 已被驱动申请的主设备号集合（预设或动态）。
    claimed_majors: BTreeSet<usize>,
    /// 两级设备号分配器。
    ids: IdAlloc<DeviceId, DEVICE_ID_MINOR_BITS>,
}

impl DeviceManager {
    /// 构造一个空的设备管理器，并预留全部 [`PRESET_MAJORS`]。
    pub fn new() -> Self {
        let mut ids =
            IdAlloc::from_range(DeviceId::from(0)..DeviceId::from(goethite_config::DEVICE_ID_MAX))
                .expect("device id range must be valid");

        for &major in &PRESET_MAJORS {
            ids.alloc_major_at(DeviceId::from_parts(major, 0))
                .expect("preset major must fit in the device id range");
        }

        Self {
            devices: BTreeMap::new(),
            by_name: BTreeMap::new(),
            claimed_majors: BTreeSet::new(),
            ids,
        }
    }

    /// 申请一个主设备号。
    ///
    /// - [`Major::Static`]：申请一个内核预设的主设备号，必须属于
    ///   [`PRESET_MAJORS`] 且尚未被占用；
    /// - [`Major::Dynamic`]：由内核从空闲区动态分配一个主设备号。
    ///
    /// 返回申请到的主设备号。
    ///
    /// # Errors
    ///
    /// - [`DeviceError::InvalidMajor`]：静态主设备号不是内核预设。
    /// - [`DeviceError::MajorInUse`]：该静态主设备号已被申请。
    /// - [`DeviceError::AllocFailed`]：动态主设备号空间已耗尽。
    pub fn request_major(&mut self, request: Major) -> Result<usize, DeviceError> {
        match request {
            Major::Static(major) => {
                if !PRESET_MAJORS.contains(&major) {
                    return Err(DeviceError::InvalidMajor);
                }
                if !self.claimed_majors.insert(major) {
                    return Err(DeviceError::MajorInUse);
                }
                Ok(major)
            }
            Major::Dynamic => {
                let base = self.ids.alloc_major().ok_or(DeviceError::AllocFailed)?;
                let major = base.major();
                self.claimed_majors.insert(major);
                Ok(major)
            }
        }
    }

    /// 释放一个已申请的主设备号。
    ///
    /// 预设主设备号被释放后回到「未申请」状态；动态主设备号则整块归还给
    /// 分配器。
    ///
    /// # Errors
    ///
    /// - [`DeviceError::NotFound`]：该主设备号未被申请。
    pub fn release_major(&mut self, major: usize) -> Result<(), DeviceError> {
        if !self.claimed_majors.remove(&major) {
            return Err(DeviceError::NotFound);
        }
        if !PRESET_MAJORS.contains(&major) {
            self.ids.dealloc_major(DeviceId::from_parts(major, 0)).ok();
        }
        Ok(())
    }

    /// 在某主设备号下注册一台设备：分配从设备号、构造 [`DeviceNode`] 并建立
    /// 名字索引。
    ///
    /// # Errors
    ///
    /// - [`DeviceError::InvalidMajor`]：`major` 未被申请。
    /// - [`DeviceError::AllocFailed`]：该主设备号下的从设备号已耗尽。
    pub fn register(
        &mut self,
        major: usize,
        desc: Arc<DeviceDesc>,
        device: Arc<dyn Device>,
        driver: Arc<dyn Driver>,
        mode: Mode,
    ) -> Result<DeviceId, DeviceError> {
        if !self.claimed_majors.contains(&major) {
            return Err(DeviceError::InvalidMajor);
        }

        let id = self
            .ids
            .alloc_minor(DeviceId::from_parts(major, 0))
            .ok_or(DeviceError::AllocFailed)?;
        let name = desc.name.clone();

        let node = Arc::new(DeviceNode {
            id,
            device,
            driver,
            mode,
            name: name.clone(),
            desc,
        });

        self.devices.insert(id, node);
        self.by_name.entry(name).or_default().push(id);

        Ok(id)
    }

    /// 按 id 取出设备节点。
    pub fn get(&self, id: DeviceId) -> Option<Arc<DeviceNode>> {
        self.devices.get(&id).cloned()
    }

    /// 按设备名取出所有同名设备节点。
    pub fn by_name(&self, name: &str) -> Vec<Arc<DeviceNode>> {
        self.by_name
            .get(name)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.devices.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 所有已注册设备节点。
    pub fn all(&self) -> Vec<Arc<DeviceNode>> {
        self.devices.values().cloned().collect()
    }

    /// 已注册设备数量。
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// 是否没有任何已注册设备。
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// 注销一台设备：解绑驱动、清除索引并回收 id。
    ///
    /// # Errors
    ///
    /// - [`DeviceError::NotFound`]：`id` 未注册。
    pub fn unregister(&mut self, id: DeviceId) -> Result<(), DeviceError> {
        let node = self.devices.remove(&id).ok_or(DeviceError::NotFound)?;
        node.driver.remove(&node.device);

        if let Some(ids) = self.by_name.get_mut(&node.name) {
            ids.retain(|&registered| registered != id);
            if ids.is_empty() {
                self.by_name.remove(&node.name);
            }
        }

        self.ids.dealloc(id).ok();

        Ok(())
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 全内核唯一的设备管理器，懒加载并以自旋锁保护。
#[lazy(rwlock)]
pub static DEVICE_MANAGER: DeviceManager = DeviceManager::new();
