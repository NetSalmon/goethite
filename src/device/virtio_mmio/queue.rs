//! virtio 虚拟队列。
//!
//! 包含描述符表、avail / used 环与对齐到页的 [`Queue`] 布局，以及队列初始化
//! 逻辑。队列内存经 [`Handle`] 从伙伴系统分配，得到的是内核高半区虚拟地址；
//! 写入 virtio-mmio 寄存器时再转换为设备可识别的**物理**地址。
use core::{
    ptr::addr_of,
    sync::atomic::{Ordering, fence},
};

use goethite_config::RING_SIZE;
use goethite_macros::bits;
use goethite_riscv64::addr::VirtualAddr;

use crate::{
    device::DeviceError,
    mm::object::{Handle, Init, Uninit},
};

/// `Flags` 访问 / 写入以 `u16` 存储，与 virtio 规范一致。
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VirtQueueDescriptor {
    pub addr: u64,
    pub len: u32,
    pub flags: Flags,
    pub next: u16,
}

bits! {
    pub type Flags : u16 {
        next: 0,
        write: 1,
        indirect: 2,
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvailableRing {
    pub flags: u16,
    pub idx: u16,
    pub ring: Ring<u16>,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct UsedElement {
    pub id: u32,
    pub len: u32,
}

#[repr(C)]
#[derive(Debug)]
pub struct UsedRing {
    pub flags: u16,
    pub idx: u16,
    pub ring: Ring<UsedElement>,
}

#[repr(C, align(4096))]
pub struct Queue {
    pub desc: [VirtQueueDescriptor; RING_SIZE],
    pub avail: AvailableRing,
    // 把 used 环对齐到下一页：legacy virtio 要求 used 环落在页边界上
    // （avail 紧跟 desc 表，offset = 16 * RING_SIZE = 4096）。
    _pad: [u8; 4096 - size_of::<AvailableRing>()],
    pub used: UsedRing,
}

impl Queue {
    pub fn zeroed() -> Self {
        // SAFETY: `Queue` 全部由普通整数 / 数组构成，全零是合法初值。
        unsafe { core::mem::zeroed() }
    }
}

/// 一段从伙伴系统分配、按 [`Queue`] 解释的虚拟队列内存。
pub struct VirtQueue {
    mem: Handle<Queue, Init>,
}

impl VirtQueue {
    /// 分配并清零一个虚拟队列。
    pub fn new() -> Result<Self, DeviceError> {
        let uninit = Handle::<Queue, Uninit>::alloc().map_err(|_| DeviceError::AllocFailed)?;
        Ok(Self {
            mem: uninit.init(Queue::zeroed()),
        })
    }

    /// 队列内存的高半区虚拟地址。
    fn base(&self) -> usize {
        // SAFETY: 只读取已初始化页块的地址，不解引用。
        unsafe { self.mem.as_ptr() as usize }
    }

    /// 队列内存的物理地址（设备通过该地址访问）。
    fn paddr(&self) -> u64 {
        u64::from(VirtualAddr::from(self.base()).as_paddr())
    }

    pub fn desc_addr(&self) -> u64 {
        self.paddr() + core::mem::offset_of!(Queue, desc) as u64
    }

    pub fn avail_addr(&self) -> u64 {
        self.paddr() + core::mem::offset_of!(Queue, avail) as u64
    }

    pub fn used_addr(&self) -> u64 {
        self.paddr() + core::mem::offset_of!(Queue, used) as u64
    }

    pub fn address(&self) -> u64 {
        self.paddr()
    }

    pub fn as_mut(&mut self) -> &mut Queue {
        self.mem.get_mut()
    }

    /// 把 `head` 指向的描述符链追加到 avail 环，返回追加前的 used 索引
    /// （作 [`Self::wait_used`] 的等待基准）。
    ///
    /// 调用前必须已把描述符表填好；写入后执行 `fence`，保证设备在收到
    /// notify 前能看到全部描述符与 avail 更新。
    pub fn post_avail(&mut self, head: u16) -> u16 {
        let queue = self.as_mut();
        let last_used = queue.used.idx;

        // 环索引按队列大小（RING_SIZE）取模，与设备视角一致。
        let slot = queue.avail.idx as usize % RING_SIZE;
        queue.avail.ring[slot] = head;
        queue.avail.idx = queue.avail.idx.wrapping_add(1);

        fence(Ordering::SeqCst);
        last_used
    }

    /// 轮询等待设备产生新的 used 元素（`last_used` 之后第一个），
    /// 返回被消费的描述符链信息。
    pub fn wait_used(&mut self, last_used: u16) -> Result<UsedElement, DeviceError> {
        loop {
            let queue = self.as_mut();
            let used_idx = unsafe { addr_of!(queue.used.idx).read_volatile() };
            if used_idx != last_used {
                let slot = used_idx.wrapping_sub(1) as usize % RING_SIZE;
                let elem = unsafe { addr_of!(queue.used.ring[slot]).read_volatile() };
                return Ok(elem);
            }
            fence(Ordering::SeqCst);
            core::hint::spin_loop();
        }
    }
}

pub type Ring<T> = [T; RING_SIZE];
