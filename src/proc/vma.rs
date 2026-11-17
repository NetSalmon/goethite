//! 进程虚拟内存区域（VMA）与内存集合（[`MemorySet`]）。
//!
//! [`MemorySet`] 把一个进程的内存抽象为两层：
//!
//! - **虚拟布局**：一个按虚拟地址抽象的 [`Vma`] 列表，描述进程期望看到的地址范围、
//!   权限与类型（代码/数据/BSS/栈/堆/mmap）。这是进程内存的「第一公民」。
//! - **活动页表**：由 [`crate::mm::address_space::ASpaceManager`] 集中管理的
//!   [`crate::mm::address_space::AddressSpace`]。`MemSet` **不直接持有**页表，只保存进程
//!   id（[`TaskGroupId`]），所有映射 / 切换操作都经由管理器委托。
//!
//! 与「直接把物理 [`Region`] · 页表塞进 `MemSet`」的设计不同，这里物理后备帧是每个
//! [`Vma`] 的属性（[`Vma::backing`]），并记录每块物理帧被映射到的虚拟地址
//! （[`MappedRegion::vaddr`]）。
//!
//! # 内容填充
//!
//! 后备帧的**内容**（匿名/BSS 的零填充、Text/Data 段从文件的读取）不是本模块的
//! 职责：它们由加载器 / 缺页处理路径按 [`VmaKind`] 与 [`Vma::file_offset`] 决定。
//! 本模块只保证帧的分配、映射登记与生命周期。

use alloc::vec::Vec;

use goethite_config::{BUDDY_MAX_ORDER, PAGE_SIZE, align};
use goethite_riscv64::addr::VirtualAddr;

use crate::{
    mm::{
        address_space::{AS_MANAGER, ASpaceError},
        buddy_system::order_size,
        object::AllocError,
        page_table::PageTableEntryFlags,
        region::Region,
    },
    proc::{TaskGroupId, alloc_tgid, free_tgid},
};

/// 一个虚拟内存区域的类型，决定其后备帧来源与增长行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmaKind {
    /// 代码段：可执行，来自 ELF 文件。
    Text,
    /// 数据段：可读写，来自 ELF 文件或匿名内存。
    Data,
    /// BSS：零填充匿名内存。
    Bss,
    /// 栈：向低地址增长。
    Stack,
    /// 堆：由 `brk` 控制向高地址增长。
    Heap,
    /// 文件 / 共享映射（预留）。
    Mmap,
}

/// 一个已映射的物理块：物理帧 + 它被映射到的虚拟地址起点。
///
/// `vaddr` 记录映射起点，便于在需要时针对这一块做 `unmap`。
pub struct MappedRegion {
    /// 物理帧的 RAII 句柄，析构时归还伙伴系统。
    pub region: Region,
    /// 该物理块被映射到的虚拟地址起点（页对齐）。
    pub vaddr: VirtualAddr,
}

/// 虚拟内存区域描述符——进程内存布局的第一公民。
///
/// `len` 是**保留的虚拟跨度**（页对齐）；`backing` 是其中真正分配并映射了的物理块，
/// 因而缺页处理可以在保留跨度内按需把某一个物理块映射进来（见 [`MemorySet::handle_page_fault`]）。
pub struct Vma {
    /// 起始虚拟地址。
    pub vaddr: VirtualAddr,
    /// 保留的虚拟跨度（字节，页对齐）。
    pub len: usize,
    /// 区域权限（仅使用 `r/w/x/u` 位）。
    pub perms: PageTableEntryFlags,
    /// 区域类型。
    pub kind: VmaKind,
    /// 文件来源段在文件内的偏移；匿名区域为 0。
    pub file_offset: usize,
    /// 已分配并映射到本区域的后备物理块。
    pub backing: Vec<MappedRegion>,
}

impl Vma {
    /// 区域起始虚拟地址（`usize` 形式）。
    #[inline]
    pub fn start(&self) -> usize {
        usize::from(self.vaddr)
    }

