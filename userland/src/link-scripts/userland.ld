OUTPUT_ARCH(riscv)
ENTRY(_start)
/* 用户程序加载基址：内核按 ELF p_vaddr 映射，无固定要求，取低地址即可。 */
BASE_ADDRESS = 0x10000;

SECTIONS {
    . = BASE_ADDRESS;

    .text : ALIGN(4K) {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    .bss : ALIGN(4K) {
        *(.bss .bss.*)
        *(COMMON)
        *(.sbss .sbss.*)
    }

    /DISCARD/ : {
        *(.eh_frame)
    }
}
