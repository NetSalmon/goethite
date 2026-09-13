//! 设备驱动注册表。
//!
//! [`DRIVERS`] 持有所有已注册的驱动器（[`Driver`] trait object）。当前仅完成
//! 骨架：具体驱动的枚举、与设备描述的匹配以及 bind/unbind 流程尚未落地。
use alloc::{boxed::Box, vec::Vec};

use goethite_macros::lazy;

#[lazy(rwlock)]
pub static DRIVERS: Vec<Box<dyn Driver>> = Vec::new();

pub trait Driver {}
