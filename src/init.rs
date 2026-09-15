use alloc::vec::Vec;
use core::{cmp::max, ops::Range};

use goethite_config::PAGE_SIZE;
use goethite_macros::load_symbol;
use goethite_riscv64::addr::PhysicalAddr;

use crate::{
    FDT_ADDR,
    device::{desc::DEVICE_DESC_MANAGER, driver},
    error::{KernelError, KernelResult},
    mm::{address_space::AS_MANAGER, alloc::FRAME_ALLOCATOR, page_table::PageTableEntryFlags},
};

/// 探测设备树中的全部设备：匹配驱动、probe 并注册进设备管理器。
///
/// 返回成功登记的设备数。需在 [`init_page_table`] 完成设备 MMIO 映射之后调用。
pub fn init_devices() -> usize {
    driver::probe_all()
}

/// 建立内核地址空间并激活。
///
/// `memory_ranges` 是物理内存区间（来自设备树 `/memory`），本函数把它们整段
/// 线性映射到高半区（`虚拟地址 = 物理地址 + KERNEL_OFFSET`）。这样伙伴系统 /
/// SLUB 返回的高半区虚拟地址在页表切换后依然可解引用；内核镜像本身也落在该
/// 线性映射内，无需单独映射。
pub fn init_page_table(memory_ranges: &[Range<usize>]) -> KernelResult<()> {
    let flag = PageTableEntryFlags::builder()
        .set_readable(true)
        .set_writable(true)
        .set_executable(true)
        .set_validate(true)
        .build();

    for range in memory_ranges {
        let start = goethite_config::align(range.start, PAGE_SIZE);
        for pa in (start..range.end).step_by(PAGE_SIZE) {
            let pa = PhysicalAddr::from(pa);
            let va = pa.as_vaddr();

            AS_MANAGER.force().write().kernel_mut().map(va, pa, flag)?;
        }
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

/// 初始化伙伴系统：解析设备树 `/memory`，把内核镜像与 FDT 之外的物理内存交给
/// [`FRAME_ALLOCATOR`] 管理。
///
/// 返回设备树中声明的**全部物理内存区间**（含内核镜像与 FDT 所在部分），供
/// [`init_page_table`] 建立线性映射。
pub fn init_buddy_system() -> KernelResult<Vec<Range<usize>>> {
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

    let mut ranges = Vec::new();

    for r in reg.iter::<usize, usize>() {
        let Ok(r) = r else { continue };

        let start = r.address;
        let end = start + r.len;

        let alloc_start = max(start, kernel_phys_end);
        // 避开物理内存中的 FDT 区域
        let alloc_end = if fdt_paddr >= alloc_start && fdt_paddr < end {
            fdt_paddr
        } else {
            end
        };

        assert!(alloc_start >= 0x80200000 && alloc_end > alloc_start);

        // 必须先喂给伙伴系统，再向 `ranges` 里 push——后者是首次堆分配，
        // 而堆的物理页来自伙伴系统，顺序颠倒会导致无页可分配。
        FRAME_ALLOCATOR.lock().insert(alloc_start..alloc_end);
        ranges.push(start..end);
    }

    Ok(ranges)
}
