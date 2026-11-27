//! 在 QEMU（S 模式 + OpenSBI）中运行的硬件/固件冒烟测试。
//!
//! 这些测试通过 `goethite-test-kernel` 的 `qemu-tests` feature 注册，不参与
//! 普通内核构建。

use goethite_test::{assert, assert_eq, test_suite};

use crate::registers::{csr, values::SstatusValue};

test_suite! {
    fn csr_sscratch_roundtrip() {
        let original = csr::Sscratch::read();
        csr::Sscratch::write(0x1234_5678_9abc_def0);
        assert_eq!(csr::Sscratch::read(), 0x1234_5678_9abc_def0);
        csr::Sscratch::write(original);
    }

    fn interrupt_disable_restores_state() {
        let before = SstatusValue::from(csr::Sstatus::read());
        let previous = crate::disable_interrupt();
        let during = SstatusValue::from(csr::Sstatus::read());
        assert!(!during.sie(), "disable_interrupt must clear sstatus.SIE");

        crate::enable_interrupt(previous);
        let after = SstatusValue::from(csr::Sstatus::read());
        assert_eq!(after.sie(), before.sie());
    }

    fn sbi_base_extension_is_available() {
        assert!(
            crate::sbi::base::get_spec_version().is_ok(),
            "SBI base extension must respond"
        );
        assert!(
            crate::sbi::base::probe_extension(crate::sbi::Eid::BASE).is_ok(),
            "probe_extension(BASE) must succeed"
        );
    }

    fn hart_id_is_initialised() {
        // 测试内核的 boot.s 将 tp 置 0，因此这里也顺带验证 gpr 读写路径。
        assert_eq!(crate::hart_id(), 0);
    }

    fn fences_do_not_fault() {
        crate::instruction_fence();
        crate::address_translation_fence();
    }
}
