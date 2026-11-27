//! 内存地址空间。
//!
//! [`AddressSpace`] 由一个根页表（用于更高层级的映射）与一组以 VirtualPageNumber 分量为键的
//! 子页表组成，后续用于进程地址空间的构建与切换。
//!
//! 三张页表按 Sv39 的 V2/V1/V0 三级寻址组织：`root` 存 V2 分量，level-1 页表存
//! V1 分量，level-0 页表存 V0 分量并存放实际数据帧的叶项。中间页表用 [`Handle`]
//! 从伙伴系统分配并以自身物理页号（PhysicalPageNumber）为键存入 [`AddressSpace::children`]。
use alloc::collections::BTreeMap;
use core::{arch::asm, fmt::Debug};

use goethite_config::{KERNEL_PTE_START, PAGE_TABLE_SIZE};
use goethite_macros::lazy;
use goethite_riscv64::{
    addr::{PhysicalAddr, VirtualAddr},
    registers::{
        csr::Satp,
        values::{SatpMode, SatpValue},
    },
};

use crate::{
    mm::{
        object::{AllocError, Handle, Init},
        page_table::{PageTable, PageTableEntry, PageTableEntryFlags, PageTableLevel},
    },
    proc::TaskGroupId,
};

/// 一个内存地址空间：根页表 + 子页表集合。
#[derive(Debug)]
pub struct AddressSpace {
    /// 根页表，映射地址空间的顶层。
    pub root: Handle<PageTable, Init>,
    /// 中间页表（level-1 / level-0），以自身的物理页号（PhysicalPageNumber）为键。
    pub children: BTreeMap<usize, Handle<PageTable, Init>>,
}

impl AddressSpace {
    /// 新建一个仅含根页表的空地址空间。
    ///
    /// 根页表分配失败时 panic；若希望处理失败请改用 [`try_new`](Self::try_new)。
    pub fn new() -> Self {
        Self::try_new().expect("address space root allocation failed")
    }

    /// 同上，但根页表分配失败时返回 [`AllocError`] 而不是 panic。
    pub fn try_new() -> Result<Self, AllocError> {
        let root = Handle::alloc()?.init(PageTable::new());
        Ok(Self {
            root,
            children: BTreeMap::new(),
        })
    }

    /// 把物理页 `pa` 以 `flags` 映射到虚拟地址 `va`。
    ///
    /// 沿 V2/V1/V0 逐级下行，中间页表缺失时从伙伴系统逐个创建并登记到
    /// [`Self::children`]，随后在叶页表项写入 PhysicalPageNumber 与权限标志。
    ///
    /// # Errors
    ///
    /// - [`AllocError`]：创建中间页表时伙伴系统无足够空闲页块。
    pub fn map(
        &mut self,
        va: VirtualAddr,
        pa: PhysicalAddr,
        flags: PageTableEntryFlags,
    ) -> Result<(), AllocError> {
        let vpn2 = va.vpn2();
        let vpn1 = va.vpn1();
        let vpn0 = va.vpn0();

        // 确保指向 level-1 页表的根页表项存在，并取该 level-1 页表的物理页号
        // 作为它在 children 中的键。
        let pt1_key = {
            let pte2 = &mut self.root.entries[vpn2];
            if !pte2.validate() {
                let new_pt = Handle::alloc()?.init(PageTable::new());
                let addr = new_pt.get().as_phys_addr();
                pte2.set_ppn(addr.ppn() as u64);
                pte2.set_validate(true);
                self.children.insert(addr.ppn(), new_pt);
            }
            pte2.ppn() as usize
        };

        // 确保指向 level-0 页表的 level-1 页表项存在，并取该 level-0 页表的物理页号
        // 作为它在 children 中的键。
        let pt0_key = {
            // 中间页表必然已登记：根页表项有效即由本次或先前调用插入过。
            let need_create = {
                let pt1 = &self.children[&pt1_key];
                !pt1.entries[vpn1].validate()
            };
            if need_create {
                let new_pt = Handle::alloc()?.init(PageTable::new());
                let addr = new_pt.get().as_phys_addr();
                {
                    let pt1 = self
                        .children
                        .get_mut(&pt1_key)
                        .expect("level-1 page table missing");

                    let pte1 = &mut pt1.entries[vpn1];
                    pte1.set_ppn(addr.ppn() as u64);
                    pte1.set_validate(true);
                }
                self.children.insert(addr.ppn(), new_pt);
            }
            self.children[&pt1_key].entries[vpn1].ppn() as usize
        };

        // 写入叶页表项。
        let pt0 = self
            .children
            .get_mut(&pt0_key)
            .expect("level-0 page table missing");

        let pte0 = &mut pt0.entries[vpn0];
        pte0.set_ppn(pa.ppn() as u64);
        pte0.set_validate(true);
        pte0.set_readable(flags.readable());
        pte0.set_writable(flags.writable());
        pte0.set_executable(flags.executable());
        pte0.set_user(flags.user());
        pte0.set_global(flags.global());
        pte0.set_accessed(flags.accessed());
        pte0.set_dirty(flags.dirty());

        Ok(())
    }

