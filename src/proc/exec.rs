//! 用户程序加载（ELF64）。
//!
//! 把 [`goethite_elf`] 解析出的 ELF 程序头映射为 [`LoadSegment`]，交由
//! [`MemorySet::with_segments`] 分配物理帧并建立页表映射，随后把各 `PT_LOAD`
//! 段的内容从文件缓冲区写入对应物理帧，其余部分保持零填充（即 `.bss` 与段尾）。
//!
//! 本模块只负责「把一段 ELF 映像变成可运行的地址空间」：初始用户栈的构建、
//! 任务 / 上下文创建与系统调用接线不在其内，调用方拿到 [`LoadedImage`] 后自行
//! 决定如何执行。

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::ptr;

use goethite_config::{PAGE_SIZE, USER_STACK_LOWER_BOUND, USER_STACK_SIZE, USER_STACK_TOP, align};
use goethite_elf::{ElfError, ElfHeader, ProgramHeader, ProgramType};
use goethite_fs::Path;
use goethite_riscv64::{
    addr::VirtualAddr,
    registers::{Registers, values::SstatusValue},
};
use goethite_stack::{AT_ENTRY, AT_PAGESZ, Input, required_len, write};

use crate::{
    fs,
    mm::{page_table::PageTableEntryFlags, stack::Stack},
    proc::{
        Context, Task, TaskId, TaskStatus, alloc_tid,
        schedule::SCHEDULER,
        vma::{LoadSegment, MemorySetError, MemorySet, Vma, VmaKind},
    },
};

/// ELF 加载可能出现的错误。
#[derive(Debug, Clone, thiserror::Error)]
pub enum ExecError {
    /// ELF 文件头 / 程序头解析失败。
    #[error(transparent)]
    Elf(#[from] ElfError),
    /// 地址空间构建失败（物理帧分配或页表映射）。
    #[error(transparent)]
    MemSet(#[from] MemorySetError),
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

/// 将数据切片写入指定 VMA 的虚拟地址 `user_vaddr` 处。
fn copy_to_vma(vma: &Vma, user_vaddr: usize, data: &[u8]) {
    let mut offset = 0;
    while offset < data.len() {
        let cur_va = user_vaddr + offset;
        let mr = vma
            .backing
            .iter()
            .find(|mr| {
                let start = usize::from(mr.vaddr);
                let end = start + mr.region.byte_size();
                start <= cur_va && cur_va < end
            })
            .expect("user address must fall within VMA backing regions");
        let mr_start = usize::from(mr.vaddr);
        let mr_offset = cur_va - mr_start;
        let chunk_len = (mr.region.byte_size() - mr_offset).min(data.len() - offset);
        unsafe {
            let dst = mr.region.as_mut_ptr::<u8>().add(mr_offset);
            ptr::copy_nonoverlapping(data.as_ptr().add(offset), dst, chunk_len);
        }
        offset += chunk_len;
    }
}

pub fn execve(path: Path, mut args: Vec<String>, envp: BTreeMap<String, String>) -> TaskId {
    let buffer = fs::read_file(path.as_str()).unwrap();

    let mut loaded = load_elf(&buffer).unwrap();

    let tgid = loaded.memory_set.tgid;
    let tid = alloc_tid(tgid).unwrap();

    // 为用户栈分配并映射 VMA
    let stack_perms = PageTableEntryFlags::builder()
        .set_readable(true)
        .set_writable(true)
        .set_user(true)
        .set_accessed(true)
        .set_dirty(true)
        .build();

    loaded
        .memory_set
        .add_anon(
            VirtualAddr::from(USER_STACK_LOWER_BOUND),
            USER_STACK_SIZE,
            stack_perms,
            VmaKind::Stack,
        )
        .unwrap();

    let stack_vma = loaded.memory_set.vmas.last_mut().unwrap();
    for mr in &stack_vma.backing {
        unsafe {
            ptr::write_bytes(mr.region.as_mut_ptr::<u8>(), 0, mr.region.byte_size());
        }
    }

    if args.is_empty() {
        args.push(path.as_str().to_string());
    }

    let argv_strs: Vec<&str> = args.iter().map(String::as_str).collect();
    let env_strs: Vec<(&str, &str)> = envp.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let auxv = [
        (AT_PAGESZ, PAGE_SIZE as u64),
        (AT_ENTRY, usize::from(loaded.entry) as u64),
    ];

    let input = Input {
        argv: &argv_strs,
        env: &env_strs,
        auxv: &auxv,
    };

    let img_len = required_len(&input);
    let mut img_buf = vec![0u8; img_len];
    write(&mut img_buf, &input).unwrap();

    // 在用户栈顶写入栈映像（对齐到 16 字节）
    let image_start = USER_STACK_TOP
        .checked_sub(img_len)
        .expect("initial stack image exceeds address space")
        & !0xf;
    assert!(
        image_start >= USER_STACK_LOWER_BOUND + 16,
        "initial stack image exceeds user stack"
    );
    copy_to_vma(stack_vma, image_start, &img_buf);

    // 用户栈指针 sp（16 字节对齐），栈顶存放指向栈映像起始地址的指针
    let user_sp = (image_start - 16) & !0xf;
    let image_ptr_bytes = (image_start as u64).to_le_bytes();
    copy_to_vma(stack_vma, user_sp, &image_ptr_bytes);

    let satp = loaded.memory_set.satp();

    let mut registers = Registers::default();
    registers.sp = user_sp as u64;

    // sstatus: SPP = 0 (U-mode), SPIE = 1 (开启中断)
    let sstatus = SstatusValue::builder().set_spie(true).build().into();

    let task = Task {
        task_id: tid,
        task_group_id: tgid,
        parent: None,
        children: vec![],
        memory_set: loaded.memory_set,
        status: TaskStatus::Ready,
        file_desc: vec![],
        priority: 0,
        stack: Stack::new(),
        context: Context {
            sepc: usize::from(loaded.entry) as u64,
            sstatus,
            satp,
            registers,
        },
    };

    SCHEDULER.lock().insert_task(task);

    tid
}
