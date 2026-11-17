//! virtio-net 网卡驱动（QEMU `virtio-net-device`，virtio-mmio）。
//!
//! 队列 0 收包，队列 1 发包。只协商 `VIRTIO_NET_F_MAC` 和 modern 传输要求的
//! `VIRTIO_F_VERSION_1`：不启用校验和卸载、TSO、合并接收缓冲和控制队列，报文按
//! 普通以太网帧原样收发。modern 包头固定 12 字节（含 `num_buffers`），legacy
//! 包头是 10 字节。
//!
//! [`Device::read`] 取出一帧，没有包时返回 `0`；[`Device::write`] 发送一帧。
//! 完成状态靠轮询 used 环，不登记 PLIC 中断。
use alloc::{sync::Arc, vec::Vec};
use core::{
    ptr::addr_of,
    sync::atomic::{Ordering, fence},
};

use goethite_config::{
    PAGE_SIZE, RING_SIZE, VIRTIO_F_VERSION_1, VIRTIO_NET_F_MAC, VIRTIO_NET_HDR_LEGACY,
    VIRTIO_NET_HDR_MODERN, VIRTIO_VERSION_LEGACY, VIRTQ_AVAIL_F_NO_INTERRUPT,
};
use goethite_stdio::{eprintln, println};
use goethite_sync::SpinLock;

use crate::{
    device::{
        Device, DeviceError, DeviceInfo, DeviceResult, FileType, Major,
        desc::DeviceDesc,
        driver::Driver,
        preset, virt_to_phys,
        virtio_mmio::{
            DeviceType, VirtQueueConfig, VirtioDevice, is_device,
            queue::{Flags, VirtQueue, VirtQueueDescriptor},
        },
    },
    mm::alloc::FRAME_ALLOCATOR,
};

/// 设备专有配置区里 MAC 地址的偏移（virtio-mmio 配置区从 `0x100` 起）。
const CONFIG_MAC: usize = 0x100;
/// 接收队列索引。
const RX_QUEUE: usize = 0;
/// 发送队列索引。
const TX_QUEUE: usize = 1;
/// 预先放入接收队列的缓冲个数。
const RX_SLOTS: usize = 16;

/// 单帧以太网报文的上限（不含 virtio 包头）。按更长的 modern 包头计算，保证
/// 一页接收缓冲装得下包头加这一帧。
pub const VIRTIO_NET_MAX_FRAME: usize = PAGE_SIZE - VIRTIO_NET_HDR_MODERN;

const _: () = assert!(RX_SLOTS > 0 && RX_SLOTS <= RING_SIZE);

/// virtio-net 包头。modern 布局含末尾的 `num_buffers`；legacy 只用前 10 个字节。
#[repr(C)]
#[derive(Clone, Copy)]
struct VirtioNetHdr {
    flags: u8,
    gso_type: u8,
    hdr_len: u16,
    gso_size: u16,
    csum_start: u16,
    csum_offset: u16,
    num_buffers: u16,
}

impl VirtioNetHdr {
    const fn zeroed() -> Self {
        Self {
            flags: 0,
            gso_type: 0,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
            num_buffers: 0,
        }
    }
}

const _: () = assert!(core::mem::size_of::<VirtioNetHdr>() == VIRTIO_NET_HDR_MODERN);

/// 接收队列上已经交给设备的页。
struct RxState {
    /// 每个接收槽一页，下标就是描述符 id。
    bufs: Vec<usize>,
    /// 驱动已经消费的 used 环索引。
    last_used: u16,
}

/// 从 used 环取出一帧后的结果。
enum Recv {
    /// 缓冲放不下一整帧，或设备写来的长度不合法。页需要归还接收队列。
    Reject { id: usize, page: usize },
    /// 已经把整帧拷进调用方缓冲。
    Frame { id: usize, page: usize, len: usize },
}

/// virtio-net 设备：MMIO 描述、收发队列，以及预先放好的接收页。
pub struct VirtioNet {
    pub device: DeviceInfo,
    pub queues: SpinLock<Option<Vec<VirtQueue>>>,
    rx: SpinLock<RxState>,
    /// 发送时放在设备可见内存里的包头。同一时刻只有一个发送在飞。
    tx_hdr: SpinLock<VirtioNetHdr>,
}

