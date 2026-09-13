OUTPUT_ARCH(riscv)
ENTRY(_boot)
/* 内核物理加载地址，OpenSBI/QEMU 从 0x80200000 加载并跳到入口。 */
BASE_ADDRESS = 0x80200000;
/* 物理地址到高半区虚拟地址的偏移（Sv39）。 */
KERNEL_OFFSET = 0xffffffc000000000;

SECTIONS {
    /* ---- 低地址段：MMU 关闭时运行，VMA = LMA = BASE_ADDRESS ---- */
    . = BASE_ADDRESS;

    _kernel_low_start = .;

    .text.low : ALIGN(4K) {
        *(.text.low .text.low.*)
    }

    .rodata.low : ALIGN(4K) {
        *(.rodata.low .rodata.low*)
        *(.srodata.low .srodata.low*)
    }

    .data.low : ALIGN(4K) {
        *(.data.low .data.low*)
        *(.sdata.low .sdata.low*)
    }

    .bss.low : ALIGN(4K) {
        *(.bss.low .bss.low*)
        *(.sbss.low .sbss.low*)
    }

    . = ALIGN(4K);
    /* 低段物理结束地址，同时也是高段的物理加载起点。 */
    _kernel_low_end = .;

    /* ---- 高地址段：MMU 开启后运行，VMA 位于高半区，
       LMA 紧跟低段之后，保证物理镜像连续铺放 ---- */
    . = _kernel_low_end + KERNEL_OFFSET;

    _kernel_high_start = .;

    .text : AT(_kernel_low_end) ALIGN(4K) {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    .rodata : AT(ADDR(.rodata) - KERNEL_OFFSET) ALIGN(4K) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    .data : AT(ADDR(.data) - KERNEL_OFFSET) ALIGN(4K) {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    .bss : AT(ADDR(.bss) - KERNEL_OFFSET) ALIGN(4K) {
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }

    . = ALIGN(4K);

    _kernel_high_end = .;

    /* 整个内核镜像的物理末尾，供伙伴分配器确定可用内存起点。 */
    _end = _kernel_high_end - KERNEL_OFFSET;

    /DISCARD/ : {
        *(.eh_frame)
    }
}
