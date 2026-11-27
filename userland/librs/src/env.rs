//! Rust 原生用户态环境变量管理。
//!
//! 初始栈上的 `goethite-stack` 映像是只读的；本模块在进程首次访问可变环境表时，
//! 懒加载地将初始环境变量解析为 [`BTreeMap<String, String>`]，之后支持 O(log n)
//! 的查询、新增、修改与删除，并在需要时直接以 `&[(&str, &str)]` 传递给 `execve`。

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use goethite_stack::StackRef;

use crate::sync::UserSpinLock;

/// 可变环境变量存储（纯逻辑结构，便于宿主机单元测试）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvStore {
    vars: BTreeMap<String, String>,
}

impl EnvStore {
    /// 创建一个空的环境表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从零拷贝栈映像视图初始化环境表（若存在重复键，后者覆盖前者）。
    pub fn from_stack(stack: &StackRef<'_>) -> Self {
        let mut vars = BTreeMap::new();
        for (k, v) in stack.env() {
            vars.insert(k.to_string(), v.to_string());
        }
        Self { vars }
    }

    /// 获取环境变量值。
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    /// 设置或新增环境变量，返回旧值（若有）。
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> Option<String> {
        self.vars.insert(key.into(), value.into())
    }

    /// 删除环境变量，返回旧值（若有）。
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.vars.remove(key)
    }

    /// 是否包含指定键。
    pub fn contains(&self, key: &str) -> bool {
        self.vars.contains_key(key)
    }

    /// 清空全部环境变量。
    pub fn clear(&mut self) {
        self.vars.clear();
    }

    /// 当前环境变量条数。
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// 只读借用底层有序映射。
    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    /// 导出为 `(String, String)` 列表。
    pub fn to_vec(&self) -> Vec<(String, String)> {
        self.vars
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

static GLOBAL_ENV: UserSpinLock<Option<EnvStore>> = UserSpinLock::new(None);

fn ensure_loaded<'a>(slot: &'a mut Option<EnvStore>) -> &'a mut EnvStore {
    slot.get_or_insert_with(|| match crate::stack() {
        Some(view) => EnvStore::from_stack(&view),
        None => EnvStore::new(),
    })
}

/// 重新从当前 `STACK_IMAGE` 加载全局环境变量（主要用于测试或重置状态）。
pub fn reload_from_stack() {
    let mut guard = GLOBAL_ENV.lock();
    *guard = Some(match crate::stack() {
        Some(view) => EnvStore::from_stack(&view),
        None => EnvStore::new(),
    });
}

/// 读取环境变量的值（返回拥有所有权的 `String`）。
pub fn get(key: &str) -> Option<String> {
    let mut guard = GLOBAL_ENV.lock();
    ensure_loaded(&mut guard).get(key).map(str::to_string)
}

/// 设置或新增环境变量。
pub fn set(key: impl Into<String>, value: impl Into<String>) {
    let mut guard = GLOBAL_ENV.lock();
    ensure_loaded(&mut guard).set(key, value);
}

/// 删除环境变量，返回旧值（若有）。
pub fn remove(key: &str) -> Option<String> {
    let mut guard = GLOBAL_ENV.lock();
    ensure_loaded(&mut guard).remove(key)
}

/// 判断是否存在某个环境变量。
pub fn contains(key: &str) -> bool {
    let mut guard = GLOBAL_ENV.lock();
    ensure_loaded(&mut guard).contains(key)
}

/// 清空当前进程的环境变量表。
pub fn clear() {
    let mut guard = GLOBAL_ENV.lock();
    ensure_loaded(&mut guard).clear();
}

/// 获取当前所有环境变量的快照列表（按键字典序排列）。
pub fn vars() -> Vec<(String, String)> {
    let mut guard = GLOBAL_ENV.lock();
    ensure_loaded(&mut guard).to_vec()
}

/// 在持有锁期间以只读方式访问底层 `BTreeMap`。
pub fn with_vars<R>(f: impl FnOnce(&BTreeMap<String, String>) -> R) -> R {
    let mut guard = GLOBAL_ENV.lock();
    let store = ensure_loaded(&mut guard);
    f(store.as_map())
}

/// 以当前进程的可变环境变量表执行 `execve`（结构化 `&[(&str, &str)]`，无 `"K=V"` 拼接开销）。
#[cfg(target_arch = "riscv64")]
pub fn execve(path: &str, args: &[&str]) -> goethite_abi::SysResult<i32> {
    use goethite_abi::SyscallOps;
    let snapshot = vars();
    let pairs: Vec<(&str, &str)> = snapshot
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    goethite_syscall::Syscalls::execve(path, args, &pairs)
}