    /// 区域结束虚拟地址（`usize` 形式，不包含）。
    #[inline]
    pub fn end(&self) -> usize {
        self.start() + self.len
    }

    /// 判断 `addr`（`usize` 形式）是否落在本区域保留跨度内。
    #[inline]
    pub fn covers(&self, addr: usize) -> bool {
        self.start() <= addr && addr < self.end()
    }

    /// 判断某个页（`usize` 形式）是否已被某个 `backing` 块映射。
    #[inline]
    pub fn is_mapped(&self, page: usize) -> bool {
        self.backing.iter().any(|mr| {
            let s = usize::from(mr.vaddr);
            let e = s + mr.region.byte_size();
            s <= page && page < e
        })
    }
}

/// 一段待加载的段描述符：从 ELF 的 program header 解析而来，仅描述**虚拟布局**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadSegment {
    /// 段在内存中的虚拟地址（页对齐）。
    pub vaddr: VirtualAddr,
    /// 段在该虚拟地址的字节长度（页对齐）。
    pub memsz: usize,
    /// 段在文件中的偏移（无文件内容的段为 0）。
    pub file_offset: usize,
    /// 段权限。
    pub perms: PageTableEntryFlags,
    /// 段类型（决定内容填充方式）。
    pub kind: VmaKind,
}

/// [`MemorySet`] 操作可能出现的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MemSetError {
    /// 底层分配失败。
    #[error(transparent)]
    Alloc(AllocError),
    /// 地址空间管理器操作失败（如引用了不存在的 [`TaskGroupId`]）。
    #[error(transparent)]
    As(ASpaceError),
    /// 进程 id（tgid）已用尽。
    #[error("task group id exhausted")]
    IdExhausted,
    /// 虚拟地址未被任何 VMA 覆盖（非法访问）。
    #[error("virtual address is not mapped by any vma")]
    Unmapped,
}

impl From<AllocError> for MemSetError {
    fn from(e: AllocError) -> Self {
        MemSetError::Alloc(e)
    }
}

impl From<ASpaceError> for MemSetError {
    fn from(e: ASpaceError) -> Self {
        MemSetError::As(e)
    }
}

/// 进程虚拟内存集合：虚拟布局（[`Vma`] 列表）+ 进程 id 下的集中管理地址空间。
pub struct MemorySet {
    /// 按虚拟地址抽象的有序 VMA 列表，权威描述进程虚拟布局。
    pub vmas: Vec<Vma>,
    /// 本进程的线程组 id（tgid）：也是集中管理器中该进程地址空间的索引。
    pub tgid: TaskGroupId,
}

impl MemorySet {
    /// 创建一个新进程的内存集合：分配一个进程 id（tgid），并在地址空间管理器中
    /// 为其关联一个新的空地址空间。
    pub fn new() -> Result<Self, MemSetError> {
        let tgid = alloc_tgid().ok_or(MemSetError::IdExhausted)?;
        if let Err(e) = AS_MANAGER.write().alloc(tgid) {
            free_tgid(tgid);
            return Err(MemSetError::As(e));
        }
        Ok(Self {
            vmas: Vec::new(),
            tgid,
        })
    }

    /// 由一段段加载段描述符构建内存集合。
    ///
    /// 每段都会被分配物理帧、映射进页表并登记为对应的 [`Vma`]。
    pub fn with_segments(segments: &[LoadSegment]) -> Result<Self, MemSetError> {
        let mut ms = MemorySet::new()?;
        for seg in segments {
            ms.add_region(seg.vaddr, seg.memsz, seg.perms, seg.kind, seg.file_offset)?;
        }
        Ok(ms)
    }

