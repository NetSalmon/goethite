#![no_std]
//! 侵入式单链表。
//!
//! “下一个节点指针”存放在节点自身的首字处，不额外分配内存，供伙伴
//! 系统与 SLUB 的空闲链表复用。
use core::{
    fmt,
    fmt::{Debug, Formatter},
    marker::PhantomData,
};

/// 侵入式单链表。
///
/// “下一个节点指针”存放在节点自身的首字（`*mut usize`）处，不额外分配内存，
/// 供伙伴系统与 SLUB 的空闲链表复用。
#[derive(Copy, Clone)]
pub struct IntrusiveList {
    next: *mut usize,
}

impl Default for IntrusiveList {
    fn default() -> Self {
        Self::new()
    }
}

impl IntrusiveList {
    /// 构造一个空链表。
    pub const fn new() -> Self {
        Self {
            next: core::ptr::null_mut(),
        }
    }

    /// 判断链表是否为空（头指针为 `null`）。
    pub fn is_empty(&self) -> bool {
        self.next.is_null()
    }

    /// 把 `item` 作为新的头节点插入链表。
    ///
    /// `item` 必须指向一段可写内存，其首字用来存放原头节点，从而把新节点接到
    /// 链表前部。
    pub fn push(&mut self, item: *mut usize) {
        unsafe { *item = self.next as usize };
        self.next = item;
    }

    /// 弹出链表头节点，链表为空时返回 `None`。
    pub fn pop(&mut self) -> Option<*mut usize> {
        match self.is_empty() {
            true => None,
            false => {
                let item = self.next;
                self.next = unsafe { *item as *mut usize };
                Some(item)
            }
        }
    }

    /// 从链表中移除地址为 `item` 的节点，成功移除时返回 `true`。
    pub fn remove(&mut self, item: usize) -> bool {
        if self.next as usize == item {
            self.next = unsafe { *self.next as *mut usize };
            return true;
        }

        let mut current = self.next;
        while !current.is_null() {
            let next = unsafe { *current };
            if next == item {
                unsafe { *current = *(next as *mut usize) }
                return true;
            }
            current = unsafe { *current as *mut usize };
        }

        false
    }

    /// 返回一个只读迭代器，从头节点开始遍历链表。
    pub fn iter(&self) -> IntrusiveListIter<'_> {
        IntrusiveListIter {
            current: self.next,
            _marker: PhantomData,
        }
    }

    /// 返回一个可变迭代器，从头节点开始遍历链表。
    pub fn iter_mut(&mut self) -> IntrusiveListIterMut<'_> {
        IntrusiveListIterMut {
            current: self.next,
            _marker: PhantomData,
        }
    }
}

impl Debug for IntrusiveList {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.write_str("[ ")?;
        for i in self.iter() {
            fmt.write_fmt(format_args!("{:#x} ", i as usize))?;
        }
        fmt.write_str("]")
    }
}

impl PartialEq for IntrusiveList {
    fn eq(&self, other: &Self) -> bool {
        self.next == other.next
    }
}

/// 只读迭代器，遍历链表中的头指针。
///
/// 由 [`IntrusiveList::iter`] 构造。
pub struct IntrusiveListIter<'a> {
    current: *mut usize,
    _marker: PhantomData<&'a mut IntrusiveList>,
}

impl<'a> Iterator for IntrusiveListIter<'a> {
    type Item = *mut usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            None
        } else {
            let item = self.current;
            self.current = unsafe { *item as *mut usize };
            Some(item)
        }
    }
}

/// 可变迭代器，遍历链表中的头指针。
///
/// 由 [`IntrusiveList::iter_mut`] 构造。
pub struct IntrusiveListIterMut<'a> {
    current: *mut usize,
    _marker: PhantomData<&'a mut IntrusiveList>,
}

impl<'a> Iterator for IntrusiveListIterMut<'a> {
    type Item = *mut usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            None
        } else {
            let item = self.current;
            self.current = unsafe { *item as *mut usize };
            Some(item)
        }
    }
}
