//! 内核栈。
//!
//! 每块 [`Stack`] 拥有一段 [`KERNEL_STACK_SIZE`] 的栈内存，并用 [`Stack::sp`]
//! 记住离开这块栈时的栈指针。换栈见 [`Stack::switch_to`]：当前 `sp` 写入
//! 本栈，再从目标栈保存的 `sp` 恢复。
//!
//! 底层内存由 [`RefHandle`] 持有，因此 [`Stack`] 可以 [`Clone`]：克隆出的栈
//! 共享同一段栈内存，引用计数归零时才归还页块。

use core::sync::atomic::{Ordering, compiler_fence};

use crate::{
    KERNEL_STACK_SIZE,
    mm::object::{AllocError, Init, RefHandle},
    switch::{self, SWITCH_FRAME_SIZE},
};

/// 换栈时除切换帧以外，调用链本身还要占用的栈空间。
const SWITCH_HEADROOM: usize = 512;

#[repr(align(4096))]
pub struct Stack {
    pub handle: RefHandle<[u8; KERNEL_STACK_SIZE], Init>,
    /// 离开本栈时保存的栈指针。
    ///
    /// 新建时等于栈顶，表示还没有可恢复的帧。[`Stack::set_entry`] 写成初始帧
    /// 地址；[`Stack::switch_to`] 与 [`Stack::switch_out`] 在切走时写入当时的 `sp`。
    pub sp: usize,
}

impl Clone for Stack {
    /// 克隆共享同一段底层栈内存（引用计数加一），并复制当前保存的 `sp`。
    ///
    /// 注意：克隆出的 `Stack` 不是独立栈，若同时作为执行栈使用会互相覆盖；它适合
    /// 让多个引用指向同一个内核栈。
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            sp: self.sp,
        }
    }
}

impl Stack {
    pub fn try_new() -> Result<Stack, AllocError> {
        let handle = RefHandle::alloc()?;
        let handle = handle.init([0u8; KERNEL_STACK_SIZE]);
        let sp = raw_top(&handle) as usize;

        Ok(Stack { handle, sp })
    }

    pub fn new() -> Self {
        Self::try_new().unwrap()
    }

    /// 栈顶（高地址、开区间端点）。空栈的 `sp` 从这里开始向下增长。
    pub fn stack_top(&self) -> *const () {
        raw_top(&self.handle)
    }

    pub fn stack_bottom(&self) -> *const () {
        raw_bottom(&self.handle)
    }

    pub fn sp(&self) -> usize {
        self.sp
    }

    /// 在栈顶放好初始帧，使第一次切入时从 `entry` 开始执行。
    ///
    /// `entry` 运行在本栈上，且不能依赖调用者的栈帧。它应当再切回另一块栈；
    /// 若直接返回，内核会 panic。
    pub fn set_entry(&mut self, entry: extern "C" fn()) {
        let top = self.stack_top() as *mut u8;
        self.sp = unsafe { switch::write_entry_frame(top, entry as usize) as usize };
    }

    /// 把当前 `sp` 存进 `saved_sp`，并切入本栈。
    ///
    /// 用于从启动栈（不是 [`Stack`]）进入第一块内核栈。本栈必须已经有可恢复的
    /// 帧（[`Stack::set_entry`]，或先前从本栈切出）。`saved_sp` 在切回来之前
    /// 必须一直有效。
    pub fn switch_in(&mut self, saved_sp: &mut usize) {
        self.assert_frame();
        compiler_fence(Ordering::SeqCst);
        unsafe { switch::switch_stack(saved_sp, self.sp) };
        compiler_fence(Ordering::SeqCst);
    }

    /// 从本栈换到 `next`，并把当前 `sp` 写入 [`Stack::sp`]。
    ///
    /// 调用时必须正运行在 `self` 的栈内存上。`next` 必须已有可恢复的帧。
    /// 从 `next` 再切回本栈时，本函数返回。
    pub fn switch_to(&mut self, next: &mut Stack) {
        self.assert_can_switch();
        next.assert_frame();
        compiler_fence(Ordering::SeqCst);
        unsafe { switch::switch_stack(&mut self.sp, next.sp) };
        compiler_fence(Ordering::SeqCst);
    }

    /// 把当前 `sp` 写入 [`Stack::sp`]，并切到一个已经保存好的栈指针。
    ///
    /// # Safety
    ///
    /// `next_sp` 必须指向仍有效的切换帧（由 [`Stack::switch_in`] 或
    /// [`Stack::switch_to`] 写入），并且 16 字节对齐。调用时必须正运行在本栈上。
    pub unsafe fn switch_out(&mut self, next_sp: usize) {
        self.assert_can_switch();
        assert!(
            next_sp != 0 && next_sp % 16 == 0,
            "stack switch: bad target sp {next_sp:#x}"
        );
        compiler_fence(Ordering::SeqCst);
        unsafe { switch::switch_stack(&mut self.sp, next_sp) };
        compiler_fence(Ordering::SeqCst);
    }

    fn assert_frame(&self) {
        let bottom = self.stack_bottom() as usize;
        let top = self.stack_top() as usize;
        let sp = self.sp;
        assert!(
            sp % 16 == 0
                && sp >= bottom
                && sp
                    .checked_add(SWITCH_FRAME_SIZE)
                    .is_some_and(|end| end <= top),
            "stack switch: saved sp {sp:#x} is outside {bottom:#x}..{top:#x}"
        );
    }

    fn assert_can_switch(&self) {
        let bottom = self.stack_bottom() as usize;
        let top = self.stack_top() as usize;
        let sp = switch::current_sp();
        assert!(
            sp % 16 == 0 && sp >= bottom.saturating_add(SWITCH_HEADROOM) && sp <= top,
            "stack switch: current sp {sp:#x} is outside {bottom:#x}..{top:#x}"
        );
    }
}

fn raw_bottom(handle: &RefHandle<[u8; KERNEL_STACK_SIZE], Init>) -> *const () {
    unsafe { handle.as_ptr() }
}

fn raw_top(handle: &RefHandle<[u8; KERNEL_STACK_SIZE], Init>) -> *const () {
    // `as_ptr` 返回 `*const ()`。`()` 是零大小类型，对它 `offset` 不会按字节
    // 移动地址，所以先换成 `*const u8`。
    unsafe { (handle.as_ptr() as *const u8).add(KERNEL_STACK_SIZE) as *const () }
}
