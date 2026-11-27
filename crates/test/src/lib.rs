#![no_std]
//! `no_std` 测试框架，用于在 QEMU 中运行内核 crate 的测试。
//!
//! 本 crate 提供与平台无关的测试套件注册、断言宏、过滤器、输出格式与结果统计；
//! 真正的串口输出和“退出 QEMU”由测试内核通过 [`Platform`] 注入。
//!
//! # 在 crate 中注册 QEMU 测试
//!
//! ```ignore
//! use goethite_test::{assert_eq, test_suite};
//!
//! test_suite! {
//!     fn addition_works() {
//!         assert_eq!(1 + 1, 2);
//!     }
//!
//!     #[ignore]
//!     fn skipped_case() {
//!         assert_eq!(1, 2);
//!     }
//! }
//! ```
//!
//! 宏会自动将测试体包装为返回 `Result<(), TestFailure>` 的函数，断言失败时直接
//! `return Err(..)` 并附带 `file:line` 位置，不依赖 `std` 的 panic 捕获。
//!
//! # 运行结果
//!
//! [`run_suites`] 会输出每个测试的状态，最后输出一行机器可读的汇总：
//!
//! ```text
//! GOETHITE_TEST_SUMMARY passed=12 failed=0 ignored=0
//! ```
//!
//! `xtask`（`cargo xtest qemu`）通过该行与 QEMU 的退出码判断测试是否通过。

use core::{
    fmt::{self, Write as _},
    ptr,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

/// 单条失败信息能够保存的最大字节数。
const MAX_MESSAGE: usize = 512;

/// 测试失败信息（固定容量，无需分配器）。
#[derive(Clone)]
pub struct TestFailure {
    file: &'static str,
    line: u32,
    bytes: [u8; MAX_MESSAGE],
    len: usize,
}

impl TestFailure {
    /// 用 `core::fmt` 参数构造失败信息（不含位置信息）。
    pub fn new(args: fmt::Arguments<'_>) -> Self {
        Self::at("", 0, args)
    }

    /// 构造附带源码位置（`file:line`）的失败信息。
    pub fn at(file: &'static str, line: u32, args: fmt::Arguments<'_>) -> Self {
        let mut failure = Self {
            file,
            line,
            bytes: [0; MAX_MESSAGE],
            len: 0,
        };
        let _ = failure.write_fmt(args);
        failure
    }

    /// 失败信息文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("<truncated failure message>")
    }

    /// 失败发生的源文件路径（未提供时为空字符串）。
    #[must_use]
    pub const fn file(&self) -> &'static str {
        self.file
    }

    /// 失败发生的行号（未提供时为 `0`）。
    #[must_use]
    pub const fn line(&self) -> u32 {
        self.line
    }
}

impl fmt::Display for TestFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.file.is_empty() && self.line > 0 {
            write!(f, "{}:{}: {}", self.file, self.line, self.as_str())
        } else {
            f.write_str(self.as_str())
        }
    }
}

impl fmt::Debug for TestFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestFailure")
            .field("file", &self.file)
            .field("line", &self.line)
            .field("message", &self.as_str())
            .finish()
    }
}

impl fmt::Write for TestFailure {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let remaining = MAX_MESSAGE - self.len;
        let mut n = s.len().min(remaining);
        // 避免从 UTF-8 字符中间截断。
        while n > 0 && !s.is_char_boundary(n) {
            n -= 1;
        }
        self.bytes[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// 测试函数类型。
pub type TestFn = fn() -> Result<(), TestFailure>;

/// 一条已注册的测试。
#[derive(Clone, Copy)]
pub struct TestCase {
    /// 测试名（用于输出和定位 panic）。
    pub name: &'static str,
    /// 测试函数。
    pub run: TestFn,
    /// 是否标记为忽略（跳过执行）。
    pub ignored: bool,
}

impl TestCase {
    /// 构造一条测试。
    pub const fn new(name: &'static str, run: TestFn) -> Self {
        Self {
            name,
            run,
            ignored: false,
        }
    }

    /// 构造一条被忽略的测试。
    pub const fn ignored(name: &'static str, run: TestFn) -> Self {
        Self {
            name,
            run,
            ignored: true,
        }
    }
}

/// 一组具名测试套件。
#[derive(Clone, Copy)]
pub struct TestSuite {
    /// 套件名（默认取注册位置的 `module_path!()`）。
    pub name: &'static str,
    /// 套件内的全部测试用例。
    pub tests: &'static [TestCase],
}

impl TestSuite {
    /// 构造一个测试套件。
    pub const fn new(name: &'static str, tests: &'static [TestCase]) -> Self {
        Self { name, tests }
    }
}

/// 测试执行汇总结果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TestSummary {
    /// 通过的测试数。
    pub passed: usize,
    /// 失败的测试数。
    pub failed: usize,
    /// 忽略/跳过的测试数。
    pub ignored: usize,
}

impl TestSummary {
    /// 是否全部通过（无失败用例）。
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.failed == 0
    }
}

