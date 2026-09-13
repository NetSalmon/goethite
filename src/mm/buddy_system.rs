//! 伙伴系统物理页分配器。
//!
//! 以 2 的幂次阶（order）维护空闲链表：分配时从高到低查找并逐级对半
//! 拆分，释放时尝试与 buddy 合并成大块。
//!
//! `BuddyAllocator` 通过 const 泛型 `MAX_ORDER` 参数化空闲链表的档数，
//! 因此每个实例能在类型层面静态地编码自己的最大阶数，和配置解耦。

use core::ops::Range;

use goethite_config::*;
use goethite_intrusive_list::IntrusiveList;

/// 伙伴系统分配器：按阶（order）组织空闲页块。
pub struct BuddyAllocator<const MAX_ORDER: usize> {
    /// 各阶的空闲块链表；`free_list[order]` 存放大小为 `order_size(order)` 的块。
    pub free_list: [IntrusiveList; MAX_ORDER],
}

impl<const MAX_ORDER: usize> BuddyAllocator<MAX_ORDER> {
    /// 构造一个所有空闲链表均为空的分配器。
    pub const fn new() -> Self {
        Self {
            free_list: [IntrusiveList::new(); MAX_ORDER],
        }
    }

    fn add_frame(&mut self, start: usize, end: usize) {
        let mut current = start;
        for order in (0..MAX_ORDER).rev() {
            if current >= end {
                break;
            }
            let order_size = order_size(order);

            let chunks = (end - current) / order_size;

            for _ in 0..chunks {
                self.free_list[order].push(current as *mut _);
                current += order_size;
            }
        }
    }

    /// 把一段物理地址范围 [`Range`]（字节寻址）交给分配器管理。
    ///
    /// 起始地址会对齐到该分配器的最大阶块大小，然后在 `add_frame` 中按阶
    /// 拆分并以空闲块形式挂入对应链表。
    pub fn insert(&mut self, range: Range<usize>) {
        let start = align(range.start, 1 << (MAX_ORDER + 11));
        self.add_frame(start, range.end);
    }

    /// 分配——按指定阶 `order` 返回一个空闲物理页块的首地址，若没有足够大
    /// 的空闲块则返回 `None`。
    ///
    /// 若高阶级存在空闲块，会逐级对半拆分，把剩余的一半挂回低阶链表。
    pub fn alloc(&mut self, order: usize) -> Option<usize> {
        for i in order..self.free_list.len() {
            if self.free_list[i].is_empty() {
                continue;
            }

            let chunk = self.free_list[i].pop()? as usize;

            for idx in (order..i).rev() {
                let left = chunk + order_size(idx);
                self.free_list[idx].push(left as *mut _);
            }

            return Some(chunk);
        }

        None
    }

    /// 归还——把从 `start` 开始、阶为 `order` 的块释放回分配器，并尝试与
    /// 相邻的 buddy 块向上合并成更大的块。
    pub fn dealloc(&mut self, start: usize, order: usize) {
        let mut current_ptr = start;
        let mut current_order = order;

        for i in order..MAX_ORDER {
            let buddy = buddy_address(current_ptr, current_order);
            if self.free_list[current_order].remove(buddy) {
                current_ptr = if buddy < current_ptr {
                    buddy
                } else {
                    current_ptr
                };
                current_order = i + 1;
                if current_order >= MAX_ORDER {
                    current_order = MAX_ORDER - 1;
                    break;
                }
            } else {
                break;
            }
        }

        self.free_list[current_order].push(current_ptr as *mut _);
    }

    /// 按字节大小分配，自动换算为阶。
    pub fn alloc_frame(&mut self, size: usize) -> Option<usize> {
        let order = size_to_order(size, MAX_ORDER);
        self.alloc(order)
    }

    /// 按字节大小归还，自动换算为阶。
    pub fn dealloc_frame(&mut self, start: usize, size: usize) {
        let order = size_to_order(size, MAX_ORDER);
        self.dealloc(start, order)
    }
}

// SAFETY: `free_list` 里的节点指针只通过 `&mut self` 解引用，且访问都发生在
// 单一线程里（由外层 `SpinLock` 串行化）。因此把分配器在已串行化的执行流之间
// 移动是安全的，手动断言 `Send` 以便它能被放进 `SpinLock` 成为 `Sync` 的全局。
unsafe impl<const MAX_ORDER: usize> Send for BuddyAllocator<MAX_ORDER> {}

/// 把字节大小换算为需要的阶（order），最多不超过 `max_order - 1`。
#[inline]
pub fn size_to_order(size: usize, max_order: usize) -> usize {
    let n = ilog2_ceil(size);
    if n <= PAGE_SIZE_LOG_2 {
        0
    } else {
        (n - PAGE_SIZE_LOG_2).min(max_order - 1)
    }
}

/// 计算与 `base`（阶为 `order`）相邻的 buddy 块地址：即翻转对应阶的位。
#[inline]
pub fn buddy_address(base: usize, order: usize) -> usize {
    base ^ order_size(order)
}

/// 阶 `order` 对应的块大小（字节）：`1 << order << PAGE_SIZE_LOG_2`。
#[inline]
pub fn order_size(order: usize) -> usize {
    1 << order << PAGE_SIZE_LOG_2
}