impl Device for VirtioNet {
    /// 拷贝一帧以太网报文到 `buf`，返回帧长。当前没有包时返回 `0`。
    ///
    /// `buf` 短于这一帧时丢弃该帧并返回 [`DeviceError::Io`]，避免把截断的报文
    /// 交给上层。调用方应至少准备 [`VIRTIO_NET_MAX_FRAME`] 字节。
    fn read(&self, buf: &mut [u8]) -> DeviceResult {
        self.recv_frame(buf)
    }

    /// 发送 `buf` 中的一帧以太网报文（驱动自己补零包头），返回帧长。
    fn write(&self, buf: &[u8]) -> DeviceResult {
        self.send_frame(buf)
    }
}

impl VirtioDevice for VirtioNet {
    const DEVICE_TYPE: DeviceType = DeviceType::NET;

    fn device(&self) -> DeviceInfo {
        self.device
    }

    fn queues(&self) -> &SpinLock<Option<Vec<VirtQueue>>> {
        &self.queues
    }

    fn queue_count(&self) -> u32 {
        2
    }

    /// 只要 MAC 和 `VIRTIO_F_VERSION_1`。其余特性会改变包头或描述符环的布局。
    fn negotiate(&self, features_low: u32, features_high: u32) -> (u32, u32) {
        (
            features_low & VIRTIO_NET_F_MAC,
            features_high & VIRTIO_F_VERSION_1,
        )
    }

    fn negotiate_legacy(&self, features: u32) -> u32 {
        features & VIRTIO_NET_F_MAC
    }

    fn prepare_queues(&self, queues: &mut [VirtQueue]) -> Result<(), DeviceError> {
        if queues.len() < 2 {
            return Err(DeviceError::NotReady);
        }
        for queue in queues.iter_mut() {
            queue.as_mut().avail.flags = VIRTQ_AVAIL_F_NO_INTERRUPT;
        }
        self.prime_rx(&mut queues[RX_QUEUE])
    }

    fn activate(&self) -> Result<(), DeviceError> {
        self.notify_queue(RX_QUEUE as u32);
        Ok(())
    }
}

impl From<DeviceInfo> for VirtioNet {
    fn from(device: DeviceInfo) -> Self {
        VirtioNet {
            device,
            queues: SpinLock::new(None),
            rx: SpinLock::new(RxState {
                bufs: Vec::new(),
                last_used: 0,
            }),
            tx_hdr: SpinLock::new(VirtioNetHdr::zeroed()),
        }
    }
}

impl Drop for VirtioNet {
    fn drop(&mut self) {
        // 写 0 到 status 让设备停止 DMA，再归还接收页。
        VirtQueueConfig {
            device: self.device,
        }
        .write_status(0);
        fence(Ordering::SeqCst);
        self.free_rx();
    }
}

impl VirtioNet {
    /// 从设备描述探测：要求 `device_id` 为网卡。
    pub fn from_desc(desc: &DeviceDesc) -> Option<Self> {
        let device = DeviceInfo::from_desc(desc)?;
        is_device(device, Self::DEVICE_TYPE).then(|| device.into())
    }

    /// 配置空间里的 MAC 地址。协商了 `VIRTIO_NET_F_MAC` 时这 6 个字节有效。
    pub fn mac(&self) -> [u8; 6] {
        let lo: u32 = self.device.mmio.read(CONFIG_MAC);
        let hi: u16 = self.device.mmio.read(CONFIG_MAC + 4);
        let mut mac = [0u8; 6];
        mac[..4].copy_from_slice(&lo.to_le_bytes());
        mac[4..].copy_from_slice(&hi.to_le_bytes());
        mac
    }

    /// 当前传输下的包头长度。modern 固定带 `num_buffers`，legacy 则没有。
    fn hdr_size(&self) -> usize {
        let version = VirtQueueConfig {
            device: self.device(),
        }
        .version();
        if version == VIRTIO_VERSION_LEGACY {
            VIRTIO_NET_HDR_LEGACY
        } else {
            VIRTIO_NET_HDR_MODERN
        }
    }

