//! 任务常量。
//!
//! [`TID_MAX`] 为 TID（任务标识）分配器的最大可分配值。
use goethite_macros::const_val;

/// TID 分配器的最大可分配值。
#[const_val]
pub const TID_MAX: usize = 1024;
