//! Sv39 地址位域。
//!
//! [`VirtualAddr`] 与 [`PhysicalAddr`] 由 `bits!` 生成，可拆出
//! `page_offset` / `vpn` / `ppn` 等字段，也可通过 `as_ptr` / `add` 等
//! 方法用于指针算术与访问；[`AsPAddr`] / [`AsVAddr`] 为指针与引用提供到
//! 物理/虚拟地址的转换。
//!
//! 两者彼此转换通过内核偏移 `KERNEL_OFFSET`：物理地址加偏移得虚拟地址，
//! 虚拟地址减偏移得物理地址。
use goethite_macros::bits;

/// 把指针或引用转换为物理地址。
///
/// 为裸指针与引用（`*const T` / `*mut T` / `&T` / `&mut T`）实现，便于直接
/// 用物理地址访问映射后的内存。
pub trait AsPAddr {
    /// 把当前指针或引用转换为物理地址。
    fn as_paddr(&self) -> PhysicalAddr;
}

/// 把裸指针或引用转换为虚拟地址。
///
/// 为裸指针与引用（`*const T` / `*mut T` / `&T` / `&mut T`）实现。
pub trait AsVAddr {
    /// 把当前指针或引用转换为虚拟地址。
    fn as_vaddr(&self) -> VirtualAddr;
}

bits! {
    pub type VirtualAddr : usize {
        page_offset: 0 => 11,
        vpn0: 12 => 20,
        vpn1: 21 => 29,
        vpn2: 30 => 38,
        vpn: 12 => 38,
    }
}

bits! {
    pub type PhysicalAddr : usize {
        page_offset: 0 => 11,
        ppn0: 12 => 20,
        ppn1: 21 => 29,
        ppn2: 30 => 55,
        ppn: 12 => 55,
    }
}

impl<T> AsPAddr for *mut T {
    fn as_paddr(&self) -> PhysicalAddr {
        PhysicalAddr::from(*self as usize)
    }
}

impl<T> AsPAddr for *const T {
    fn as_paddr(&self) -> PhysicalAddr {
        PhysicalAddr::from(*self as usize)
    }
}

impl<T> AsPAddr for &T {
    fn as_paddr(&self) -> PhysicalAddr {
        PhysicalAddr::from(*self as *const T as usize)
    }
}

impl<T> AsPAddr for &mut T {
    fn as_paddr(&self) -> PhysicalAddr {
        PhysicalAddr::from(*self as *const T as usize)
    }
}

impl<T> AsVAddr for *const T {
    fn as_vaddr(&self) -> VirtualAddr {
        VirtualAddr::from(*self as usize)
    }
}

impl<T> AsVAddr for *mut T {
    fn as_vaddr(&self) -> VirtualAddr {
        VirtualAddr::from(*self as usize)
    }
}

impl<T> AsVAddr for &T {
    fn as_vaddr(&self) -> VirtualAddr {
        VirtualAddr::from(*self as *const T as usize)
    }
}

impl<T> AsVAddr for &mut T {
    fn as_vaddr(&self) -> VirtualAddr {
        VirtualAddr::from(*self as *const T as usize)
    }
}

impl From<PhysicalAddr> for u64 {
    fn from(paddr: PhysicalAddr) -> u64 {
        paddr.0 as u64
    }
}

impl PhysicalAddr {
    /// 把物理地址当作类型 `T` 的常量指针。
    ///
    /// 不检查地址有效性，由调用者保证该地址当前可访问。
    pub fn as_ptr<T>(&self) -> *const T {
        self.0 as *const T
    }

    /// 把物理地址当作类型 `T` 的可变指针。
    ///
    /// 安全性同 [`as_ptr`](Self::as_ptr) 由调用者保证。
    pub fn as_mut_ptr<T>(&self) -> *mut T {
        self.0 as *mut T
    }

    /// 转换为虚拟地址（加上内核偏移 `KERNEL_OFFSET`）。
    pub fn as_vaddr(&self) -> VirtualAddr {
        self.clone().into()
    }

    /// 返回新的物理地址：在原地址上加上 `count` 个 `T` 元素的大小。
    pub fn add<T>(&self, count: usize) -> Self {
        Self(self.0 + (count * size_of::<T>()))
    }
}

impl VirtualAddr {
    /// 把虚拟地址当作类型 `T` 的常量指针。
    ///
    /// 不检查地址有效性，由调用者保证该地址当前可访问。
    pub fn as_ptr<T>(&self) -> *const T {
        self.0 as *const T
    }

    /// 把虚拟地址当作类型 `T` 的可变指针。
    ///
    /// 安全性同 [`as_ptr`](Self::as_ptr) 由调用者保证。
    pub fn as_mut_ptr<T>(&self) -> *mut T {
        self.0 as *mut T
    }

    /// 转换为物理地址（减去内核偏移 `KERNEL_OFFSET`）。
    pub fn as_paddr(&self) -> PhysicalAddr {
        self.clone().into()
    }

    /// 返回新的虚拟地址：在原地址上加上 `count` 个 `T` 元素的大小。
    pub fn add<T>(&self, count: usize) -> Self {
        Self(self.0 + (count * size_of::<T>()))
    }
}

impl From<PhysicalAddr> for VirtualAddr {
    fn from(paddr: PhysicalAddr) -> VirtualAddr {
        VirtualAddr::from(paddr.0 + goethite_config::KERNEL_OFFSET)
    }
}

impl From<VirtualAddr> for PhysicalAddr {
    fn from(vaddr: VirtualAddr) -> PhysicalAddr {
        PhysicalAddr::from(vaddr.0 - goethite_config::KERNEL_OFFSET)
    }
}

pub enum VPN {
    VPN0,
    VPN1,
    VPN2,
}

pub trait GetPPN<T> {
    fn get_ppn(&self, cmd: PPN) -> T;
}

impl GetPPN<usize> for PhysicalAddr {
    fn get_ppn(&self, cmd: PPN) -> usize {
        match cmd {
            PPN::PPN0 => self.ppn0(),
            PPN::PPN1 => self.ppn1(),
            PPN::PPN2 => self.ppn2(),
        }
    }
}
pub enum PPN {
    PPN0,
    PPN1,
    PPN2,
}

pub trait GetVPN<T> {
    fn get_vpn(&self, cmd: VPN) -> T;
}

impl GetVPN<usize> for VirtualAddr {
    fn get_vpn(&self, cmd: VPN) -> usize {
        match cmd {
            VPN::VPN0 => self.vpn0(),
            VPN::VPN1 => self.vpn1(),
            VPN::VPN2 => self.vpn2(),
        }
    }
}