/// QEMU 测试内核需要提供的平台能力。
pub trait Platform {
    /// 输出一段文本（通常写到 SBI 调试控制台）。
    fn write_str(&self, message: &str);

    /// 结束整个测试进程。`code == 0` 表示全部通过。
    ///
    /// QEMU 测试内核可通过 SiFive test finisher 或 SBI SRST 实现。
    fn exit(&self, code: u8) -> !;
}

/// 便于在测试套件中构造 `Result::Err`，自动附带当前源码位置。
#[macro_export]
macro_rules! fail {
    ($($arg:tt)*) => {
        return ::core::result::Result::Err($crate::TestFailure::at(
            file!(),
            line!(),
            format_args!($($arg)*),
        ))
    };
}

/// 断言条件为真；失败时结束当前测试并报告表达式与源码位置。
#[macro_export]
macro_rules! assert {
    ($cond:expr $(,)?) => {
        if !$cond {
            $crate::fail!("assertion failed: {}", stringify!($cond));
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            $crate::fail!("{}: {}", format_args!($($arg)+), stringify!($cond));
        }
    };
}

/// 断言两个值相等；失败时打印两侧的值、表达式与源码位置。
#[macro_export]
macro_rules! assert_eq {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (left_value, right_value) => {
                if !(*left_value == *right_value) {
                    $crate::fail!(
                        "assertion `{} == {}` failed: left = {:?}, right = {:?}",
                        stringify!($left),
                        stringify!($right),
                        left_value,
                        right_value
                    );
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_value, right_value) => {
                if !(*left_value == *right_value) {
                    $crate::fail!(
                        "{}: `{} == {}` failed: left = {:?}, right = {:?}",
                        format_args!($($arg)+),
                        stringify!($left),
                        stringify!($right),
                        left_value,
                        right_value
                    );
                }
            }
        }
    };
}

/// 断言两个值不相等。
#[macro_export]
macro_rules! assert_ne {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (left_value, right_value) => {
                if *left_value == *right_value {
                    $crate::fail!(
                        "assertion `{} != {}` failed: both = {:?}",
                        stringify!($left),
                        stringify!($right),
                        left_value
                    );
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_value, right_value) => {
                if *left_value == *right_value {
                    $crate::fail!(
                        "{}: `{} != {}` failed: both = {:?}",
                        format_args!($($arg)+),
                        stringify!($left),
                        stringify!($right),
                        left_value
                    );
                }
            }
        }
    };
}

/// 断言表达式匹配给定模式。
#[macro_export]
macro_rules! assert_matches {
    ($expr:expr, $pat:pat $(if $guard:expr)? $(,)?) => {
        match $expr {
            $pat $(if $guard)? => {}
            ref value => {
                $crate::fail!(
                    "assertion `matches!({}, {})` failed: value = {:?}",
                    stringify!($expr),
                    stringify!($pat $(if $guard)?),
                    value
                );
            }
        }
    };
    ($expr:expr, $pat:pat $(if $guard:expr)?, $($arg:tt)+) => {
        match $expr {
            $pat $(if $guard)? => {}
            ref value => {
                $crate::fail!(
                    "{}: `matches!({}, {})` failed: value = {:?}",
                    format_args!($($arg)+),
                    stringify!($expr),
                    stringify!($pat $(if $guard)?),
                    value
                );
            }
        }
    };
}

/// 内部辅助宏：把 `test_suite!` 中的单个函数条目构造成 [`TestCase`]。
#[doc(hidden)]
#[macro_export]
macro_rules! __test_case {
    (@case $name:ident, #[ignore $(= $reason:literal)?] $($rest:tt)*) => {
        $crate::TestCase::ignored(stringify!($name), $name)
    };
    (@case $name:ident, #[$($other:tt)*] $($rest:tt)*) => {
        $crate::__test_case!(@case $name, $($rest)*)
    };
    (@case $name:ident,) => {
        $crate::TestCase::new(stringify!($name), $name)
    };
}

