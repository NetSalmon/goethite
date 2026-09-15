//! RISC-V 寄存器抽象。
//!
//! - [`csr`]：CSR 读写（`sie` / `sepc` / `satp` / `stimecmp` 等）；
//! - [`gpr`]：通用寄存器读写（含 ABI 别名，如 `tp`）；
//! - [`values`]：寄存器位域值类型（`SatpValue` / `SstatusBits` 等）。
//!
//! 寄存器类型使用零大小标记结构体表示，例如 [`csr::Sie`]。这避免了把
//! CSR 编号或 GPR 名称作为运行时数据传递，同时让读写权限在 API 层可见。
pub mod csr;
pub mod gpr;
pub mod values;

#[repr(C)]
#[allow(unused)]
#[derive(Debug, Clone)]
pub struct Registers {
    _zero: u64,
    pub ra: u64,
    pub sp: u64,
    pub gp: u64,
    pub tp: u64,
    pub t0: u64,
    pub t1: u64,
    pub t2: u64,
    pub s0: u64,
    pub s1: u64,
    pub a0: u64,
    pub a1: u64,
    pub a2: u64,
    pub a3: u64,
    pub a4: u64,
    pub a5: u64,
    pub a6: u64,
    pub a7: u64,
    pub s2: u64,
    pub s3: u64,
    pub s4: u64,
    pub s5: u64,
    pub s6: u64,
    pub s7: u64,
    pub s8: u64,
    pub s9: u64,
    pub s10: u64,
    pub s11: u64,
    pub t3: u64,
    pub t4: u64,
    pub t5: u64,
    pub t6: u64,
}

const _: () = assert!(size_of::<Registers>() as u64 == 256);
