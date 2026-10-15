use core::panic::PanicInfo;

use fdt::FdtError;
use goethite_fs::{device::IoError as FsIoError, error::FsError};
use goethite_riscv64::sbi::srst::{ResetReason, ResetType};
use goethite_stdio::eprintln;

use crate::{
    mm::{address_space::ASpaceError, object::AllocError},
    proc::exec::ExecError,
};

#[panic_handler]
fn panic_handler(info: &PanicInfo<'_>) -> ! {
    eprintln!("kernel panic: {}", info);
    let _ = goethite_riscv64::sbi::srst::system_reset(ResetType::SHUTDOWN, ResetReason::NONE);

    loop {
        core::hint::spin_loop();
    }
}

#[derive(thiserror::Error, Debug)]
pub enum KernelError {
    #[error(transparent)]
    MemoryAllocError(#[from] AllocError),
    #[error(transparent)]
    AddressSpaceError(#[from] ASpaceError),
    /// 用户程序（ELF）加载错误。
    #[error(transparent)]
    ExecError(#[from] ExecError),
    #[error("memory reg not exist")]
    MemoryRegNotExist,
    #[error("memory node not exist")]
    MemoryNodeNotExist,
    #[error("{0}")]
    FdtError(FdtError),
    /// 设备树中没有任何块设备可供挂载根文件系统。
    #[error("no block device found")]
    NoBlockDevice,
    /// 块设备上不是 MINIX v1 文件系统。
    #[error("block device is not a minix v1 filesystem")]
    NotMinixFs,
    /// 文件系统层 I/O 错误。
    #[error(transparent)]
    FsIoError(#[from] FsIoError),
    /// 文件系统层通用错误。
    #[error(transparent)]
    FsError(#[from] FsError),
}

pub type KernelResult<T> = Result<T, KernelError>;
