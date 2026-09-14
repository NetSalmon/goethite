//! [`bits`](macro@bits) 过程宏的实现。
//!
//! 解析输入的位域类型定义，生成一个 `#[repr(transparent)]` 的包装结构体、
//! 各比特字段的 `getter` / `setter`、一个消费型 `Builder`、`Debug` 输出，
//! 以及 `From` / `Deref` / 位运算 / 派生 trait 等实现。字段支持三种形态：
//! [`FieldKind::Bit`]（单个比特）、[`FieldKind::Bits`]（比特区间）、
//! [`FieldKind::TypedBits`]（带类型转换的比特区间）。

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Ident, LitInt, Result, Token, Type, Visibility,
    parse::{Parse, ParseStream},
};

/// 展开 [`bits`](macro@bits) 宏：解析输入并生成输出 TokenStream。
pub(crate) fn expand(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let expanded = match syn::parse2::<Input>(input.into()) {
        Ok(input) => input.expand(),
        Err(err) => err.to_compile_error(),
    };
    expanded.into()
}

/// 一次 `bits` 调用的解析结果：可见性、类型名、底层整数类型与字段列表。
struct Input {
    vis: Visibility,
    name: Ident,
    ty: Type,
    fields: Vec<Field>,
}

/// 单个位域字段的声明：字段名与字段形态。
struct Field {
    name: Ident,
    kind: FieldKind,
}

/// 位域字段形态。
enum FieldKind {
    /// 单比特位：`name: index`，`getter` 返回 `bool`。
    Bit { from: LitInt },
    /// 连续比特区间：`name: from => to`，`getter` 返回底层整数类型。
    Bits { from: LitInt, to: LitInt },
    /// 带类型转换的比特区间：`name: Type : from => to`，`getter`/`setter` 使用 `Type`。
    TypedBits {
        ty: Box<Type>,
        from: LitInt,
        to: LitInt,
    },
}

impl Parse for Input {
    /// 解析 `vis type Name(u8) { FieldName[: [Type :]] BitRange, ... }`。
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let vis: Visibility = input.parse()?;
        input.parse::<Token![type]>()?;
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty: Type = input.parse()?;

        let content;
        syn::braced!(content in input);

        let mut fields = Vec::new();
        while !content.is_empty() {
            let name: Ident = content.parse()?;
            content.parse::<Token![:]>()?;

            // `name: 2 => 4` / `name: 2`（比特位是字面量）；
            // `name: Type : 5 => 6`（类型转换字段，类型不能以字面量开头）。
            let kind = if content.peek(LitInt) {
                let from: LitInt = content.parse()?;
                if content.peek(Token![=>]) {
                    content.parse::<Token![=>]>()?;
                    let to: LitInt = content.parse()?;
                    FieldKind::Bits { from, to }
                } else {
                    FieldKind::Bit { from }
                }
            } else {
                let ty: Type = content.parse()?;
                content.parse::<Token![:]>()?;
                let from: LitInt = content.parse()?;
                content.parse::<Token![=>]>()?;
                let to: LitInt = content.parse()?;
                FieldKind::TypedBits {
                    ty: Box::new(ty),
                    from,
                    to,
                }
            };

            fields.push(Field { name, kind });

            if !content.peek(Token![,]) {
                break;
            }
            content.parse::<Token![,]>()?;
        }

        if !content.is_empty() {
            return Err(content.error("expected `,`"));
        }

        Ok(Input {
            vis,
            name,
            ty,
            fields,
        })
    }
}

