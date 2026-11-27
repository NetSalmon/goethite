//! 虚拟内存与标准 I/O 字符设备。
//!
//! 在 [`preset::MEM`] 主设备号下注册常见虚拟字符设备，配合 [`crate::fs::device::DeviceFs`]
//! 暴露到 `/dev`：
//!
//! - `/dev/null`（[`NullDevice`]）：读始终返回 EOF（0 字节），写丢弃全部数据；
//! - `/dev/zero`（[`ZeroDevice`]）：读填充全 0 字节，写丢弃全部数据；
//! - `/dev/stdin`（[`StdinDevice`]）：从调试控制台读取输入字节（只读）；
//! - `/dev/stdout`（[`StdoutDevice`]）：向调试控制台输出字节（只写）；
//! - `/dev/stderr`（[`StderrDevice`]）：向调试控制台输出错误字节（只写）。

use alloc::sync::Arc;

use goethite_config::KERNEL_OFFSET;
use goethite_riscv64::sbi::{dbcn, legacy};

use crate::device::{
    DEVICE_MANAGER, Device, DeviceError, DeviceResult, FileType, Major, Mode,
    desc::DeviceDesc,
    driver::{self, Driver},
    preset,
};

/// `/dev/null`：读返回 0（EOF），写丢弃所有字节。
pub struct NullDevice;

impl Device for NullDevice {
    fn read(&self, _buf: &mut [u8]) -> DeviceResult {
        Ok(0)
    }

    fn write(&self, buf: &[u8]) -> DeviceResult {
        Ok(buf.len())
    }
}

/// `/dev/zero`：读返回全 0 字节，写丢弃所有字节。
pub struct ZeroDevice;

impl Device for ZeroDevice {
    fn read(&self, buf: &mut [u8]) -> DeviceResult {
        buf.fill(0);
        Ok(buf.len())
    }

    fn write(&self, buf: &[u8]) -> DeviceResult {
        Ok(buf.len())
    }
}

/// `/dev/stdin`：从控制台读取可用字节（非阻塞，当前无输入时返回 0）。
pub struct StdinDevice;

impl Device for StdinDevice {
    fn read(&self, buf: &mut [u8]) -> DeviceResult {
        let mut count = 0;
        for byte in buf.iter_mut() {
            let Some(ch) = legacy::console_getchar() else {
                break;
            };
            *byte = ch;
            count += 1;
        }
        Ok(count)
    }
}

/// 将字节切片经 SBI DBCN 写出到控制台。
///
/// 若 `buf` 位于高半区线性映射之外（例如低地址或非连续缓冲区），通过内核栈上小缓冲
/// 分块转发，避免向 SBI 传入非法物理地址。
fn write_console(buf: &[u8]) -> DeviceResult {
    if buf.is_empty() {
        return Ok(0);
    }

    if (buf.as_ptr() as usize) >= KERNEL_OFFSET {
        dbcn::write_bytes(buf).map_err(|_| DeviceError::Io)?;
    } else {
        let mut scratch = [0u8; 64];
        for chunk in buf.chunks(scratch.len()) {
            scratch[..chunk.len()].copy_from_slice(chunk);
            dbcn::write_bytes(&scratch[..chunk.len()]).map_err(|_| DeviceError::Io)?;
        }
    }
    Ok(buf.len())
}

/// `/dev/stdout`：向控制台输出标准输出流。
pub struct StdoutDevice;

impl Device for StdoutDevice {
    fn write(&self, buf: &[u8]) -> DeviceResult {
        write_console(buf)
    }
}

/// `/dev/stderr`：向控制台输出标准错误流。
pub struct StderrDevice;

impl Device for StderrDevice {
    fn write(&self, buf: &[u8]) -> DeviceResult {
        write_console(buf)
    }
}

/// 虚拟内存 / 标准 I/O 设备的占位驱动，负责申请 [`preset::MEM`] 主设备号。
pub struct VirtualMemDriver;

impl Driver for VirtualMemDriver {
    fn name(&self) -> &'static str {
        "virtual-mem"
    }

    fn compatibles(&self) -> &'static [&'static str] {
        &[]
    }

    fn major(&self) -> Major {
        Major::Static(preset::MEM)
    }

    fn file_type(&self) -> FileType {
        FileType::CHR
    }

    fn probe(&self, _desc: &Arc<DeviceDesc>) -> Result<Arc<dyn Device>, DeviceError> {
        Err(DeviceError::Unsupported)
    }
}

fn chr_mode(read: bool, write: bool) -> Mode {
    Mode::builder()
        .set_file_type(FileType::CHR)
        .set_user_read(read)
        .set_user_write(write)
        .set_group_read(read)
        .set_group_write(write)
        .set_other_read(read)
        .set_other_write(write)
        .build()
}

/// 注册内置虚拟设备（`null`、`zero`、`stdin`、`stdout`、`stderr`）到 [`DEVICE_MANAGER`]。
///
/// 返回成功注册的设备数量。
pub fn init() -> usize {
    let driver: Arc<dyn Driver> = Arc::new(VirtualMemDriver);
    let Ok(major) = driver::register(driver.clone()) else {
        return 0;
    };

    let entries: [(&str, Arc<dyn Device>, Mode); 5] = [
        ("null", Arc::new(NullDevice), chr_mode(true, true)),
        ("zero", Arc::new(ZeroDevice), chr_mode(true, true)),
        ("stdin", Arc::new(StdinDevice), chr_mode(true, false)),
        ("stdout", Arc::new(StdoutDevice), chr_mode(false, true)),
        ("stderr", Arc::new(StderrDevice), chr_mode(false, true)),
    ];

    let mut mgr = DEVICE_MANAGER.write();
    let mut registered = 0;
    for (name, device, mode) in entries {
        let desc = Arc::new(DeviceDesc::virtual_device(name));
        if mgr
            .register(major, desc, device, driver.clone(), mode)
            .is_ok()
        {
            registered += 1;
        }
    }
    registered
}
