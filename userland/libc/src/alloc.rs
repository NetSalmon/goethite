//! POSIX `malloc` / `free` / `calloc` / `realloc` 堆分配器。
//!
//! 底层使用第三方 `linked_list_allocator::Heap` 管理静态 `.bss` 堆区，并在每个
//! `malloc` 返回的指针前放置 16 字节对齐的头部记录请求尺寸，使 `free` 与 `realloc`
//! 无需调用方传入 `Layout`。

use core::{
    alloc::Layout,
    cell::UnsafeCell,
    ffi::c_void,
    mem::{align_of, size_of},
    ptr::{self, NonNull},
    sync::atomic::{AtomicBool, Ordering},
};

use goethite_abi::PosixErrno;
use linked_list_allocator::Heap;

use crate::errno::set_errno;

const HEAP_SIZE: usize = 256 * 1024;
const ALIGN: usize = 16;

#[repr(C, align(16))]
struct BlockHeader {
    /// 整个分配块（含 `BlockHeader`）的字节数，用于还原 `Layout`。
    total_size: usize,
    _pad: usize,
}

const HEADER_SIZE: usize = size_of::<BlockHeader>();

#[allow(dead_code)]
const _: () = assert!(align_of::<BlockHeader>() == ALIGN);

#[repr(C, align(16))]
struct AlignedBuf([u8; HEAP_SIZE]);

struct GlobalHeap {
    locked: AtomicBool,
    inner: UnsafeCell<Heap>,
    storage: UnsafeCell<AlignedBuf>,
}

unsafe impl Sync for GlobalHeap {}

static HEAP: GlobalHeap = GlobalHeap {
    locked: AtomicBool::new(false),
    inner: UnsafeCell::new(Heap::empty()),
    storage: UnsafeCell::new(AlignedBuf([0; HEAP_SIZE])),
};

fn with_heap<R>(f: impl FnOnce(&mut Heap) -> R) -> R {
    while HEAP
        .locked
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        while HEAP.locked.load(Ordering::Relaxed) {
            core::hint::spin_loop();
        }
    }
    let heap = unsafe { &mut *HEAP.inner.get() };
    if heap.size() == 0 {
        let base = unsafe { (*HEAP.storage.get()).0.as_mut_ptr() };
        unsafe { heap.init(base, HEAP_SIZE) };
    }
    let res = f(heap);
    HEAP.locked.store(false, Ordering::Release);
    res
}

/// POSIX `malloc`。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    let req = size.max(1);
    let Some(total_size) = req.checked_add(HEADER_SIZE) else {
        set_errno(PosixErrno::ENOMEM.0 as i32);
        return ptr::null_mut();
    };
    let Ok(layout) = Layout::from_size_align(total_size, ALIGN) else {
        set_errno(PosixErrno::ENOMEM.0 as i32);
        return ptr::null_mut();
    };

    let raw = with_heap(|h| h.allocate_first_fit(layout).ok());
    let Some(nn) = raw else {
        set_errno(PosixErrno::ENOMEM.0 as i32);
        return ptr::null_mut();
    };

    let hdr = nn.as_ptr() as *mut BlockHeader;
    unsafe {
        hdr.write(BlockHeader {
            total_size,
            _pad: 0,
        });
        nn.as_ptr().add(HEADER_SIZE) as *mut c_void
    }
}

/// POSIX `free`。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let raw_ptr = unsafe { (ptr as *mut u8).sub(HEADER_SIZE) };
    let total_size = unsafe { (*(raw_ptr as *const BlockHeader)).total_size };
    let Ok(layout) = Layout::from_size_align(total_size, ALIGN) else {
        return;
    };
    if let Some(nn) = NonNull::new(raw_ptr) {
        with_heap(|h| unsafe { h.deallocate(nn, layout) });
    }
}

/// POSIX `calloc`。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut c_void {
    let Some(total) = nmemb.checked_mul(size) else {
        set_errno(PosixErrno::ENOMEM.0 as i32);
        return ptr::null_mut();
    };
    let p = unsafe { malloc(total) };
    if !p.is_null() && total > 0 {
        unsafe { ptr::write_bytes(p as *mut u8, 0, total) };
    }
    p
}

/// POSIX `realloc`。
#[cfg_attr(target_arch = "riscv64", unsafe(no_mangle))]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if ptr.is_null() {
        return unsafe { malloc(size) };
    }
    if size == 0 {
        unsafe { free(ptr) };
        return ptr::null_mut();
    }

    let raw_ptr = unsafe { (ptr as *const u8).sub(HEADER_SIZE) };
    let old_payload = unsafe { (*(raw_ptr as *const BlockHeader)).total_size - HEADER_SIZE };
    if size <= old_payload {
        return ptr;
    }

    let new_ptr = unsafe { malloc(size) };
    if new_ptr.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        ptr::copy_nonoverlapping(ptr as *const u8, new_ptr as *mut u8, old_payload.min(size));
        free(ptr);
    }
    new_ptr
}

/// 供 `staticlib`（`libc.a`）链接 `alloc` 时满足 Rust `#[global_allocator]` 约束。
#[cfg(all(target_arch = "riscv64", not(test)))]
struct LibcGlobalAlloc;

#[cfg(all(target_arch = "riscv64", not(test)))]
unsafe impl core::alloc::GlobalAlloc for LibcGlobalAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        with_heap(|h| {
            h.allocate_first_fit(layout)
                .map(NonNull::as_ptr)
                .unwrap_or(ptr::null_mut())
        })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(nn) = NonNull::new(ptr) {
            with_heap(|h| unsafe { h.deallocate(nn, layout) });
        }
    }
}

#[cfg(all(target_arch = "riscv64", not(test)))]
#[global_allocator]
static GLOBAL_ALLOC: LibcGlobalAlloc = LibcGlobalAlloc;