    /// 在虚拟地址 `vaddr` 起新增一段跨度为 `len` 的区域：分配物理帧、映射进页表，
    /// 并登记一个 [`Vma`]。
    ///
    /// 物理帧按 2 的幂次块贪心分配（不超过 `BUDDY_MAX_ORDER`），并记录每块映射到的
    /// 虚拟地址。任何一次分配失败都会回滚已映射部分。
    ///
    /// 内容填充（零 / 文件字节）由调用方决定，本方法只负责映射与登记。
    pub fn add_region(
        &mut self,
        vaddr: VirtualAddr,
        len: usize,
        perms: PageTableEntryFlags,
        kind: VmaKind,
        file_offset: usize,
    ) -> Result<(), MemSetError> {
        let total = align(len.max(1), PAGE_SIZE);
        let backing = self.alloc_and_map(vaddr, total, perms)?;
        self.vmas.push(Vma {
            vaddr,
            len: total,
            perms,
            kind,
            file_offset,
            backing,
        });
        Ok(())
    }

    /// 分配一段零填充的匿名内存（BSS、栈、堆、匿名 `mmap` 的占位实现）。
    pub fn add_anon(
        &mut self,
        vaddr: VirtualAddr,
        len: usize,
        perms: PageTableEntryFlags,
        kind: VmaKind,
    ) -> Result<(), MemSetError> {
        self.add_region(vaddr, len, perms, kind, 0)
    }

    /// 按 2 的幂次块贪心分配并映射 `/span/` 字节的物理帧，返回登记好的后备块列表。
    ///
    /// 任何一次分配失败都会先把已映射的块 `unmap` 掉再返回错误，避免页表残留指向
    /// 已归还帧的指针。映射操作经由 [`AS_MANAGER`] 委托给对应地址空间。
    fn alloc_and_map(
        &mut self,
        mut vaddr: VirtualAddr,
        span: usize,
        perms: PageTableEntryFlags,
    ) -> Result<Vec<MappedRegion>, MemSetError> {
        let total = align(span.max(1), PAGE_SIZE);
        let mut remaining = total;
        let mut backing: Vec<MappedRegion> = Vec::new();

        let mut mgr = AS_MANAGER.write();

        while remaining > 0 {
            let pages = remaining / PAGE_SIZE;
            // 最大的 2 的幂块：页数不超过剩余，且受伙伴系统最大阶限制。
            let order = (pages.ilog2() as usize).min(BUDDY_MAX_ORDER - 1);
            let block = order_size(order);

            let region = match Region::new(order) {
                Some(r) => r,
                None => {
                    for mr in &backing {
                        mgr.unmap(self.tgid, mr.vaddr).ok();
                    }
                    return Err(MemSetError::Alloc(AllocError::OutOfMemory));
                }
            };
            let pa = region.as_phys_addr();

            if let Err(e) = mgr.map(self.tgid, vaddr, pa, perms) {
                for mr in &backing {
                    mgr.unmap(self.tgid, mr.vaddr).ok();
                }
                return Err(MemSetError::As(e));
            }

            backing.push(MappedRegion { region, vaddr });
            vaddr = vaddr.add::<u8>(block);
            remaining -= block;
        }

        Ok(backing)
    }

    /// 定位覆盖某个虚拟地址的 [`Vma`]。
    pub fn find_vma(&self, addr: VirtualAddr) -> Option<&Vma> {
        let a = usize::from(addr);
        self.vmas.iter().find(|v| v.covers(a))
    }

    /// 处理一次缺页：优先处理栈向低地址增长，随后在覆盖该地址的 VMA 内按需映射缺失页。
    ///
    /// 对于已映射的页直接返回（权限类缺页由上层另行判定）；对未被任何 VMA 覆盖的
    /// 地址返回 [`MemSetError::Unmapped`]。
    pub fn handle_page_fault(&mut self, fault_addr: VirtualAddr) -> Result<(), MemSetError> {
        let fault = usize::from(fault_addr);
        let page = align(fault, PAGE_SIZE);

        // 栈向低地址增长：地址落在栈起点之下时，先扩展栈 VMA。
        if let Some(sidx) = self.vmas.iter().position(|v| v.kind == VmaKind::Stack) {
            let sstart = usize::from(self.vmas[sidx].vaddr);
            if page < sstart {
                self.grow_stack(VirtualAddr::from(page))?;
            }
        }

        let idx = match self.vmas.iter().position(|v| v.covers(page)) {
            Some(i) => i,
            None => return Err(MemSetError::Unmapped),
        };

        let va = VirtualAddr::from(page);
        if self.vmas[idx].is_mapped(page) {
            return Ok(());
        }
        self.map_page_to(idx, va)?;
        Ok(())
    }

