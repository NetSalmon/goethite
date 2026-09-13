//! 链接器符号。
//!
//! `_kernel_low_start` / `_kernel_low_end` 为内核低半区（物理地址）的起止，
//! `_kernel_high_start` / `_kernel_high_end` 为高半区（虚拟地址）的起止，
//! `_end` 为内核镜像末尾（`_kernel_high_end - KERNEL_OFFSET`）。这些符号均由
//! 链接脚本 `link-scripts/linker.ld` 提供。
unsafe extern "C" {
    /// 内核低半区起点（映射前，物理地址），由链接脚本 `_kernel_low_start` 提供。
    pub fn _kernel_low_start();
    /// 内核低半区终点（映射前，物理地址）。
    pub fn _kernel_low_end();
    /// 内核高半区起点（切换到 `KERNEL_OFFSET` 后的虚拟地址）。
    pub fn _kernel_high_start();
    /// 内核高半区终点。
    pub fn _kernel_high_end();
    /// 内核镜像末尾的物理地址（`_kernel_high_end - KERNEL_OFFSET`），空闲内存由此开始。
    pub fn _end();
}