    /// 取消 `va` 的映射：把对应叶页表项清零。
    ///
    /// 若沿途任一页表项无效或中间页表缺失，则说明 `va` 未被映射，直接返回。
    pub fn unmap(&mut self, va: VirtualAddr) {
        let vpn2 = va.vpn2();
        let vpn1 = va.vpn1();
        let vpn0 = va.vpn0();

        let pt1_key = {
            let pte2 = self.root.entries[vpn2];
            if !pte2.validate() {
                return;
            }
            pte2.ppn() as usize
        };

        let pt0_key = {
            let pt1 = match self.children.get(&pt1_key) {
                Some(pt) => pt,
                None => return,
            };
            let pte1 = pt1.entries[vpn1];
            if !pte1.validate() {
                return;
            }
            pte1.ppn() as usize
        };

        let pt0 = match self.children.get_mut(&pt0_key) {
            Some(pt) => pt,
            None => return,
        };

        let pte0 = &mut pt0.entries[vpn0];
        *pte0 = PageTableEntry::new();
    }

    /// 通过虚拟地址 `va` 获取对应的叶页表项（只读）。
    ///
    /// 沿 V2/V1/V0 逐级下行；仅当沿途中间页表项有效且对应页表存在时返回
    /// `Some`。叶页表项自身可能无效（`V=0`），由调用方按需用
    /// [`PageTableEntry::validate`] 判断是否已建立映射。
    pub fn pte(&self, va: VirtualAddr) -> Option<&PageTableEntry> {
        let pt1_key = {
            let pte2 = self.root.get_valid(va.vpn2())?;
            pte2.ppn() as usize
        };

        let pt1 = self.children.get(&pt1_key)?;
        let pte1 = pt1.get_valid(va.vpn1())?;

        let pt0 = self.children.get(&(pte1.ppn() as usize))?;
        pt0.entries.get(va.vpn0())
    }

    /// 通过虚拟地址 `va` 获取对应的叶页表项（可变），用于就地修改。
    ///
    /// 定位语义与 [`pte`](Self::pte) 相同；返回可变引用以便直接修改页表项的
    /// 标志或物理页号。若 `va` 未被映射，返回 `None`。
    pub fn pte_mut(&mut self, va: VirtualAddr) -> Option<&mut PageTableEntry> {
        // 通过VPN2获取Level1页表的索引
        let level1_index = {
            let pte2 = self.root.get_valid(va.vpn2())?;
            pte2.ppn() as usize
        };

        // 通过VPN1获取Level0页表索引
        let pt0_key = {
            let pt1 = self.children.get(&level1_index)?;
            let pte1 = pt1.get_valid(va.vpn1())?;
            pte1.ppn() as usize
        };

        let pt0 = self.children.get_mut(&pt0_key)?;
        pt0.entries.get_mut(va.vpn0())
    }

    pub fn satp(&self) -> u64 {
        SatpValue::builder()
            .set_ppn(self.root.get().as_phys_addr().ppn() as u64)
            .set_mode(SatpMode::SV39.into())
            .build()
            .into()
    }

    /// 激活地址空间
    ///
    /// # Safety
    ///
    /// 调用方需保证处理好切换地址空间带来的地址冲突风险
    pub unsafe fn enable(&self) {
        let satp = SatpValue::builder()
            .set_ppn(self.root.get().as_phys_addr().ppn() as u64)
            .set_mode(SatpMode::SV39.into())
            .build();

        Satp::write(satp.into());

        unsafe { asm!("sfence.vma") }
    }

    pub fn try_from_kernel(kernel: &Self) -> Result<Self, AllocError> {
        let mut aspace = Self::try_new()?;

        for i in KERNEL_PTE_START..PAGE_TABLE_SIZE {
            aspace.root.entries[i] = kernel.root.entries[i];
        }

        Ok(aspace)
    }

    pub fn from_kernel(kernel: &Self) -> Self {
        let mut aspace = Self::new();

        for i in KERNEL_PTE_START..PAGE_TABLE_SIZE {
            aspace.root.entries[i] = kernel.root.entries[i];
        }

        aspace
    }

    pub fn copy_from_kernel(&mut self, other: &Self) {
        for i in KERNEL_PTE_START..PAGE_TABLE_SIZE {
            self.root.entries[i] = other.root.entries[i];
        }
    }
}

/// [`AddressSpaceManager`] 操作可能出现的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AddressSpaceError {
    /// 传入的 [`TaskGroupId`] 没有对应的地址空间。
    #[error("task group id not found")]
    NotFound,
    /// 传入的 [`TaskGroupId`] 已关联了地址空间。
    #[error("task group id already exists")]
    AlreadyExists,
    /// 底层分配失败（或缺页处理中的页表分配失败）。
    #[error(transparent)]
    Alloc(AllocError),
}

