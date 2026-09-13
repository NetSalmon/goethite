//! goethite-stack 的零拷贝（zero-copy / 无分配）反序列化视图。
//!
//! [`StackRef`] 借用一片字节缓冲（栈映像），在不拷贝任何字符串数据的前提下暴露
//! `argv` / `env` / `auxv` 的迭代视图。字符串数据直接引用缓冲内的字符串池字节；
//! 头部字段以小端序解码，各表区域在对齐校验通过后作为类型化切片借出，从而避免
//! 对齐相关的未定义行为。
//!
//! 该视图的生命周期 `'a` 与输入字节缓冲绑定，不会被遮蔽。

use crate::{
    error::Error,
    raw::{
        HEADER_SIZE, Header, PAIR_SIZE, Pair, STRING_ID_SIZE, STRING_REF_SIZE, StringId, StringRef,
        le32,
    },
};

/// 借用缓冲的零拷贝栈映像视图。
#[derive(Clone, Copy, Debug)]
pub struct StackRef<'a> {
    header: Header,
    string_table: &'a [StringRef],
    envp: &'a [Pair],
    auxv: &'a [Pair],
    argv: &'a [StringId],
    pool: &'a [u8],
}

/// 解析一个字节缓冲为 [`StackRef`]。
///
/// 该函数只做结构（头部、各表偏移、长度、对齐）校验并把表区域借出，不会把字符串拷入
/// 堆内存。字符串池中若存在非法 UTF-8，会在经由 [`StackRef::string`] 等访问器时暴露。
///
/// # 对齐
///
/// 为安全地借出类型化表切片，要求缓冲基址加上各表偏移后分别按 `StringRef` / `Pair` /
/// `StringId`（均为 4 字节）对齐；否则返回 [`Error::Misaligned`]。由本 crate 序列化器
/// 产出的缓冲偏移均为 4 的倍数，因此只要缓冲基址 4 字节对齐即可通过。
impl<'a> StackRef<'a> {
    /// 从字节缓冲解析出零拷贝视图。
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_SIZE {
            return Err(Error::Truncated { region: "header" });
        }
        if bytes[0..8] != crate::raw::GOETHITE_STACK_MAGIC {
            return Err(Error::BadMagic);
        }

        let total_size = le32(bytes, 8) as usize;
        let index_offset = le32(bytes, 12) as usize;
        let envp_offset = le32(bytes, 16) as usize;
        let auxv_offset = le32(bytes, 20) as usize;
        let argv_offset = le32(bytes, 24) as usize;
        let pool_offset = le32(bytes, 28) as usize;

        if total_size > bytes.len() {
            return Err(Error::Truncated { region: "total" });
        }
        if index_offset < HEADER_SIZE {
            return Err(Error::InvalidLayout);
        }
        // 各偏移必须单调递增且落在 [HEADER_SIZE, total_size] 内
        let offsets = [
            index_offset,
            envp_offset,
            auxv_offset,
            argv_offset,
            pool_offset,
        ];
        for w in offsets.windows(2) {
            if w[0] > w[1] {
                return Err(Error::InvalidLayout);
            }
        }
        if pool_offset > total_size {
            return Err(Error::InvalidLayout);
        }

        let strtable_bytes = &bytes[index_offset..envp_offset];
        let envp_bytes = &bytes[envp_offset..auxv_offset];
        let auxv_bytes = &bytes[auxv_offset..argv_offset];
        let argv_bytes = &bytes[argv_offset..pool_offset];
        let pool = &bytes[pool_offset..total_size];

        if strtable_bytes.len() % STRING_REF_SIZE != 0 {
            return Err(Error::Misaligned {
                region: "string_table",
            });
        }
        if envp_bytes.len() % PAIR_SIZE != 0 {
            return Err(Error::Misaligned { region: "envp" });
        }
        if auxv_bytes.len() % PAIR_SIZE != 0 {
            return Err(Error::Misaligned { region: "auxv" });
        }
        if argv_bytes.len() % STRING_ID_SIZE != 0 {
            return Err(Error::Misaligned { region: "argv" });
        }

        // 需要把表区域按元素对齐后才能安全地作为切片借出
        if !is_aligned(bytes.as_ptr(), index_offset, STRING_REF_SIZE) {
            return Err(Error::Misaligned {
                region: "string_table",
            });
        }
        if !is_aligned(bytes.as_ptr(), envp_offset, PAIR_SIZE) {
            return Err(Error::Misaligned { region: "envp" });
        }
        if !is_aligned(bytes.as_ptr(), auxv_offset, PAIR_SIZE) {
            return Err(Error::Misaligned { region: "auxv" });
        }
        if !is_aligned(bytes.as_ptr(), argv_offset, STRING_ID_SIZE) {
            return Err(Error::Misaligned { region: "argv" });
        }

