//! [`numeric`](macro@numeric) 过程宏的实现。
//!
//! 解析输入的整数枚举定义，生成一个 `#[repr(transparent)]` 的整数 newtype：
//! 为常量项产生关联常量，为区间项产生 `is_<name>` 谓词函数，并实现 `Debug`
//! 与 `From` 双向转换。带 `with ops` 语法时还会生成一组 `Add` / `Sub` / `Mul` /
//! `Div` 算术实现。

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Expr, Ident, Result, Token, Type, Visibility, custom_keyword,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

custom_keyword!(with);
custom_keyword!(ops);

/// 展开 [`numeric`](macro@numeric) 宏：解析输入并生成输出 TokenStream。
pub(crate) fn expand(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let expanded = match syn::parse2::<Input>(input.into()) {
        Ok(input) => input.expand(),
        Err(err) => err.to_compile_error(),
    };
    expanded.into()
}

/// 一次 `numeric` 调用的解析结果：可见性、类型名、是否启用 `with ops` 算术、
/// 底层整数类型与条目列表。
struct Input {
    vis: Visibility,
    name: Ident,
    with_ops: bool,
    ty: Type,
    entries: Vec<Entry>,
}

/// 单个枚举条目：具名常量或具名取值区间。
enum Entry {
    /// 具名常量：`name = value`。
    Const { name: Ident, value: Expr },
    /// 具名取值区间：`name = from => to`。
    Range { name: Ident, from: Expr, to: Expr },
}

impl Parse for Input {
    /// 解析 `vis enum Name [with ops]: ty { Entry, ... }`。
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let vis: Visibility = input.parse()?;
        input.parse::<Token![enum]>()?;
        let name: Ident = input.parse()?;
        let with_ops = input.peek(with);
        if with_ops {
            input.parse::<with>()?;
            input.parse::<ops>()?;
        }
        input.parse::<Token![:]>()?;
        let ty: Type = input.parse()?;

        let content;
        syn::braced!(content in input);
        let entries = Punctuated::<Entry, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();

        Ok(Input {
            vis,
            name,
            with_ops,
            ty,
            entries,
        })
    }
}

impl Parse for Entry {
    /// 解析条目：`name = value`（常量）或 `name = from => to`（区间）。
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let from: Expr = input.parse()?;
        if input.peek(Token![=>]) {
            input.parse::<Token![=>]>()?;
            let to: Expr = input.parse()?;
            Ok(Entry::Range { name, from, to })
        } else {
            Ok(Entry::Const { name, value: from })
        }
    }
}

/// 展开整数枚举定义，生成整数 newtype、常量/谓词与 trait 实现及可选算术。
impl Input {
    /// 生成整数 newtype、关联常量与谓词函数、`Debug`、`From` 转换及可选算术的实现。
    fn expand(&self) -> TokenStream {
        let Input {
            vis,
            name,
            with_ops,
            ty,
            entries,
        } = self;
        let name_str = name.to_string();

        // 常量项 -> 关联常量；区间项 -> `is_<lower>` 谓词函数。
        let consts = entries.iter().filter_map(|entry| match entry {
            Entry::Const { name, value } => Some(quote! {
                #vis const #name : Self = Self(#value);
            }),
            Entry::Range { .. } => None,
        });
        let predicates = entries.iter().filter_map(|entry| match entry {
            Entry::Range { name, from, to } => {
                let is_name = format_ident!("is_{}", name.to_string().to_lowercase());
                Some(quote! {
                    #[inline]
                    #[allow(unused_comparisons)]
                    #vis const fn #is_name(value: #ty) -> bool {
                        value >= #from && value <= #to
                    }
                })
            }
            Entry::Const { .. } => None,
        });

        // Debug：常量项优先，区间项按声明顺序，最后回退到 `unknown`。
        let const_arms = entries.iter().filter_map(|entry| match entry {
            Entry::Const { name, .. } => {
                let full = format!("{}::{}", name_str, name);
                Some(quote! { Self::#name => f.write_str(#full), })
            }
            Entry::Range { .. } => None,
        });
        let range_arms = entries.iter().filter_map(|entry| match entry {
            Entry::Range { name, from, to } => {
                let full = format!("{}::{}", name_str, name);
                Some(quote! { x if x.0 >= #from && x.0 <= #to => f.write_str(#full), })
            }
            Entry::Const { .. } => None,
        });

        let ops = if *with_ops {
            Some(ops_impls(name, ty))
        } else {
            None
        };

        quote! {
            #[repr(transparent)]
            #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
            #vis struct #name(pub #ty);

            impl #name {
                #(#consts)*
                #(#predicates)*
            }

            #[allow(unused)]
            impl core::fmt::Debug for #name {
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    match *self {
                        #(#const_arms)*
                        #(#range_arms)*
                        _ => f.write_str("unknown"),
                    }
                }
            }

            impl core::convert::From<#ty> for #name {
                fn from(value: #ty) -> Self {
                    #name(value)
                }
            }

            impl core::convert::From<#name> for #ty {
                fn from(value: #name) -> Self {
                    value.0
                }
            }

            #ops
        }
    }
}

/// 生成 `with ops` 时的一组 `Add` / `Sub` / `Mul` / `Div` 算术实现。
///
/// 覆盖 `#name` 与 `#ty` 的组合形态：同类型加减、newtype 与底层整数类型的
/// 混合算术（结果类型为 `#ty` 或 `#name` 视操作数而定）。
fn ops_impls(name: &Ident, ty: &Type) -> TokenStream {
    quote! {
        impl core::ops::Add for #name {
            type Output = #name;

            fn add(self, rhs: Self) -> Self::Output {
                #name(self.0 + rhs.0)
            }
        }

        impl core::ops::Sub for #name {
            type Output = #name;

            fn sub(self, rhs: Self) -> Self::Output {
                #name(self.0 - rhs.0)
            }
        }

        impl core::ops::Add<#ty> for #name {
            type Output = #ty;

            fn add(self, rhs: #ty) -> Self::Output {
                self.0 + rhs
            }
        }

        impl core::ops::Add<#name> for #ty {
            type Output = #ty;

            fn add(self, rhs: #name) -> Self::Output {
                self + rhs.0
            }
        }

        impl core::ops::Sub<#ty> for #name {
            type Output = #ty;

            fn sub(self, rhs: #ty) -> Self::Output {
                self.0 - rhs
            }
        }

        impl core::ops::Sub<#name> for #ty {
            type Output = #ty;

            fn sub(self, rhs: #name) -> Self::Output {
                self - rhs.0
            }
        }

        impl core::ops::Mul<#ty> for #name {
            type Output = #ty;

            fn mul(self, rhs: #ty) -> Self::Output {
                self.0 * rhs
            }
        }

        impl core::ops::Mul<#name> for #ty {
            type Output = #ty;

            fn mul(self, rhs: #name) -> Self::Output {
                self * rhs.0
            }
        }

        impl core::ops::Div<#ty> for #name {
            type Output = #ty;

            fn div(self, rhs: #ty) -> Self::Output {
                self.0 / rhs
            }
        }

        impl core::ops::Div<#name> for #ty {
            type Output = #ty;

            fn div(self, rhs: #name) -> Self::Output {
                self / rhs.0
            }
        }
    }
}