/// `Field` 的代码生成辅助：生成 getter / setter / builder setter / debug 片段。
impl Field {
    /// 生成该字段的 `getter` 方法（`pub fn name(&self) -> ...`）。
    fn getter(&self, ty: &Type) -> TokenStream {
        let name = &self.name;
        match &self.kind {
            FieldKind::Bit { from } => quote! {
                #[inline(always)]
                pub fn #name(&self) -> bool {
                    const MASK: #ty = 1 << #from;
                    (self.0 & MASK) != 0
                }
            },
            FieldKind::Bits { from, to } => quote! {
                #[inline(always)]
                pub fn #name(&self) -> #ty {
                    const MASK: #ty = ((1 << (#to - #from + 1)) - 1) << #from;
                    (self.0 & MASK) >> #from
                }
            },
            FieldKind::TypedBits {
                ty: field_ty,
                from,
                to,
            } => quote! {
                #[inline(always)]
                pub fn #name(&self) -> #field_ty {
                    const MASK: #ty = ((1 << (#to - #from + 1)) - 1) << #from;
                    ((self.0 & MASK) >> #from).into()
                }
            },
        }
    }

    /// 生成该字段的 `setter` 方法（`pub fn set_name(&mut self, value: ...)`）。
    fn setter(&self, ty: &Type) -> TokenStream {
        let set = format_ident!("set_{}", self.name);
        match &self.kind {
            FieldKind::Bit { from } => quote! {
                #[inline(always)]
                pub fn #set(&mut self, value: bool) {
                    const CLR_MASK: #ty = !(1 << #from);
                    self.0 = (self.0 & CLR_MASK) | ((value as #ty) << #from);
                }
            },
            FieldKind::Bits { from, to } => quote! {
                #[inline(always)]
                pub fn #set(&mut self, value: #ty) {
                    const CLR_MASK: #ty = !(((1 << (#to - #from + 1)) - 1) << #from);
                    self.0 = (self.0 & CLR_MASK) | ((value & ((1 << (#to - #from + 1)) - 1)) << #from);
                }
            },
            FieldKind::TypedBits {
                ty: field_ty,
                from,
                to,
            } => quote! {
                #[inline(always)]
                pub fn #set(&mut self, value: #field_ty) {
                    const CLR_MASK: #ty = !(((1 << (#to - #from + 1)) - 1) << #from);
                    let value: #ty = value.into();
                    self.0 = (self.0 & CLR_MASK) | ((value & ((1 << (#to - #from + 1)) - 1)) << #from);
                }
            },
        }
    }

    /// 生成 `Builder` 上的 `set_name(mut self, value) -> Self` 消费型 setter。
    fn builder_setter(&self, ty: &Type) -> TokenStream {
        let set = format_ident!("set_{}", self.name);
        match &self.kind {
            FieldKind::Bit { from } => quote! {
                #[inline(always)]
                pub fn #set(mut self, value: bool) -> Self {
                    const CLR_MASK: #ty = !(1 << #from);
                    self.0 = (self.0 & CLR_MASK) | ((value as #ty) << #from);
                    self
                }
            },
            FieldKind::Bits { from, to } => quote! {
                #[inline(always)]
                pub fn #set(mut self, value: #ty) -> Self {
                    const CLR_MASK: #ty = !(((1 << (#to - #from + 1)) - 1) << #from);
                    self.0 = (self.0 & CLR_MASK) | ((value & ((1 << (#to - #from + 1)) - 1)) << #from);
                    self
                }
            },
            FieldKind::TypedBits {
                ty: field_ty,
                from,
                to,
            } => quote! {
                #[inline(always)]
                pub fn #set(mut self, value: #field_ty) -> Self {
                    const CLR_MASK: #ty = !(((1 << (#to - #from + 1)) - 1) << #from);
                    let value: #ty = value.into();
                    self.0 = (self.0 & CLR_MASK) | ((value & ((1 << (#to - #from + 1)) - 1)) << #from);
                    self
                }
            },
        }
    }

    /// 生成 `Debug` 输出中该字段的片段（`write!(f, ", name: {:?}", self.name())`）。
    fn debug_write(&self) -> TokenStream {
        let name = &self.name;
        let name_str = self.name.to_string();
        quote! {
            write!(f, ", {}: {:?}", #name_str, self.#name())?;
        }
    }
}

/// 生成位运算实现：`#name` 与底层整数 `#ty` 之间的 `&` / `|` / `^` / `<<` / `>>`，
/// 结果仍为 `#name`。
fn bitwise_impls(name: &Ident, ty: &Type) -> TokenStream {
    quote! {
        impl core::ops::BitAnd<#ty> for #name {
            type Output = #name;

            fn bitand(self, rhs: #ty) -> Self::Output {
                #name(self.0 & rhs)
            }
        }

        impl core::ops::BitOr<#ty> for #name {
            type Output = #name;

            fn bitor(self, rhs: #ty) -> Self::Output {
                #name(self.0 | rhs)
            }
        }

        impl core::ops::BitXor<#ty> for #name {
            type Output = #name;

            fn bitxor(self, rhs: #ty) -> Self::Output {
                #name(self.0 ^ rhs)
            }
        }

        impl core::ops::Shl<#ty> for #name {
            type Output = #name;

            fn shl(self, rhs: #ty) -> Self::Output {
                #name(self.0 << rhs)
            }
        }

        impl core::ops::Shr<#ty> for #name {
            type Output = #name;

            fn shr(self, rhs: #ty) -> Self::Output {
                #name(self.0 >> rhs)
            }
        }
    }
}

/// 展开位域定义，生成完整结构体、`Builder` 与 trait 实现。
impl Input {
    /// 生成位域结构体、`Builder`、`Debug`、`From` / `Deref` 等实现的展开结果。
    fn expand(&self) -> TokenStream {
        let Input {
            vis,
            name,
            ty,
            fields,
        } = self;
        let builder = format_ident!("{}Builder", name);
        let name_str = name.to_string();

        let getters = fields.iter().map(|f| f.getter(ty));
        let setters = fields.iter().map(|f| f.setter(ty));
        let builder_setters = fields.iter().map(|f| f.builder_setter(ty));
        let debug_fields = fields.iter().map(|f| f.debug_write());
        let bitwise = bitwise_impls(name, ty);

        quote! {
            #[repr(transparent)]
            #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
            #vis struct #name(#ty);

            impl #name {
                #[inline(always)]
                pub const fn from(value: #ty) -> Self {
                    Self(value)
                }

                #[inline(always)]
                pub const fn new() -> Self {
                    Self(0)
                }

                #[inline(always)]
                pub const fn builder() -> #builder {
                    #builder::new()
                }

                #(#getters)*
                #(#setters)*
            }

            impl core::fmt::Debug for #name {
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    write!(f, "{} {{ raw: {:#x}", #name_str, self.0)?;
                    #(#debug_fields)*
                    write!(f, " }}")
                }
            }

            #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
            #vis struct #builder(#ty);

            impl #builder {
                #[inline(always)]
                pub const fn new() -> Self {
                    Self(0)
                }

                #(#builder_setters)*

                #[inline(always)]
                pub const fn build(self) -> #name {
                    #name::from(self.0)
                }
            }

            impl core::convert::From<#ty> for #name {
                fn from(value: #ty) -> Self {
                    Self(value)
                }
            }

            impl core::convert::From<#name> for #ty {
                fn from(value: #name) -> Self {
                    value.0
                }
            }

            impl core::ops::Deref for #name {
                type Target = #ty;

                fn deref(&self) -> &Self::Target {
                    &self.0
                }
            }

            #bitwise
        }
    }
}
