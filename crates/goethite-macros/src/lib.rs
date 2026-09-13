#![no_std]
//! 内核通用宏与编译期常量解析辅助函数。
//!
//! - [`macro@bits`]：为整数类型生成位域包装结构体，并为各比特字段提供读写访问器；
//! - [`macro@array_struct`]：为定长数组生成透明包装结构体，并提供按命名元素的访问器；
//! - [`macro@numeric`]：为整数类型生成 new type，携带命名常量与可选取值区间；
//! - [`macro@const_val`]：带约束的编译期常量（min / max / multiple_of + env 覆写）；
//! - [`macro@lazy`] / [`macro@spin`] / [`macro@rwlock`]：懒加载 / 自旋锁 / 读写锁静态；
//! - [`macro@load_symbol`]：安全加载 64 位符号地址（RISC-V 下规避 PC 相对寻址越界）；
//! - [`num`]：编译期数字 / 布尔字符串解析，由 [`macro@const_val`] 内部调用。
//! - [`derive@Id`]：为单字段结构体自动实现 `goethite_id_alloc::Id`。

pub mod num;

pub use goethite_macros_impl::{
    Id, array_struct, bits, const_val, lazy, numeric, rwlock, spin,
};

/// 安全加载 64 位符号地址。
///
/// 在 RISC-V 架构下，当代码处于低地址（例如 boot 阶段）需要跳转或引用
/// 高半区内核符号（例如 `__kernel_entry`、`main`）时，普通的 PC 相对寻址
/// （PCREL，如 `la` 指令）受限于 32 位寻址范围（±2 GiB），跨越虚拟/物理边界
/// 会导致链接器报错（`relocation out of range`）。
///
/// 本宏使用 `.quad` 伪指令在代码段内放置 8 字节绝对重定位表项，并使用
/// 本地相对寻址 `ld` 指令安全读入寄存器，彻底消除重定位越界问题。
///
/// 支持传入符号标识符（如 `__kernel_entry`）或字符串字面量（如 `"__kernel_entry"`）。
///
/// # 语法
///
/// - `load_symbol!(var, symbol)` / `load_symbol!(var, "symbol")`: 将符号地址写入已有变量 `var` 中。
/// - `load_symbol!(let var: $ty, symbol)`: 声明指定类型的变量并加载。
/// - `load_symbol!(let var, symbol)`: 声明 `usize` 类型的变量并加载。
/// - `load_symbol!(symbol)`: 表达式形式，返回符号的 `usize` 地址。
#[macro_export]
macro_rules! load_symbol {
    // 形式 1: 声明带类型的变量并加载
    (let $var:ident : $ty:ty, $sym:tt) => {
        let $var: $ty;
        $crate::load_symbol!($var, $sym);
    };
    // 形式 2: 声明带 mut 和类型的变量并加载
    (let mut $var:ident : $ty:ty, $sym:tt) => {
        let mut $var: $ty;
        $crate::load_symbol!($var, $sym);
    };
    // 形式 3: 声明默认 usize 的变量并加载
    (let $var:ident, $sym:tt) => {
        let $var: usize;
        $crate::load_symbol!($var, $sym);
    };
    // 形式 4: 声明默认 mut usize 的变量并加载
    (let mut $var:ident, $sym:tt) => {
        let mut $var: usize;
        $crate::load_symbol!($var, $sym);
    };
    // 形式 5: 填入变量和字符串字面量 -> 直接将符号地址写入已有变量中
    ($var:expr, $sym:literal) => {
        unsafe {
            ::core::arch::asm!(
                "ld {out}, 1f",
                "j 2f",
                ".balign 8",
                "1:",
                concat!(".quad ", $sym),
                "2:",
                out = out(reg) $var,
            );
        }
    };
    // 形式 6: 填入变量和符号标识符 -> 转换为字符串后加载
    ($var:expr, $sym:ident) => {
        unsafe {
            ::core::arch::asm!(
                "ld {out}, 1f",
                "j 2f",
                ".balign 8",
                "1:",
                concat!(".quad ", stringify!($sym)),
                "2:",
                out = out(reg) $var,
            );
        }
    };
    // 形式 7: 作为表达式返回值
    ($sym:tt) => {{
        let out: usize;
        $crate::load_symbol!(out, $sym);
        out
    }};
}

/// 在执行 `$body` 期间关闭中断（通过 [`goethite_riscv64::InterruptGuard`]），
/// 退出作用域时自动恢复此前的中断使能状态。
///
/// # 依赖
///
/// 本宏展开后会引用 `goethite_riscv64::InterruptGuard`，因此**调用方 crate 必须把
/// `goethite-riscv64` 加为（直接或通过传递可达的）依赖**，否则宏展开处无法解析该路径。
#[macro_export]
macro_rules! critical_section {
    ($($body:tt)*) => {
        {
            let _guard = goethite_riscv64::InterruptGuard::new();

            $($body)*
        }
    }
}