    fn ensure_ready(&self) -> Result<(), DeviceError> {
        if self.queues().lock().is_some() {
            Ok(())
        } else {
            self.handshake()
        }
    }

    /// 分配接收页并挂上队列 0。设备尚未 `DRIVER_OK`，不会先把这些页收走。
    fn prime_rx(&self, queue: &mut VirtQueue) -> Result<(), DeviceError> {
        self.free_rx();

        let mut bufs = Vec::with_capacity(RX_SLOTS);
        for slot in 0..RX_SLOTS {
            let page = match alloc_rx_page() {
                Ok(page) => page,
                Err(err) => {
                    for page in bufs {
                        FRAME_ALLOCATOR.lock().dealloc_frame(page, PAGE_SIZE);
                    }
                    return Err(err);
                }
            };
            post_rx(queue, page, slot as u16);
            bufs.push(page);
        }

        let mut rx = self.rx.lock();
        rx.bufs = bufs;
        rx.last_used = 0;
        Ok(())
    }

    fn free_rx(&self) {
        let bufs = core::mem::take(&mut self.rx.lock().bufs);
        self.rx.lock().last_used = 0;
        for page in bufs {
            FRAME_ALLOCATOR.lock().dealloc_frame(page, PAGE_SIZE);
        }
    }

    fn recv_frame(&self, buf: &mut [u8]) -> Result<usize, DeviceError> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.ensure_ready()?;
        let hdr_size = self.hdr_size();

        let mut queues = self.queues().lock();
        let Some(rxq) = queues.as_mut().and_then(|queues| queues.get_mut(RX_QUEUE)) else {
            return Err(DeviceError::NotReady);
        };

        let outcome = {
            let queue = rxq.as_mut();
            // SAFETY: used 环由设备写入，读索引必须是易失读。
            let used_idx = unsafe { addr_of!(queue.used.idx).read_volatile() };
            let mut rx = self.rx.lock();
            if rx.last_used == used_idx {
                return Ok(0);
            }
            fence(Ordering::SeqCst);

            let slot = rx.last_used as usize % RING_SIZE;
            // SAFETY: 该槽已由设备发布，读元素同样要绕过缓存的普通读。
            let elem = unsafe { addr_of!(queue.used.ring[slot]).read_volatile() };
            rx.last_used = rx.last_used.wrapping_add(1);

            let id = elem.id as usize;
            if id >= rx.bufs.len() {
                return Err(DeviceError::Io);
            }
            let page = rx.bufs[id];
            let total = (elem.len as usize).min(PAGE_SIZE);
            let frame_len = total.saturating_sub(hdr_size);
            // 没协商合并接收。QEMU 在 modern 包头里把 num_buffers 留成 0 或填 1，
            // 都表示这一帧只占当前这一个缓冲；大于 1 就拼不回来。
            let num_buffers = if hdr_size == VIRTIO_NET_HDR_MODERN {
                u16::from_le(unsafe {
                    ((page as *const u8).add(VIRTIO_NET_HDR_LEGACY) as *const u16).read_volatile()
                })
            } else {
                1
            };
            if total < hdr_size || buf.len() < frame_len || num_buffers > 1 {
                Recv::Reject { id, page }
            } else {
                fence(Ordering::SeqCst);
                // SAFETY: `page` 是本设备持有的接收页，帧落在包头之后且不超过 `buf`。
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        (page as *const u8).add(hdr_size),
                        buf.as_mut_ptr(),
                        frame_len,
                    );
                }
                Recv::Frame {
                    id,
                    page,
                    len: frame_len,
                }
            }
        };

        match outcome {
            Recv::Reject { id, page } => {
                post_rx(rxq, page, id as u16);
                self.notify_queue(RX_QUEUE as u32);
                self.ack_interrupt();
                Err(DeviceError::Io)
            }
            Recv::Frame { id, page, len } => {
                post_rx(rxq, page, id as u16);
                self.notify_queue(RX_QUEUE as u32);
                self.ack_interrupt();
                Ok(len)
            }
        }
    }

    fn send_frame(&self, frame: &[u8]) -> Result<usize, DeviceError> {
        if frame.is_empty() || frame.len() > VIRTIO_NET_MAX_FRAME {
            return Err(DeviceError::Io);
        }
        // 设备靠 DMA 读这帧。易失读迫使编译器先把调用方写入的字节落到内存，
        // 否则 LTO 会把「只被取了地址」的报文当成死存储删掉。
        unsafe {
            core::ptr::read_volatile(frame.as_ptr());
            core::ptr::read_volatile(frame.as_ptr().add(frame.len() - 1));
        }
        fence(Ordering::SeqCst);
        self.ensure_ready()?;

        let mut tx_hdr = self.tx_hdr.lock();
        let hdr = &mut *tx_hdr;
        // SAFETY: 包头驻留在设备对象里，发送完成前不会被搬走或复用。
        unsafe { core::ptr::write_volatile(hdr, VirtioNetHdr::zeroed()) };
        let hdr_addr = virt_to_phys(hdr as *mut VirtioNetHdr as usize) as u64;

        let mut queues = self.queues().lock();
        let Some(tx) = queues.as_mut().and_then(|queues| queues.get_mut(TX_QUEUE)) else {
            return Err(DeviceError::NotReady);
        };

        let mut flags = Flags::new();
        flags.set_next(true);
        tx.as_mut().desc[0] = VirtQueueDescriptor {
            addr: hdr_addr,
            len: self.hdr_size() as u32,
            flags,
            next: 1,
        };

        tx.as_mut().desc[1] = VirtQueueDescriptor {
            addr: virt_to_phys(frame.as_ptr() as usize) as u64,
            len: frame.len() as u32,
            flags: Flags::new(),
            next: 0,
        };

        let last_used = tx.post_avail(0);
        self.notify_queue(TX_QUEUE as u32);
        let elem = tx.wait_used(last_used)?;
        if elem.id != 0 {
            return Err(DeviceError::Io);
        }
        self.ack_interrupt();
        Ok(frame.len())
    }

    fn ack_interrupt(&self) {
        let cfg = VirtQueueConfig {
            device: self.device(),
        };
        let status = cfg.interrupt_status();
        if status != 0 {
            cfg.write_interrupt_ack(status);
        }
    }
}

