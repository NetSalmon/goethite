//! QEMU 测试内核：把各 crate 的 `qemu-tests` 套件汇总后执行。
//!
//! 启动路径刻意保持最小：`boot.s` 设置栈、清 BSS、调用 [`rust_main`]，并挂载
//! 最小 S 模式陷阱处理入口 `_test_trap_entry`，防止未预期异常导致 QEMU 挂死超时。
//! 输出走 SBI 调试控制台，退出通过 QEMU virt 的 SiFive test finisher
//! （地址 `0x0010_0000`）完成，因此 `xtask` 可以拿到真实的进程退出码。

#![no_std]
#![no_main]

use core::{arch::global_asm, panic::PanicInfo};

use goethite_riscv64::registers::csr::{Scause, Sepc, Stval, Stvec};
use goethite_test::{Platform, TestSuite};

global_asm!(include_str!("boot.s"));

unsafe extern "C" {
    fn _test_trap_entry();
}

/// QEMU virt 的 SiFive test finisher。写 `0x5555` 退出 0；
/// 写 `0x3333 | (code << 16)` 以 `code` 退出。
const SIFIVE_TEST: *mut u32 = 0x0010_0000 as *mut u32;

struct SifiveTest;

impl Platform for SifiveTest {
    fn write_str(&self, message: &str) {
        if goethite_riscv64::sbi::dbcn::write_bytes(message.as_bytes()).is_err() {
            // 某些固件没有 DBCN，退回 legacy putchar。
            for byte in message.bytes() {
                goethite_riscv64::sbi::legacy::console_putchar(byte);
            }
        }
    }

    fn exit(&self, code: u8) -> ! {
        let value = if code == 0 {
            0x5555
        } else {
            0x3333 | ((code as u32) << 16)
        };
        // SAFETY: `SIFIVE_TEST` 是 QEMU virt 固定的 32 位 MMIO 寄存器。
        unsafe {
            self::SIFIVE_TEST.write_volatile(value);
        }
        loop {
            core::hint::spin_loop();
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_trap_abort() -> ! {
    let scause = Scause::read();
    let sepc = Sepc::read();
    let stval = Stval::read();
    let name = goethite_test::current_test();
    if name.is_empty() {
        goethite_console::println!(
            "\n[  FAILED  ] unexpected trap outside test: scause={scause:#x} sepc={sepc:#x} stval={stval:#x}"
        );
    } else {
        goethite_console::println!(
            "\n[  FAILED  ] {name}: unexpected trap: scause={scause:#x} sepc={sepc:#x} stval={stval:#x}"
        );
    }
    SifiveTest.exit(1)
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_main() -> ! {
    Stvec::write(_test_trap_entry as *const () as usize as u64);

    let platform = SifiveTest;
    let suites: &[TestSuite] = &[
        goethite_riscv64::qemu_tests::suite(),
        goethite_sync::qemu_tests::suite(),
        goethite_console::qemu_tests::suite(),
    ];
    let filter = option_env!("GOETHITE_TEST_FILTER");
    goethite_test::run_suites_filtered(suites, filter, &platform)
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    let name = goethite_test::current_test();
    if name.is_empty() {
        goethite_console::println!("\n[  FAILED  ] panic before test start: {}", info);
    } else {
        goethite_console::println!("\n[  FAILED  ] {}: panic: {}", name, info);
    }
    SifiveTest.exit(1)
}
