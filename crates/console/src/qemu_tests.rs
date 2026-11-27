//! `goethite-console` 的 QEMU 冒烟测试：确认 SBI 调试控制台可用。

use goethite_test::test_suite;

test_suite! {
    fn print_macros_write_to_console() {
        crate::print!("console: ");
        crate::println!("print macros work (value = {})", 42);
        crate::eprintln!("console: eprint shares the same console");
    }
}
