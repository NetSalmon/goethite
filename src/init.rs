use core::cmp::max;

use goethite_config::PAGE_SIZE;
use goethite_macros::load_symbol;
use goethite_riscv64::addr::{PhysicalAddr, VirtualAddr};

use crate::{
    FDT_ADDR,
    device::desc::DEVICE_DESC_MANAGER,
    error::{KernelError, KernelResult},
    mm::{address_space::AS_MANAGER, alloc::FRAME_ALLOCATOR, page_table::PageTableEntryFlags},
};

pub fn init_page_table() -> KernelResult<()> {
    let flag = PageTableEntryFlags::builder()
        .set_readable(true)
        .set_writable(true)
        .set_executable(true)
        .set_validate(true)
        .build();

    let kernel_start = load_symbol!(_kernel_high_start);
    let kernel_end = load_symbol!(_kernel_high_end);

    for va in (kernel_start..kernel_end).step_by(PAGE_SIZE) {
        let pa = PhysicalAddr::from(va - goethite_config::KERNEL_OFFSET);
        AS_MANAGER
            .force()
            .write()
            .kernel_mut()
            .map(VirtualAddr::from(va), pa, flag)?;
    }

    for node in &DEVICE_DESC_MANAGER.read().all {
        for reg in &node.resources {
            for i in (reg.start..reg.end()).step_by(PAGE_SIZE) {
                let pa = PhysicalAddr::from(i);
                let va = pa.as_vaddr();

                AS_MANAGER.force().write().kernel_mut().map(va, pa, flag)?;
            }
        }
    }

    AS_MANAGER
        .read()
        .kernel_activate()
        .map_err(|err| err.into())
}

pub fn init_buddy_system() -> KernelResult<()> {
    let ptr = unsafe { FDT_ADDR }.as_ptr();

    let root = unsafe { fdt::Fdt::from_ptr(ptr) }.map_err(KernelError::FdtError)?;

    let memory = root
        .find_node("/memory")
        .ok_or(KernelError::MemoryNodeNotExist)?;

    let reg = memory.reg().ok_or(KernelError::MemoryRegNotExist)?;

    let kernel_end = load_symbol!(_end);

    // 内核及启动页表占用的物理结束地址
    // 从 _kernel_high_end 通过 as_paddr 转换，
    // 既避免直接引用低地址符号 _end 导致的 PCREL 重定位溢出，又消除硬编码魔数
    let kernel_phys_end = goethite_config::align(kernel_end, PAGE_SIZE) + PAGE_SIZE;

    let fdt_paddr = *unsafe { FDT_ADDR }.as_paddr();

    for r in reg.iter::<usize, usize>() {
        let Ok(r) = r else { continue };

        let start = r.address;
        let end = start + r.len;

        let start = max(start, kernel_phys_end);
        // 避开物理内存中的 FDT 区域
        let end = if fdt_paddr >= start && fdt_paddr < end {
            fdt_paddr
        } else {
            end
        };

        assert!(start >= 0x80200000 && end > start.into());

        FRAME_ALLOCATOR.lock().insert(start..end);
    }

    Ok(())
}