    /// 把单个缺失页分配并映射到指定 VMA，并登记进其 `backing`。
    fn map_page_to(&mut self, idx: usize, page: VirtualAddr) -> Result<(), MemSetError> {
        let perms = self.vmas[idx].perms;
        let mut new_backing = self.alloc_and_map(page, PAGE_SIZE, perms)?;
        self.vmas[idx].backing.append(&mut new_backing);
        Ok(())
    }

    /// 扩展栈：把栈 VMA 的起点下移到 `new_start`，并映射新增的披露范围。
    pub fn grow_stack(&mut self, new_start: VirtualAddr) -> Result<(), MemSetError> {
        let idx = self
            .vmas
            .iter()
            .position(|v| v.kind == VmaKind::Stack)
            .ok_or(MemSetError::Unmapped)?;

        let cur_start = usize::from(self.vmas[idx].vaddr);
        let new_start_u = usize::from(new_start);
        if new_start_u >= cur_start {
            return Ok(());
        }

        let growth = cur_start - new_start_u;
        let perms = self.vmas[idx].perms;
        let mut new_backing = self.alloc_and_map(new_start, growth, perms)?;

        let vma = &mut self.vmas[idx];
        vma.vaddr = new_start;
        vma.len += growth;
        vma.backing.append(&mut new_backing);
        Ok(())
    }

    /// 扩展堆：把堆 VMA 的末端上移到 `new_brk`，并映射新增范围。
    ///
    /// 只支持增长；收缩（释放多出的帧）留给调用方在确定不再需要时自行 `unmap` 处理。
    pub fn set_brk(&mut self, new_brk: VirtualAddr) -> Result<(), MemSetError> {
        let idx = self
            .vmas
            .iter()
            .position(|v| v.kind == VmaKind::Heap)
            .ok_or(MemSetError::Unmapped)?;

        let cur_end = usize::from(self.vmas[idx].vaddr) + self.vmas[idx].len;
        let new_end = usize::from(new_brk);
        if new_end <= cur_end {
            return Ok(());
        }

        let growth = align(new_end - cur_end, PAGE_SIZE);
        let map_va = VirtualAddr::from(cur_end);
        let perms = self.vmas[idx].perms;
        let mut new_backing = self.alloc_and_map(map_va, growth, perms)?;

        let vma = &mut self.vmas[idx];
        vma.len += growth;
        vma.backing.append(&mut new_backing);
        Ok(())
    }

    /// 经由地址空间管理器激活本进程的地址空间（写入 SATP 并刷新 TLB）。
    ///
    /// # Safety
    ///
    /// 调用方需保证处理好切换地址空间带来的地址冲突风险。
    pub unsafe fn activate(&self) -> Result<(), MemSetError> {
        unsafe { AS_MANAGER.read().activate(self.tgid) }?;
        Ok(())
    }
}

impl Drop for MemorySet {
    /// teardown：交给管理器释放本进程地址空间的全部页表；`vmas` 随后析构，
    /// 归还所有后备物理帧。
    ///
    /// 注意：这假定地址空间与进程一一对应。若将来引入 fork / COW 共享地址空间，
    /// 应改为在 [`ASpaceManager`](crate::mm::address_space::ASpaceManager) 中做引用
    /// 计数，而不要在此直接 `free`。
    fn drop(&mut self) {
        AS_MANAGER.write().free(self.tgid);
        free_tgid(self.tgid);
    }
}