        let string_table = unsafe {
            core::slice::from_raw_parts(
                bytes.as_ptr().add(index_offset) as *const StringRef,
                strtable_bytes.len() / STRING_REF_SIZE,
            )
        };
        let envp = unsafe {
            core::slice::from_raw_parts(
                bytes.as_ptr().add(envp_offset) as *const Pair,
                envp_bytes.len() / PAIR_SIZE,
            )
        };
        let auxv = unsafe {
            core::slice::from_raw_parts(
                bytes.as_ptr().add(auxv_offset) as *const Pair,
                auxv_bytes.len() / PAIR_SIZE,
            )
        };
        let argv = unsafe {
            core::slice::from_raw_parts(
                bytes.as_ptr().add(argv_offset) as *const StringId,
                argv_bytes.len() / STRING_ID_SIZE,
            )
        };

        let header = Header {
            magic: crate::raw::GOETHITE_STACK_MAGIC,
            total_size: total_size as u32,
            index_offset: index_offset as u32,
            envp_offset: envp_offset as u32,
            auxv_offset: auxv_offset as u32,
            argv_offset: argv_offset as u32,
            pool_offset: pool_offset as u32,
        };

        Ok(StackRef {
            header,
            string_table,
            envp,
            auxv,
            argv,
            pool,
        })
    }

    /// 头部。
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// 字符串引用表（按 [`StringId`] 索引）。
    pub fn string_table(&self) -> &'a [StringRef] {
        self.string_table
    }

    /// 环境变量表。
    pub fn envp(&self) -> &'a [Pair] {
        self.envp
    }

    /// 辅助向量表。
    pub fn auxv_table(&self) -> &'a [Pair] {
        self.auxv
    }

    /// 参数表（`StringId` 列表）。
    pub fn argv_refs(&self) -> &'a [StringId] {
        self.argv
    }

    /// 字符串池（原始 UTF-8 字节）。
    pub fn pool(&self) -> &'a [u8] {
        self.pool
    }

    /// 参数个数（`argc`）。
    pub fn argc(&self) -> usize {
        self.argv.len()
    }

    /// 环境变量条数。
    pub fn env_count(&self) -> usize {
        self.envp.len()
    }

    /// 辅助向量条数。
    pub fn auxv_count(&self) -> usize {
        self.auxv.len()
    }

    /// 由 [`StringId`] 解析出池内字符串（假定池内为合法 UTF-8）。
    ///
    /// `id` 越界或池内字节非法 UTF-8 时会 panic（用于可信输入）。如需安全变体，见
    /// [`StackRef::try_string`]。
    pub fn string(&self, id: StringId) -> &'a str {
        lookup_str(self.string_table, self.pool, id)
    }

    /// 由 [`StringId`] 解析出池内字符串的安全变体。
    pub fn try_string(&self, id: StringId) -> Result<&'a str, Error> {
        try_lookup(self.string_table, self.pool, id)
    }

    /// 迭代 `argv`，产出 `&str`。
    pub fn argv(&self) -> ArgsIter<'a> {
        let argv: &'a [StringId] = self.argv;
        let table: &'a [StringRef] = self.string_table;
        let pool: &'a [u8] = self.pool;
        ArgsIter {
            inner: argv.iter(),
            table,
            pool,
        }
    }

    /// 迭代 `env`，产出 `(&str, &str)`。
    pub fn env(&self) -> EnvIter<'a> {
        let envp: &'a [Pair] = self.envp;
        let table: &'a [StringRef] = self.string_table;
        let pool: &'a [u8] = self.pool;
        EnvIter {
            inner: envp.iter(),
            table,
            pool,
        }
    }

    /// 迭代 `auxv`，产出 `(&str, &str)`。
    pub fn auxv(&self) -> AuxvIter<'a> {
        let auxv: &'a [Pair] = self.auxv;
        let table: &'a [StringRef] = self.string_table;
        let pool: &'a [u8] = self.pool;
        AuxvIter {
            inner: auxv.iter(),
            table,
            pool,
        }
    }

    /// 迭代 `argv`，产出 `Result<&str, Error>`（安全变体）。
    pub fn try_argv(&self) -> TryArgsIter<'a> {
        let argv: &'a [StringId] = self.argv;
        let table: &'a [StringRef] = self.string_table;
        let pool: &'a [u8] = self.pool;
        TryArgsIter {
            inner: argv.iter(),
            table,
            pool,
        }
    }

    /// 迭代 `env`，产出 `Result<(&str, &str), Error>`（安全变体）。
    pub fn try_env(&self) -> TryEnvIter<'a> {
        let envp: &'a [Pair] = self.envp;
        let table: &'a [StringRef] = self.string_table;
        let pool: &'a [u8] = self.pool;
        TryEnvIter {
            inner: envp.iter(),
            table,
            pool,
        }
    }

    /// 迭代 `auxv`，产出 `Result<(&str, &str), Error>`（安全变体）。
    pub fn try_auxv(&self) -> TryAuxvIter<'a> {
        let auxv: &'a [Pair] = self.auxv;
        let table: &'a [StringRef] = self.string_table;
        let pool: &'a [u8] = self.pool;
        TryAuxvIter {
            inner: auxv.iter(),
            table,
            pool,
        }
    }
}

