//! RISC-V 寄存器抽象。
//!
//! - [`csr`]：CSR 读写（`sie` / `sepc` / `satp` / `stimecmp` 等）；
//! - [`gpr`]：通用寄存器读写（含 ABI 别名，如 `tp`）；
//! - [`values`]：寄存器位域值类型（`SatpValue` / `SstatusValue` 等）。
//!
//! 寄存器类型使用零大小标记结构体表示，例如 [`csr::Sie`]。这避免了把
//! CSR 编号或 GPR 名称作为运行时数据传递，同时让读写权限在 API 层可见。
#[cfg(target_arch = "riscv64")]
pub mod csr;
#[cfg(target_arch = "riscv64")]
pub mod gpr;
pub mod values;

/// 宿主机上的通用寄存器占位实现。
///
/// 纯逻辑 crate（如 `goethite-sync`）在 x86_64 上做单元测试时会通过
/// `registers::gpr::Tp` 读取 hart 编号；这里固定返回 `0`，使这些 crate
/// 无需 QEMU 即可测试。
#[cfg(not(target_arch = "riscv64"))]
pub mod gpr {
    /// 线程 / hart 指针寄存器占位。
    pub struct Tp;

    impl Tp {
        /// 宿主机构建固定返回 `0`。
        #[inline]
        pub fn read() -> u64 {
            0
        }

        /// 宿主机构建忽略写入。
        #[inline]
        pub fn write(_value: u64) {}
    }
}

#[repr(C)]
#[allow(unused)]
#[derive(Debug, Clone, Default)]
pub struct Registers {
    pub _zero: u64,
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

impl Registers {
    pub const fn new() -> Self {
        Self {
            _zero: 0,
            ra: 0,
            sp: 0,
            gp: 0,
            tp: 0,
            t0: 0,
            t1: 0,
            t2: 0,
            s0: 0,
            s1: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            a3: 0,
            a4: 0,
            a5: 0,
            a6: 0,
            a7: 0,
            s2: 0,
            s3: 0,
            s4: 0,
            s5: 0,
            s6: 0,
            s7: 0,
            s8: 0,
            s9: 0,
            s10: 0,
            s11: 0,
            t3: 0,
            t4: 0,
            t5: 0,
            t6: 0,
        }
    }

    pub fn args(&self) -> [u64; 8] {
        [
            self.a0, self.a1, self.a2, self.a3, self.a4, self.a5, self.a6, self.a7,
        ]
    }
}

const _: () = assert!(size_of::<Registers>() as u64 == 256);
