use core::panic::PanicInfo;

use fdt::FdtError;
use goethite_riscv64::sbi::srst::{ResetReason, ResetType};
use goethite_stdio::eprintln;

use crate::mm::{address_space::ASpaceError, object::AllocError};

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
    #[error("memory reg not exist")]
    MemoryRegNotExist,
    #[error("memory node not exist")]
    MemoryNodeNotExist,
    #[error("{0}")]
    FdtError(FdtError),
}

pub type KernelResult<T> = Result<T, KernelError>;