/// 判断 `base + off` 是否按 `align` 对齐。
#[inline]
fn is_aligned(base: *const u8, off: usize, align: usize) -> bool {
    (base as usize + off) % align == 0
}

/// 按表 + 池解析 `id` 为 `&str`（假定合法 UTF-8，越界或非法时 panic）。
fn lookup_str<'a>(table: &'a [StringRef], pool: &'a [u8], id: StringId) -> &'a str {
    try_lookup(table, pool, id).expect("goethite-stack: invalid string reference")
}

/// 按表 + 池解析 `id` 为 `&str` 的安全变体。
fn try_lookup<'a>(table: &'a [StringRef], pool: &'a [u8], id: StringId) -> Result<&'a str, Error> {
    let r = table.get(id as usize).ok_or(Error::InvalidLayout)?;
    let start = r.offset as usize;
    let end = start + r.len as usize;
    if end > pool.len() {
        return Err(Error::Truncated { region: "pool" });
    }
    core::str::from_utf8(&pool[start..end]).map_err(|_| Error::NotUtf8 { offset: start })
}

/// `argv` 的迭代器，产出 `&'a str`。
#[derive(Clone, Debug)]
pub struct ArgsIter<'a> {
    inner: core::slice::Iter<'a, StringId>,
    table: &'a [StringRef],
    pool: &'a [u8],
}

/// `env` 的迭代器，产出 `(&'a str, &'a str)`。
#[derive(Clone, Debug)]
pub struct EnvIter<'a> {
    inner: core::slice::Iter<'a, Pair>,
    table: &'a [StringRef],
    pool: &'a [u8],
}

/// `auxv` 的迭代器，产出 `(&'a str, &'a str)`。
#[derive(Clone, Debug)]
pub struct AuxvIter<'a> {
    inner: core::slice::Iter<'a, Pair>,
    table: &'a [StringRef],
    pool: &'a [u8],
}

/// `argv` 的安全迭代器，产出 `Result<&'a str, Error>`。
#[derive(Clone, Debug)]
pub struct TryArgsIter<'a> {
    inner: core::slice::Iter<'a, StringId>,
    table: &'a [StringRef],
    pool: &'a [u8],
}

/// `env` 的安全迭代器，产出 `Result<(&'a str, &'a str), Error>`。
#[derive(Clone, Debug)]
pub struct TryEnvIter<'a> {
    inner: core::slice::Iter<'a, Pair>,
    table: &'a [StringRef],
    pool: &'a [u8],
}

/// `auxv` 的安全迭代器，产出 `Result<(&'a str, &'a str), Error>`。
#[derive(Clone, Debug)]
pub struct TryAuxvIter<'a> {
    inner: core::slice::Iter<'a, Pair>,
    table: &'a [StringRef],
    pool: &'a [u8],
}

macro_rules! impl_infallible_iter {
    ($name:ident, $item:ty, $resolve:expr) => {
        impl<'a> Iterator for $name<'a> {
            type Item = $item;

            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                self.inner
                    .next()
                    .map(|e| $resolve(e, self.table, self.pool))
            }
        }
        impl<'a> DoubleEndedIterator for $name<'a> {
            #[inline]
            fn next_back(&mut self) -> Option<Self::Item> {
                self.inner
                    .next_back()
                    .map(|e| $resolve(e, self.table, self.pool))
            }
        }
        impl<'a> ExactSizeIterator for $name<'a> {
            fn len(&self) -> usize {
                self.inner.len()
            }
        }
        impl<'a> core::iter::FusedIterator for $name<'a> {}
    };
}

impl_infallible_iter!(ArgsIter, &'a str, |id: &StringId, table, pool| lookup_str(
    table, pool, *id
));
impl_infallible_iter!(EnvIter, (&'a str, &'a str), |p: &Pair, table, pool| {
    (
        lookup_str(table, pool, p.key),
        lookup_str(table, pool, p.value),
    )
});
impl_infallible_iter!(AuxvIter, (&'a str, &'a str), |p: &Pair, table, pool| {
    (
        lookup_str(table, pool, p.key),
        lookup_str(table, pool, p.value),
    )
});

macro_rules! impl_fallible_iter {
    ($name:ident, $item:ty, $resolve:expr) => {
        impl<'a> Iterator for $name<'a> {
            type Item = Result<$item, Error>;

            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                self.inner
                    .next()
                    .map(|e| $resolve(e, self.table, self.pool))
            }
        }
    };
}

impl_fallible_iter!(TryArgsIter, &'a str, |id: &StringId, table, pool| {
    try_lookup(table, pool, *id)
});
impl_fallible_iter!(TryEnvIter, (&'a str, &'a str), |p: &Pair, table, pool| {
    Ok((
        try_lookup(table, pool, p.key)?,
        try_lookup(table, pool, p.value)?,
    ))
});
impl_fallible_iter!(TryAuxvIter, (&'a str, &'a str), |p: &Pair, table, pool| {
    Ok((
        try_lookup(table, pool, p.key)?,
        try_lookup(table, pool, p.value)?,
    ))
});
