//! Sv39 页表与页表项。
//!
//! - [`PageTableEntry`]：用 [`bits!`](bits) 按 Sv39 规范包装
//!   `u64` 页表项，包含 `V/R/W/X/U/G/A/D` 标志、RWX 权限位与
//!   PPN0/PPN1/PPN2（三者合起来是 44 位物理页号 PhysicalPageNumber）。
//! - [`PageTableEntryFlags`]：页表项的标志位视图（只含标志与权限位，不含页号）。
//! - [`PageTable`]：512 个页表项组成的、按页（4 KiB）对齐的 Sv39 页表。
use goethite_config::PAGE_TABLE_SIZE;
use goethite_macros::bits;
use goethite_riscv64::addr::{GetPpn, PhysicalAddr, PhysicalPageNumber};

bits! {
    pub type PageTableEntry: u64 {
        validate: 0,
        readable: 1,
        writable: 2,
        executable: 3,
        user: 4,
        global: 5,
        accessed: 6,
        dirty: 7,
        // RSW[0]（bit 8）：写时复制（Copy-on-Write）软件标志位。
        cow: 8,
        // RSW（bits 8..9）：RISC-V Sv39 留给操作系统软件使用的标志位。
        rsw: 8 => 9,
        flags: 0 => 7,
        rwx: 1 => 3,
        ppn0: 10 => 18,
        ppn1: 19 => 27,
        ppn2: 28 => 53,
        ppn: 10 => 53,
    }
}

impl PageTableEntry {
    /// 提取本页表项中的权限与标志位（含 [`cow`](Self::cow)）。
    #[inline]
    pub fn to_flags(&self) -> PageTableEntryFlags {
        PageTableEntryFlags::builder()
            .set_validate(self.validate())
            .set_readable(self.readable())
            .set_writable(self.writable())
            .set_executable(self.executable())
            .set_user(self.user())
            .set_global(self.global())
            .set_accessed(self.accessed())
            .set_dirty(self.dirty())
            .set_cow(self.cow())
            .build()
    }
}

impl GetPpn<u64> for PageTableEntry {
    fn get_ppn(&self, cmd: PhysicalPageNumber) -> u64 {
        match cmd {
            PhysicalPageNumber::Ppn0 => self.ppn0(),
            PhysicalPageNumber::Ppn1 => self.ppn1(),
            PhysicalPageNumber::Ppn2 => self.ppn2(),
        }
    }
}

pub enum PageTableLevel {
    Level2,
    Level1,
    Level0,
}

bits! {
    pub type PageTableEntryFlags : usize {
        validate: 0,
        readable: 1,
        writable: 2,
        executable: 3,
        user: 4,
        global: 5,
        accessed: 6,
        dirty: 7,
        // RSW[0]（bit 8）：写时复制（Copy-on-Write）软件标志位。
        cow: 8,
        rwx: 1 => 3,
    }
}

/// 按页（4 KiB）对齐的 Sv39 页表：512 个 [`PageTableEntry`]。
#[repr(align(4096))]
#[derive(Debug)]
pub struct PageTable {
    /// 页表原始的 512 项条目。
    pub(crate) entries: [PageTableEntry; PAGE_TABLE_SIZE],
}

impl PageTable {
    /// 构造一个全零的页表（所有项 `V=0`，不映射任何地址）。
    pub const fn new() -> PageTable {
        PageTable {
            entries: [PageTableEntry::new(); PAGE_TABLE_SIZE],
        }
    }

    pub fn get_valid(&self, index: usize) -> Option<&PageTableEntry> {
        let pte = self.entries.get(index)?;
        if !pte.validate() {
            return None;
        }
        Some(pte)
    }

    /// 把 `entry` 写入第 `index` 项（`index` 通常是某一级的 VirtualPageNumber 分量）。
    pub fn insert(&mut self, entry: PageTableEntry, index: usize) {
        self.entries[index] = entry;
    }

    /// 取第 `index` 项，下标越界时返回 `None`。
    #[inline]
    pub fn nth(&self, index: usize) -> Option<&PageTableEntry> {
        self.entries.get(index)
    }

    /// 取第 `index` 项对应的物理地址（由该页表项的 PhysicalPageNumber 还原），越界时返回 `None`。
    #[inline]
    pub fn nth_as_addr(&self, index: usize) -> Option<PhysicalAddr> {
        let pte = self.nth(index)?;
        let mut pa = PhysicalAddr::new();
        pa.set_ppn(pte.ppn() as usize);
        Some(pa)
    }

    /// 指向自身（作为虚拟地址的裸指针）。
    #[inline]
    pub fn as_ptr(&self) -> *const PageTable {
        self as *const PageTable
    }

    /// 返回页表的物理地址：若自身位于内核高半区虚拟地址则做线性偏移转换。
    #[inline]
    pub fn as_phys_addr(&self) -> PhysicalAddr {
        let ptr = self.as_ptr() as usize;
        if ptr >= goethite_config::KERNEL_OFFSET {
            goethite_riscv64::addr::VirtualAddr::from(ptr).as_paddr()
        } else {
            PhysicalAddr::from(ptr)
        }
    }

    /// 指向自身的可变裸指针。
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut PageTable {
        self as *mut PageTable
    }
}
