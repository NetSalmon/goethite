//! 平台重启 / 电源控制。
//!
//! 识别 Linux 风格的 `reboot(2)` 魔数命令，经 SBI SRST / HSM 扩展执行关机、
//! 冷复位或停机。属于平台辅助而非 [`Driver`](crate::device::driver::Driver)，
//! 因此不参与设备树探测与设备登记。
use goethite_riscv64::sbi::{
    hsm,
    srst::{self, ResetReason, ResetType},
};

/// 按 `command` 执行一次平台复位；命令无法识别时返回 `false`。
///
/// - `0x4321_fedc`：关机（`LINUX_REBOOT_CMD_POWER_OFF`）；
/// - `0x0123_4567`：重启（`LINUX_REBOOT_CMD_RESTART`）；
/// - `0xcdef_0123`：停机（`LINUX_REBOOT_CMD_HALT`）。
pub fn reboot(command: u64) -> bool {
    match command {
        0x4321_fedc => {
            let _ = srst::system_reset(ResetType::SHUTDOWN, ResetReason::NONE);
        }
        0x0123_4567 => {
            let _ = srst::system_reset(ResetType::COLD_REBOOT, ResetReason::NONE);
        }
        0xcdef_0123 => {
            let _ = hsm::hart_stop();
        }
        _ => return false,
    }
    true
}
