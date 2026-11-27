//! POSIX 兼容用户程序入口（`crt0`）。
//!
//! 从内核写入栈顶的 [`goethite_stack`] 映像中还原标准 C 风格 `argc`、`argv` 与
//! 全局 `environ`，再按 `main(argc, argv, environ)` 约定调用 C/POSIX 主函数。

#![cfg(all(target_arch = "riscv64", not(test)))]

use core::{
    arch::{asm, naked_asm},
    ffi::{c_char, c_int},
    mem::size_of,
    ptr,
};

use goethite_stack::StackRef;

use crate::{alloc::malloc, env, stdlib::exit};

unsafe extern "C" {
    fn main(argc: c_int, argv: *mut *mut c_char, envp: *mut *mut c_char) -> c_int;
}

unsafe fn image_bytes(image: *const u8) -> &'static [u8] {
    let total = u32::from_le_bytes([
        unsafe { *image.add(8) },
        unsafe { *image.add(9) },
        unsafe { *image.add(10) },
        unsafe { *image.add(11) },
    ]) as usize;
    unsafe { core::slice::from_raw_parts(image, total) }
}

/// RISC-V ELF 入口：从 `0(sp)` 取出 `goethite-stack` 映像指针并转入初始化。
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    naked_asm!(
        "ld a0, 0(sp)",
        "j {entry}",
        entry = sym __goethite_libc_start,
    )
}

extern "C" fn __goethite_libc_start(image: *const u8) -> ! {
    let mut argc: c_int = 0;
    let mut argv: *mut *mut c_char = ptr::null_mut();

    if !image.is_null() {
        if let Ok(stack) = StackRef::parse(unsafe { image_bytes(image) }) {
            // 1. 初始化 POSIX `environ`
            unsafe { env::init_from_stack(&stack) };

            // 2. 在堆上构造 NUL 结尾的 C 风格 `argv`
            let count = stack.argc();
            let arr = unsafe { malloc((count + 1) * size_of::<*mut c_char>()) as *mut *mut c_char };
            if !arr.is_null() {
                for (i, arg) in stack.argv().enumerate() {
                    let bytes = arg.as_bytes();
                    let buf = unsafe { malloc(bytes.len() + 1) as *mut u8 };
                    if buf.is_null() {
                        break;
                    }
                    unsafe {
                        ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
                        *buf.add(bytes.len()) = 0;
                        *arr.add(i) = buf as *mut c_char;
                    }
                    argc = (i + 1) as c_int;
                }
                unsafe { *arr.add(argc as usize) = ptr::null_mut() };
                argv = arr;
            }
        }
    }

    let envp = unsafe { env::environ };
    let ret: c_int;
    unsafe {
        asm!(
            "call {main}",
            in("a0") argc,
            in("a1") argv,
            in("a2") envp,
            lateout("a0") ret,
            main = sym main,
            clobber_abi("C"),
        );
    }
    exit(ret)
}
