OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS {
    /* QEMU virt + OpenSBI 将内核加载到 0x8020_0000。 */
    . = 0x80200000;

    .text : {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    .rodata : ALIGN(16) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    .data : ALIGN(16) {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    .bss : ALIGN(16) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
    }
}