/// 把一组测试函数展开为 `tests() -> &'static [TestCase]` 与 `suite() -> TestSuite`。
///
/// 测试函数无需手动返回 `Ok(())`；支持在单个函数上标注 `#[ignore]` 以跳过执行。
#[macro_export]
macro_rules! test_suite {
    ($($(#[$($attr:tt)*])* fn $name:ident() $body:block)*) => {
        $(
            #[allow(unreachable_code)]
            fn $name() -> ::core::result::Result<(), $crate::TestFailure> {
                let (): () = $body;
                ::core::result::Result::Ok(())
            }
        )*

        /// 本模块注册的全部测试。
        pub fn tests() -> &'static [$crate::TestCase] {
            static TESTS: &[$crate::TestCase] = &[
                $($crate::__test_case!(@case $name, $(#[$($attr)*])*)),*
            ];
            TESTS
        }

        /// 本模块对应的具名测试套件。
        pub fn suite() -> $crate::TestSuite {
            $crate::TestSuite::new(module_path!(), tests())
        }
    };
}

static CURRENT_TEST_PTR: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
static CURRENT_TEST_LEN: AtomicUsize = AtomicUsize::new(0);

fn set_current_test(name: &'static str) {
    CURRENT_TEST_LEN.store(name.len(), Ordering::Release);
    CURRENT_TEST_PTR.store(name.as_ptr().cast_mut(), Ordering::Release);
}

/// 当前正在执行的测试名；panic / trap 处理器可用它报告失败的测试。
#[must_use]
pub fn current_test() -> &'static str {
    let ptr = CURRENT_TEST_PTR.load(Ordering::Acquire);
    let len = CURRENT_TEST_LEN.load(Ordering::Acquire);
    if ptr.is_null() {
        return "";
    }
    // SAFETY: `set_current_test` 只接受 `&'static str`，指针/长度指向
    // 静态字符串数据；读取端用 Acquire 与写入端的 Release 同步。
    unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) }
}

fn matches_filter(suite_name: &str, test_name: &str, filter: Option<&str>) -> bool {
    let Some(needle) = filter else {
        return true;
    };
    if needle.is_empty() {
        return true;
    }
    suite_name.contains(needle) || test_name.contains(needle)
}

struct Reporter<'a, P: Platform>(&'a P);

impl<P: Platform> fmt::Write for Reporter<'_, P> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.write_str(s);
        Ok(())
    }
}

/// 运行多个具名测试套件并返回统计结果，不主动退出进程。
///
/// 当 `filter` 为 `Some(substr)` 且非空时，仅运行套件名或测试名包含 `substr` 的用例。
pub fn run_suites_with_filter<W: fmt::Write>(
    suites: &[TestSuite],
    filter: Option<&str>,
    reporter: &mut W,
) -> TestSummary {
    let total: usize = suites
        .iter()
        .map(|suite| {
            suite
                .tests
                .iter()
                .filter(|test| matches_filter(suite.name, test.name, filter))
                .count()
        })
        .sum();

    if let Some(needle) = filter.filter(|s| !s.is_empty()) {
        let _ = writeln!(
            reporter,
            "goethite test harness: {} suite(s), {} test(s) (filter: \"{}\")",
            suites.len(),
            total,
            needle
        );
    } else {
        let _ = writeln!(
            reporter,
            "goethite test harness: {} suite(s), {} test(s)",
            suites.len(),
            total
        );
    }

    let mut summary = TestSummary::default();

    for suite in suites {
        for test in suite.tests {
            if !matches_filter(suite.name, test.name, filter) {
                continue;
            }
            if test.ignored {
                summary.ignored += 1;
                let _ = writeln!(reporter, "[ SKIPPED  ] {}::{}", suite.name, test.name);
                continue;
            }

            set_current_test(test.name);
            let _ = writeln!(reporter, "[ RUN      ] {}::{}", suite.name, test.name);
            match (test.run)() {
                Ok(()) => {
                    summary.passed += 1;
                    let _ = writeln!(reporter, "[       OK ] {}::{}", suite.name, test.name);
                }
                Err(failure) => {
                    summary.failed += 1;
                    let _ = writeln!(
                        reporter,
                        "[  FAILED  ] {}::{}: {}",
                        suite.name, test.name, failure
                    );
                }
            }
        }
    }

    set_current_test("");
    let _ = writeln!(
        reporter,
        "GOETHITE_TEST_SUMMARY passed={} failed={} ignored={}",
        summary.passed, summary.failed, summary.ignored
    );

    summary
}

/// 运行多个具名测试套件（支持可选过滤子串），最后调用 [`Platform::exit`]。
pub fn run_suites_filtered<P: Platform>(
    suites: &[TestSuite],
    filter: Option<&str>,
    platform: &P,
) -> ! {
    let mut reporter = Reporter(platform);
    let summary = run_suites_with_filter(suites, filter, &mut reporter);
    platform.exit(if summary.is_success() { 0 } else { 1 })
}

/// 运行多个具名测试套件，最后调用 [`Platform::exit`]。
pub fn run_suites<P: Platform>(suites: &[TestSuite], platform: &P) -> ! {
    run_suites_filtered(suites, None, platform)
}

/// 运行单个具名测试套件。
pub fn run<P: Platform>(suite: TestSuite, platform: &P) -> ! {
    run_suites(&[suite], platform)
}
