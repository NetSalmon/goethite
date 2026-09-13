//! 程序头（Program Header）结构。
//!
//! 定义程序头表项 [`ProgramHeader`]、类型 `ProgramType` 与标志 `ProgramFlags`，
//! 并实现基于 [`ElfHeader`] 遍历程序头的迭代器 [`ProgramHeaderIterator`]。
//!
//! - `ProgramType`：程序头类型（`p_type`，如 `LOAD`、`DYNAMIC`），由
//!   [`numeric!`](goethite_macros::numeric) 生成。
//! - `ProgramFlags`：程序头标志位（`p_flags`，`execute`/`write`/`read`），由
//!   [`bits!`](goethite_macros::bits) 生成。

use core::{
    fmt::{Display, Formatter},
    marker::PhantomData,
};

use goethite_macros::{bits, numeric};

use crate::types::ElfHeader;

numeric! {
    pub enum ProgramType : u32 {
        NULL         = 0x00,
        LOAD         = 0x01,
        DYNAMIC      = 0x02,
        INTERP       = 0x03,
        NOTE         = 0x04,
        SHLIB        = 0x05,
        PHDR         = 0x06,
        TLS          = 0x07,
        LOOS         = 0x60000000,
        HIOS         = 0x6FFFFFFF,
        GNU_EH_FRAME = Self::LOOS.0 + 0x474E550,
        GNU_STACK    = Self::LOOS.0 + 0x474E551,
        GNU_RELRO    = Self::LOOS.0 + 0x474E552,
        GNU_PROPERTY = Self::LOOS.0 + 0x474E553,
        SUNWBSS      = 0x6FFFFFFA,
        SUNWSTACK    = 0x6FFFFFFB,
        ARM_ARCHEXT  = 0x70000000,
        ARM_UNWIND   = 0x70000001,
    }
}

bits! {
    pub type ProgramFlags : u32 {
        execute: 0,
        write: 1,
        read: 2,
    }
}

/// ELF 程序头。
///
/// 对应程序头表项（`Elf64_Phdr`），描述一个可加载段（segment）在文件与内存中的
/// 布局。按 `#[repr(C)]` 布局，可由程序头表项大小 [`ElfHeader::program_header_entry_size`]
/// 逐项读取。
#[repr(C)]
#[derive(Debug)]
pub struct ProgramHeader {
    /// 程序头类型（`p_type`）。
    pub r#type: ProgramType,
    /// 标志位（`p_flags`）。
    pub flags: ProgramFlags,
    /// 段在文件中的字节偏移（`p_offset`）。
    pub offset: u64,
    /// 段加载到虚拟地址空间的地址（`p_vaddr`）。
    pub virtual_addr: u64,
    /// 段加载到物理地址的地址（`p_paddr`）。
    pub physical_addr: u64,
    /// 段在文件中的字节大小（`p_filesz`）。
    pub file_size: u64,
    /// 段在内存中的字节大小（`p_memsz`）。
    pub mem_size: u64,
    /// 段对齐要求（`p_align`）。
    pub align: u64,
}

impl Display for ProgramFlags {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        if self.read() {
            f.write_str("r")?
        } else {
            f.write_str("-")?
        }
        if self.write() {
            f.write_str("w")?
        } else {
            f.write_str("-")?
        }
        if self.execute() {
            f.write_str("x")
        } else {
            f.write_str("-")
        }
    }
}

impl ElfHeader {
    /// 从文件头的程序头表偏移处开始，构造遍历 `program_header_num` 个程序头的迭代器。
    pub fn ph_iter(&self) -> ProgramHeaderIterator<'_> {
        // e_phoff 是相对文件起始（即 ElfHeader）的字节偏移，需要按字节换算指针。
        let ptr = unsafe {
            (self as *const ElfHeader)
                .cast::<u8>()
                .add(self.program_header_offset as usize)
        };

        ProgramHeaderIterator {
            start: ptr.cast(),
            len: self.program_header_num as usize,
            current: 0,
            _phantom: PhantomData,
        }
    }
}

/// 程序头迭代器。
///
/// 由 [`ElfHeader::ph_iter`] 构造，按顺序产出指向各程序头的引用
/// `&ProgramHeader`。
pub struct ProgramHeaderIterator<'a> {
    start: *const ProgramHeader,
    len: usize,
    current: usize,
    _phantom: PhantomData<&'a ElfHeader>,
}

impl<'a> Iterator for ProgramHeaderIterator<'a> {
    type Item = &'a ProgramHeader;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current == self.len {
            return None;
        }

        let result = unsafe { Some(&*self.start.add(self.current)) };

        self.current += 1;

        result
    }
}