impl From<AllocError> for AddressSpaceError {
    fn from(e: AllocError) -> Self {
        AddressSpaceError::Alloc(e)
    }
}

/// 地址空间管理器：集中持有内核与所有用户地址空间，避免进程层直接持有页表。
///
/// 地址空间按**进程（线程组）** [`TaskGroupId`] 索引——同一进程的所有线程共享
/// 同一地址空间。`kernel` 是内核地址空间；`user` 映射每个进程的地址空间。内存集合
/// （`MemSet`）只保存一个 [`TaskGroupId`]（即进程 id），所有映射 / 切换操作都经由
/// 本管理器委托。
#[derive(Debug)]
pub struct AddressSpaceManager {
    /// 内核地址空间。
    kernel: AddressSpace,
    /// 用户地址空间，按进程 [`TaskGroupId`] 索引。
    user: BTreeMap<TaskGroupId, AddressSpace>,
}

impl AddressSpaceManager {
    /// 构造一个空的管理器：仅含内核地址空间，不含任何用户地址空间。
    pub fn new() -> Self {
        Self {
            kernel: AddressSpace::new(),
            user: BTreeMap::new(),
        }
    }

    /// 内核地址空间（只读）。内核用于映射自身镜像 / 设备等固定映射。
    pub fn kernel(&self) -> &AddressSpace {
        &self.kernel
    }

    pub fn kernel_mut(&mut self) -> &mut AddressSpace {
        &mut self.kernel
    }

    pub fn user(&self, tgid: TaskGroupId) -> Option<&AddressSpace> {
        self.user.get(&tgid)
    }

    pub fn user_mut(&mut self, tgid: TaskGroupId) -> Option<&mut AddressSpace> {
        self.user.get_mut(&tgid)
    }

    pub fn satp(&self, tgid: TaskGroupId) -> Option<u64> {
        self.user.get(&tgid).map(|aspace| aspace.satp())
    }

    pub fn kernel_map(
        &mut self,
        va: VirtualAddr,
        pa: PhysicalAddr,
        flags: PageTableEntryFlags,
    ) -> Result<(), AddressSpaceError> {
        self.kernel.map(va, pa, flags)?;

        Ok(())
    }

    pub fn kernel_unmap(&mut self, va: VirtualAddr) -> Result<(), AddressSpaceError> {
        self.kernel.unmap(va);

        Ok(())
    }

    pub fn kernel_activate(&self) -> Result<(), AddressSpaceError> {
        unsafe { self.kernel.enable() }

        Ok(())
    }

    /// 为进程 `tgid` 关联一个新的空地址空间。
    ///
    /// # Errors
    ///
    /// - [`AddressSpaceError::AlreadyExists`]：`tgid` 已关联地址空间。
    /// - [`AddressSpaceError::Alloc`]：创建根页表失败。
    pub fn alloc(&mut self, tgid: TaskGroupId) -> Result<(), AddressSpaceError> {
        if self.user.contains_key(&tgid) {
            return Err(AddressSpaceError::AlreadyExists);
        }
        let aspace = AddressSpace::from_kernel(&self.kernel);
        self.user.insert(tgid, aspace);
        Ok(())
    }

    /// 把物理页 `pa` 以 `flags` 映射到进程 `tgid` 地址空间的虚拟地址 `va`。
    pub fn map(
        &mut self,
        tgid: TaskGroupId,
        va: VirtualAddr,
        pa: PhysicalAddr,
        flags: PageTableEntryFlags,
    ) -> Result<(), AddressSpaceError> {
        let aspace = self.user.get_mut(&tgid).ok_or(AddressSpaceError::NotFound)?;
        aspace.map(va, pa, flags).map_err(AddressSpaceError::Alloc)
    }

    /// 取消进程 `tgid` 地址空间中 `va` 的映射。
    pub fn unmap(&mut self, tgid: TaskGroupId, va: VirtualAddr) -> Result<(), AddressSpaceError> {
        let aspace = self.user.get_mut(&tgid).ok_or(AddressSpaceError::NotFound)?;
        aspace.unmap(va);
        Ok(())
    }

    /// 激活进程 `tgid` 的地址空间（写入 SATP 并刷新 TLB）。
    ///
    /// # Safety
    ///
    /// 调用方需保证处理好切换地址空间带来的地址冲突风险。
    pub unsafe fn activate(&self, tgid: TaskGroupId) -> Result<(), AddressSpaceError> {
        let aspace = self.user.get(&tgid).ok_or(AddressSpaceError::NotFound)?;
        unsafe { aspace.enable() };
        Ok(())
    }

    /// 释放进程 `tgid` 的地址空间（归还其全部页表页）。
    pub fn free(&mut self, tgid: TaskGroupId) {
        self.user.remove(&tgid);
    }
}

impl Default for AddressSpaceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 全内核唯一的地址空间管理器，懒加载并以自旋锁保护。
#[lazy(rwlock)]
pub static ADDRESS_SPACE_MANAGER: AddressSpaceManager = AddressSpaceManager::new();
