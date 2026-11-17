//! 用户程序加载（ELF64）。
//!
//! 把 [`goethite_elf`] 解析出的 ELF 程序头映射为 [`LoadSegment`]，交由
//! [`MemorySet::with_segments`] 分配物理帧并建立页表映射，随后把各 `PT_LOAD`
//! 段的内容从文件缓冲区写入对应物理帧，其余部分保持零填充（即 `.bss` 与段尾）。
//!
//! 本模块只负责「把一段 ELF 映像变成可运行的地址空间」：初始用户栈的构建、
//! 任务 / 上下文创建与系统调用接线不在其内，调用方拿到 [`LoadedImage`] 后自行
//! 决定如何执行。

use alloc::vec::Vec;
use core::ptr;

use goethite_config::{PAGE_SIZE, align};
use goethite_elf::{ElfError, ElfHeader, ProgramHeader, ProgramType};
use goethite_riscv64::addr::VirtualAddr;

use crate::{
    mm::page_table::PageTableEntryFlags,
    proc::vma::{LoadSegment, MemSetError, MemorySet, Vma, VmaKind},
};

/// ELF 加载可能出现的错误。
#[derive(Debug, Clone, thiserror::Error)]
pub enum ExecError {
    /// ELF 文件头 / 程序头解析失败。
    #[error(transparent)]
    Elf(#[from] ElfError),
    /// 地址空间构建失败（物理帧分配或页表映射）。
    #[error(transparent)]
    MemSet(#[from] MemSetError),
    /// ELF 中没有任何 `PT_LOAD` 段。
    #[error("no loadable segment in ELF")]
    NoLoadableSegment,
    /// 程序头表项越出文件缓冲区。
    #[error("program header table out of bounds")]
    ProgramHeaderOutOfBounds,
    /// 段声明的文件范围越出文件缓冲区。
    #[error("ELF segment exceeds file size")]
    SegmentOutOfBounds,
    /// 段的文件大小大于内存大小（`p_filesz > p_memsz`）。
    #[error("invalid ELF segment: p_filesz > p_memsz")]
    InvalidSegment,
    /// 虚拟地址或长度溢出。
    #[error("virtual address overflow")]
    AddressOverflow,
}

/// 一个待加载段的内部计划：记录 ELF 原始字段（含未页对齐的虚拟地址），
/// 用于构建 [`LoadSegment`] 与把文件内容写入物理帧。
#[derive(Debug, Clone, Copy)]
struct SegmentPlan {
    /// ELF 原始虚拟地址（`p_vaddr`，可能未页对齐）。
    vaddr: usize,
    /// 向下页对齐后的虚拟地址，即 [`LoadSegment::vaddr`]。
    vaddr_aligned: usize,
    /// 页对齐后的内存跨度（含 `p_vaddr` 的页内偏移），即 [`LoadSegment::memsz`]。
    memsz: usize,
    /// 段在文件中的字节偏移（`p_offset`）。
    file_offset: usize,
    /// 段在文件中的字节大小（`p_filesz`）。
    file_size: usize,
    /// 段权限（`r/w/x`，始终带用户位）。
    perms: PageTableEntryFlags,
    /// 段类型（可执行段记为 [`VmaKind::Text`]，否则 [`VmaKind::Data`]）。
    kind: VmaKind,
}

/// 加载完成的用户程序映像。
pub struct LoadedImage {
    /// 承载本程序的内存集合（含虚拟布局与页表映射）。
    pub memory_set: MemorySet,
    /// 程序入口虚拟地址（`e_entry`）。
    pub entry: VirtualAddr,
    /// 由程序头解析出的加载段描述符，顺序与 [`MemorySet::vmas`] 一致。
    pub segments: Vec<LoadSegment>,
}

/// 解析并加载一段 ELF64 映像。
///
/// 依次完成：读取并校验文件头、遍历程序头表挑出 `PT_LOAD` 段、按段构建
/// [`MemorySet`]，最后把各段的文件内容写入已映射的物理帧（段尾与 `.bss`
/// 保持零填充）。
///
/// # Errors
///
/// - [`ExecError::Elf`]：不是合法的 64 位小端 RISC-V ELF。
/// - [`ExecError::NoLoadableSegment`]：没有可加载段。
/// - [`ExecError::ProgramHeaderOutOfBounds`] / [`ExecError::SegmentOutOfBounds`]：
///   程序头表或段数据越出缓冲区。
/// - [`ExecError::InvalidSegment`]：`p_filesz > p_memsz`。
/// - [`ExecError::MemSet`]：物理帧分配或页表映射失败。
pub fn load_elf(buffer: &[u8]) -> Result<LoadedImage, ExecError> {
    let header = ElfHeader::try_from(buffer)?;
    header.validate()?;

    let plans = plan_segments(&header, buffer)?;

    let segments: Vec<LoadSegment> = plans
        .iter()
        .map(|plan| LoadSegment {
            vaddr: VirtualAddr::from(plan.vaddr_aligned),
            memsz: plan.memsz,
            file_offset: plan.file_offset,
            perms: plan.perms,
            kind: plan.kind,
        })
        .collect();

    let memory_set = MemorySet::with_segments(&segments)?;

    // `with_segments` 按段顺序 push VMA，与 `plans` 一一对应。
    for (plan, vma) in plans.iter().zip(memory_set.vmas.iter()) {
        fill_segment(buffer, plan, vma);
    }

    Ok(LoadedImage {
        memory_set,
        entry: VirtualAddr::from(header.entry as usize),
        segments,
    })
}

/// 遍历程序头表，挑出所有 `PT_LOAD` 段并转换为 [`SegmentPlan`]。
///
/// 会校验程序头表项、段文件范围是否落在 `buffer` 内，并把每段的虚拟地址
/// 向下页对齐、内存跨度相应扩展。
fn plan_segments(header: &ElfHeader, buffer: &[u8]) -> Result<Vec<SegmentPlan>, ExecError> {
    let ph_offset = header.program_header_offset as usize;
    let ph_entry_size = header.program_header_entry_size as usize;
    let ph_num = header.program_header_num as usize;

    let mut plans = Vec::new();

    for i in 0..ph_num {
        let start = ph_offset
            .checked_add(
                i.checked_mul(ph_entry_size)
                    .ok_or(ExecError::ProgramHeaderOutOfBounds)?,
            )
            .ok_or(ExecError::ProgramHeaderOutOfBounds)?;
        let end = start
            .checked_add(ph_entry_size)
            .ok_or(ExecError::ProgramHeaderOutOfBounds)?;

        let bytes = buffer
            .get(start..end)
            .ok_or(ExecError::ProgramHeaderOutOfBounds)?;
        let ph = ProgramHeader::try_from(bytes)?;

        if ph.r#type != ProgramType::LOAD {
            continue;
        }

        let file_offset = ph.offset as usize;
        let file_size = ph.file_size as usize;
        let mem_size = ph.mem_size as usize;
        let vaddr = ph.virtual_addr as usize;

        if file_size > mem_size {
            return Err(ExecError::InvalidSegment);
        }

        let file_end = file_offset
            .checked_add(file_size)
            .ok_or(ExecError::SegmentOutOfBounds)?;
        if file_end > buffer.len() {
            return Err(ExecError::SegmentOutOfBounds);
        }

        // 段虚拟地址可能未页对齐：向下对齐并把页内偏移计入内存跨度，保证
        // 映射覆盖整个段（含文件内容与 `.bss`）。
        let vaddr_aligned = vaddr & !(PAGE_SIZE - 1);
        let delta = vaddr - vaddr_aligned;
        let span = mem_size
            .checked_add(delta)
            .ok_or(ExecError::AddressOverflow)?;
        let memsz = align(span, PAGE_SIZE);
        if memsz == 0 {
            continue;
        }

        let perms = PageTableEntryFlags::builder()
            .set_readable(ph.flags.read())
            .set_writable(ph.flags.write())
            .set_executable(ph.flags.execute())
            .set_user(true)
            .set_accessed(true)
            .set_dirty(true)
            .build();

        let kind = if ph.flags.execute() {
            VmaKind::Text
        } else {
            VmaKind::Data
        };

        plans.push(SegmentPlan {
            vaddr,
            vaddr_aligned,
            memsz,
            file_offset,
            file_size,
            perms,
            kind,
        });
    }

    if plans.is_empty() {
        return Err(ExecError::NoLoadableSegment);
    }

    Ok(plans)
}

/// 把一段的文件内容写入其已映射的物理帧：先整段清零，再按虚拟地址重叠区间
/// 拷贝 `buffer[file_offset..file_offset + file_size]`。
///
/// `vma.backing` 中的每个 [`crate::proc::vma::MappedRegion`] 记录了该物理块被映射到的
/// 虚拟地址起点，据此可把「段虚拟地址区间」换算为「物理块内偏移」。
fn fill_segment(buffer: &[u8], plan: &SegmentPlan, vma: &Vma) {
    for mapped in &vma.backing {
        // SAFETY: 后备物理块由本进程独占持有，`Region` 的高半区虚拟地址可直接写入；
        // 长度取自该块自身大小。
        unsafe {
            ptr::write_bytes(
                mapped.region.as_mut_ptr::<u8>(),
                0,
                mapped.region.byte_size(),
            );
        }
    }

    if plan.file_size == 0 {
        return;
    }

    let file = &buffer[plan.file_offset..plan.file_offset + plan.file_size];
    let content_start = plan.vaddr;
    let content_end = plan.vaddr + plan.file_size;

    for mapped in &vma.backing {
        let region_start = usize::from(mapped.vaddr);
        let region_end = region_start + mapped.region.byte_size();

        let start = region_start.max(content_start);
        let end = region_end.min(content_end);
        if start >= end {
            continue;
        }

        // SAFETY: `[start, end)` 同时落在本物理块与文件内容区间内，
        // 源 / 目标区间分别由 `file` 与 `Region` 保证有效且不重叠。
        unsafe {
            let dst = mapped.region.as_mut_ptr::<u8>().add(start - region_start);
            let src = file.as_ptr().add(start - content_start);
            ptr::copy_nonoverlapping(src, dst, end - start);
        }
    }
}
