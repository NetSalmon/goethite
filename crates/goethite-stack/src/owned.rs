//! goethite-stack 的分配版（alloc）模型：拥有所有数据的 [`Stack`]。
//!
//! 该模块仅在启用 `alloc` feature 时编译。相较于零拷贝的 [`StackRef`]，这里把
//! argv / env / auxv 还原为拥有所有权的集合：
//! - 键值对（env、auxv）使用 [`BTreeMap`]；
//! - 字符串数组（argv）使用 [`Vec<String>`]。
//!
//! 提供 [`Stack::parse`]（分配反序列化）与 [`Stack::to_bytes`]（分配序列化，返回
//! `Vec<u8>`），也提供 [`Stack::serialized_len`] 用于预分配。

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::{
    error::Error,
    ser::{self, Input},
    view::StackRef,
};

/// 栈映像的拥有数据模型。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stack {
    /// 命令行参数（`argv`）。
    pub argv: Vec<String>,
    /// 环境变量（`env`），键值对。重复键会被折叠（后写覆盖先写）。
    pub env: BTreeMap<String, String>,
    /// 辅助向量（`auxv`），键值对。
    pub auxv: BTreeMap<String, String>,
}

/// 从 [`Stack`] 借用收集出 [`Input`] 所需的切片，并持有这些借用，供整个序列化过程使用。
struct InputRef<'a> {
    argv: Vec<&'a str>,
    env: Vec<(&'a str, &'a str)>,
    auxv: Vec<(&'a str, &'a str)>,
}

impl<'a> InputRef<'a> {
    fn new(stack: &'a Stack) -> Self {
        Self {
            argv: stack.argv.iter().map(String::as_str).collect(),
            env: stack
                .env
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            auxv: stack
                .auxv
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
        }
    }

    fn as_input(&self) -> Input<'_> {
        Input {
            argv: &self.argv,
            env: &self.env,
            auxv: &self.auxv,
        }
    }
}

impl Stack {
    /// 构造一个空的 [`Stack`]。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由各部分构造。
    pub fn with_parts(
        argv: Vec<String>,
        env: BTreeMap<String, String>,
        auxv: BTreeMap<String, String>,
    ) -> Self {
        Self { argv, env, auxv }
    }

    /// 从栈映像字节解析出拥有数据的 [`Stack`]。
    ///
    /// 该方法先做零拷贝视图解析，再把每个字符串拷贝（`to_string`）到拥有集合中。
    /// 池中若含非法 UTF-8，会返回 [`Error::NotUtf8`] 而不是 panic。
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let view = StackRef::parse(bytes)?;
        let mut argv = Vec::with_capacity(view.argc());
        for entry in view.try_argv() {
            argv.push(entry?.to_string());
        }

        let mut env = BTreeMap::new();
        for entry in view.try_env() {
            let (k, v) = entry?;
            env.insert(k.to_string(), v.to_string());
        }

        let mut auxv = BTreeMap::new();
        for entry in view.try_auxv() {
            let (k, v) = entry?;
            auxv.insert(k.to_string(), v.to_string());
        }

        Ok(Stack { argv, env, auxv })
    }

    /// 序列化所需的字节数。
    ///
    /// 与 [`Stack::to_bytes`] 产出的 `Vec` 长度一致。
    pub fn serialized_len(&self) -> usize {
        let input_ref = InputRef::new(self);
        let input = input_ref.as_input();
        ser::required_len(&input)
    }

    /// 序列化为拥有所有权的字节缓冲。
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let input_ref = InputRef::new(self);
        let input = input_ref.as_input();
        let len = ser::required_len(&input);
        let mut buf = vec![0u8; len];
        let n = ser::write(&mut buf, &input)?;
        buf.truncate(n);
        Ok(buf)
    }
}

impl From<&StackRef<'_>> for Stack {
    /// 从零拷贝视图构造拥有数据的 [`Stack`]。
    ///
    /// 与 [`Stack::parse`] 不同，此转换假定视图池内为合法 UTF-8；非法时在迭代处 panic。
    fn from(view: &StackRef<'_>) -> Self {
        Stack {
            argv: view.argv().map(str::to_string).collect(),
            env: view
                .env()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            auxv: view
                .auxv()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}
