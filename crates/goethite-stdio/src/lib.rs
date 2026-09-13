//! 内核控制台输出与 `print` 系列宏。
//!
//! 通过 SBI 调试控制台（DBCN）输出文本，向内核提供
//! [`print!`](macro@print)、[`println!`](macro@println)、
//! [`eprint!`](macro@eprint)、[`eprintln!`](macro@eprintln) 一组宏，
//! 均支持 [`fmt`] 的格式化语法。

#![no_std]

use core::fmt::{self, Write};

/// 通过 SBI 调试控制台 (DBCN) 输出字节的写入器。
struct ConsoleWriter;

impl Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let _ = goethite_riscv64::sbi::dbcn::write_bytes(s.as_bytes());
        Ok(())
    }
}

/// `print!` / `println!` 宏的内部入口：把格式化结果写入调试控制台。
///
/// 该函数会被 `$crate::_print` 从外部 crate 引用，因此标记为
/// `#[doc(hidden)] pub`。
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    let _ = ConsoleWriter.write_fmt(args);
}

/// `eprint!` / `eprintln!` 宏的内部入口。
///
/// 当前内核没有独立的错误输出流，暂与调试控制台共用同一个通道；
/// 保留独立入口以便将来区分 stdout / stderr。
#[doc(hidden)]
pub fn _eprint(args: fmt::Arguments<'_>) {
    let _ = ConsoleWriter.write_fmt(args);
}

/// 向调试控制台输出格式化内容，不追加换行。
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::_print(format_args!($($arg)*));
    }};
}

/// 向调试控制台输出一行（末尾追加换行符）。
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n");
    }};
}

/// 向错误通道输出格式化内容，不追加换行。当前复用调试控制台。
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {{
        $crate::_eprint(format_args!($($arg)*));
    }};
}

/// 向错误通道输出一行（末尾追加换行符）。
#[macro_export]
macro_rules! eprintln {
    () => { $crate::eprint!("\n") };
    ($($arg:tt)*) => {{
        $crate::eprint!($($arg)*);
        $crate::eprint!("\n");
    }};
}
