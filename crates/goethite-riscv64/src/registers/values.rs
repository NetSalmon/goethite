//! 常用 CSR 的结构化值。
//!
//! 这些类型只负责描述和构造寄存器值，不执行 CSR 读写；读写仍由
//! [`super::csr`] 中的标记类型负责。均由 `bits!` / `numeric!` 生成：
//!
//! - [`SatpValue`]：`satp` 寄存器值，含 `ppn` / `asid` / `mode` 位域；
//! - [`SatpMode`]：`satp.mode` 的取值（`BARE` / `SV39` / `SV48` 等）；
//! - [`SstatusBits`]：`sstatus` 的部分位（`spp` / `spie` / `sie`）。

use goethite_macros::{bits, numeric};

bits! {
    pub type SatpValue: u64 {
        ppn: 0 => 43,
        asid: 44 => 59,
        mode: 60 => 63
    }
}

numeric! {
    pub enum SatpMode: u64 {
        BARE = 0,
        SV39 = 8,
        SV48 = 9,
        SV57 = 10,
        SV64 = 11,
    }
}

bits! {
    pub type SstatusBits: u64 {
        spp: 8,
        spie: 5,
        sie: 1
    }
}