/// 把一页可写缓冲挂回接收队列。
fn post_rx(queue: &mut VirtQueue, page: usize, head: u16) {
    let mut flags = Flags::new();
    flags.set_write(true);
    queue.as_mut().desc[head as usize] = VirtQueueDescriptor {
        addr: virt_to_phys(page) as u64,
        len: PAGE_SIZE as u32,
        flags,
        next: 0,
    };
    queue.post_avail(head);
}

fn alloc_rx_page() -> Result<usize, DeviceError> {
    let page = FRAME_ALLOCATOR
        .lock()
        .alloc_frame(PAGE_SIZE)
        .ok_or(DeviceError::AllocFailed)?;
    // SAFETY: 伙伴系统返回一整页可写的高半区地址。
    unsafe { core::ptr::write_bytes(page as *mut u8, 0, PAGE_SIZE) };
    Ok(page)
}

/// virtio-net 的驱动：匹配 `virtio,mmio` 且 `device_id` 为网卡。
pub struct VirtioNetDriver;

impl Driver for VirtioNetDriver {
    fn name(&self) -> &'static str {
        "virtio-mmio-net"
    }

    fn compatibles(&self) -> &'static [&'static str] {
        &["virtio,mmio"]
    }

    fn major(&self) -> Major {
        Major::Static(preset::NET)
    }

    fn file_type(&self) -> FileType {
        FileType::CHR
    }

    fn probe(&self, desc: &Arc<DeviceDesc>) -> Result<Arc<dyn Device>, DeviceError> {
        let Some(net) = VirtioNet::from_desc(desc) else {
            return Err(DeviceError::NotReady);
        };
        // 先放进 `Arc`，接收页和发送包头的地址在握手之后不再移动。
        let net = Arc::new(net);
        if let Err(err) = net.handshake() {
            eprintln!("virtio-net: handshake failed: {err}");
            return Err(err);
        }
        let mac = net.mac();
        println!(
            "virtio-net: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
        Ok(net)
    }
}
