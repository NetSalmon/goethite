//! ns16550a UART 驱动。
//!
//! 通过 `mmio_regs!` 访问 RBR/THR、IER、FCR、LCR、LSR 寄存器，支持 FIFO /
//! 行控制初始化、字符收发，并以 [`Driver`] 形式匹配设备树中的 `ns16550a`
//! 节点、以字符设备（[`FileType::CHR`]）登记到 [`preset::TTY`] 主设备号下。
use alloc::sync::Arc;
use core::fmt;

use goethite_macros::bits;

use crate::{
    device::{
        Device, DeviceError, DeviceInfo, DeviceNaming, DeviceResult, FileType, Major,
        desc::DeviceDesc, driver::Driver, preset,
    },
    mmio_regs,
};

/// ns16550a 设备：寄存器窗口与 FIFO / 行控制状态。
pub struct Ns16550a {
    pub device: DeviceInfo,
}

mmio_regs! {
    Ns16550a: [
        rbr_thr => 0,
        ier: IerStatus => 1,
        fcr: FcrStatus => 2,
        lcr: LcrStatus => 3,
        lsr: LsrStatus => 5,
    ]
}

bits! {
    pub type LsrStatus: u8 {
        dr: 0,      // Data Ready
        thre: 5,    // Transmit Holding Register Empty
    }
}

bits! {
    pub type FcrStatus: u8 {
        enable: 0,
        clear_rx: 1,
        clear_tx: 2,
    }
}

bits! {
    pub type LcrStatus: u8 {
        word_len_lo: 0,
        word_len_hi: 1,
        stop_bits: 2,
        dlab: 7,
    }
}

bits! {
    pub type IerStatus: u8 { }
}

impl Device for Ns16550a {
    /// 顺序读取可用的字符到 `buf`，返回实际读到的字节数。
    fn read(&self, buf: &mut [u8]) -> DeviceResult {
        let mut read = 0;
        for byte in buf.iter_mut() {
            let Some(value) = self.rbr_thr_if_ready() else {
                break;
            };
            *byte = value;
            read += 1;
        }
        Ok(read)
    }

    /// 顺序写出 `buf` 中的全部字节，返回写入的字节数。
    fn write(&self, buf: &[u8]) -> DeviceResult {
        for &byte in buf {
            while !self.lsr().thre() {}
            self.write_rbr_thr(byte);
        }
        Ok(buf.len())
    }
}

/// 让 UART 满足 `core::fmt::Write`，供 `print!` / `println!` 输出。
impl fmt::Write for Ns16550a {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let _ = Device::write(self, s.as_bytes());
        Ok(())
    }
}

impl Ns16550a {
    /// 初始化 FIFO（使能并清空收发队列）与行控制（8 位字长）。
    pub fn init(&self) {
        let mut fcr = FcrStatus::new();
        fcr.set_enable(true);
        fcr.set_clear_rx(true);
        fcr.set_clear_tx(true);
        self.write_fcr(fcr);

        let mut lcr = LcrStatus::new();
        lcr.set_word_len_lo(true);
        lcr.set_word_len_hi(true);
        self.write_lcr(lcr);
    }

    fn rbr_thr_if_ready(&self) -> Option<u8> {
        if self.lsr().dr() {
            self.rbr_thr()
        } else {
            None
        }
    }

    /// 从设备描述构造 UART（要求存在 MMIO 窗口）。
    pub fn from_desc(desc: &DeviceDesc) -> Option<Self> {
        Some(Self {
            device: DeviceInfo::from_desc(desc)?,
        })
    }
}

/// ns16550a UART 的驱动：匹配 compatible `ns16550a`，登记为 TTY 字符设备。
pub struct Ns16550aDriver;

impl Driver for Ns16550aDriver {
    fn name(&self) -> &'static str {
        "ns16550a"
    }

    fn compatibles(&self) -> &'static [&'static str] {
        &["ns16550a"]
    }

    fn major(&self) -> Major {
        Major::Static(preset::TTY)
    }

    fn file_type(&self) -> FileType {
        FileType::CHR
    }

    fn naming(&self) -> DeviceNaming {
        DeviceNaming::Numeric("ttyS")
    }

    fn probe(&self, desc: &Arc<DeviceDesc>) -> Result<Arc<dyn Device>, DeviceError> {
        let uart = Ns16550a::from_desc(desc).ok_or(DeviceError::NotReady)?;
        uart.init();
        Ok(Arc::new(uart))
    }
}
