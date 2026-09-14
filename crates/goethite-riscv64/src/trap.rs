//! RISC-V 64 陷阱（异常 / 中断）类型。
//!
//! [`Trap`] 由 `scause` 值解析而来，最高位（bit 63）区分中断与异常：
//! 中断为 [`Interrupt`]，异常为 [`Exception`]。

use goethite_macros::numeric;

const INTERRUPT_MASK: i64 = 1 << 63;

numeric! {
    pub enum Interrupt: i64 {
        U_MODE_SOFTWARE = INTERRUPT_MASK,
        S_MODE_SOFTWARE = INTERRUPT_MASK | 1,
        M_MODE_SOFTWARE = INTERRUPT_MASK | 3,
        USER_TIMER = INTERRUPT_MASK | 4,
        SUPERVISOR_TIMER = INTERRUPT_MASK | 5,
        MACHINE_TIMER = INTERRUPT_MASK | 7,
        USER_EXTERNAL = INTERRUPT_MASK | 8,
        SUPERVISOR_EXTERNAL = INTERRUPT_MASK | 9,
        MACHINE_EXTERNAL = INTERRUPT_MASK | 11,
    }
}

numeric! {
    pub enum Exception: i64 {
        INSTRUCTION_ADDRESS_MISALIGNED = 0,
        INSTRUCTION_ACCESS_FAULT = 1,
        ILLEGAL_INSTRUCTION = 2,
        BREAKPOINT = 3,
        LOAD_ADDRESS_MISALIGNED = 4,
        LOAD_ACCESS_FAULT = 5,
        STORE_ADDRESS_MISALIGNED = 6,
        STORE_ACCESS_FAULT = 7,
        U_MODE_ECALL = 8,
        S_MODE_ECALL = 9,
        M_MODE_ECALL = 11,
        INSTRUCTION_PAGE_FAULT = 12,
        LOAD_PAGE_FAULT = 13,
        STORE_PAGE_FAULT = 15,
    }
}

/// 陷阱：异常或中断。
#[derive(Clone, Copy, Debug)]
pub enum Trap {
    Exception(Exception),
    Interrupt(Interrupt),
}

impl From<i64> for Trap {
    /// RISC-V `scause` 最高位（bit 63）表示中断（1）vs 异常（0）。
    /// 作为有符号数解释时，中断为负值，异常为非负值（含 0）。
    fn from(value: i64) -> Trap {
        if value >= 0 {
            Trap::Exception(Exception::from(value))
        } else {
            Trap::Interrupt(Interrupt::from(value))
        }
    }
}

impl From<Exception> for Trap {
    fn from(value: Exception) -> Trap {
        Trap::Exception(value)
    }
}

impl From<Interrupt> for Trap {
    fn from(value: Interrupt) -> Trap {
        Trap::Interrupt(value)
    }
}

impl From<Trap> for i64 {
    fn from(value: Trap) -> i64 {
        match value {
            Trap::Exception(i) => i.into(),
            Trap::Interrupt(i) => i.into(),
        }
    }
}
