    .section .text.entry
    .globl _start
_start:
    la sp, __stack_top
    li tp, 0

    la t0, __bss_start
    la t1, __bss_end
1:
    bgeu t0, t1, 2f
    sd zero, 0(t0)
    addi t0, t0, 8
    j 1b
2:
    call rust_main
3:
    j 3b

    .section .text
    .align 2
    .globl _test_trap_entry
_test_trap_entry:
    // 发生未预期陷阱时切换到独立异常栈，避免栈溢出导致二次陷阱。
    la sp, __trap_stack_top
    call rust_trap_abort
1:
    j 1b

    .section .bss.stack, "aw", @nobits
    .align 12
__stack_bottom:
    .space 64 * 1024
__stack_top:
    .space 16 * 1024
__trap_stack_top:
