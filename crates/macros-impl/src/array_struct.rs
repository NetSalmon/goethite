//! [`array_struct`](macro@array_struct) 过程宏的实现。
//!
//! 解析输入的数组包装结构体定义，生成一个 `#[repr(transparent)]` 的结构体
//! 包装 `[#elem; #len]`，并为每个字段生成 `getter` / `setter` 访问器。
//! 字段支持三种形态：[`FieldKind::Plain`]（直接读写数组元素）、
//! [`FieldKind::Into`]（`.into()` 转换）、[`FieldKind::TryInto`]（`.try_into()` 返回 `Result`）。

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Expr, Ident, Result, Token, Type, Visibility,
    parse::{Parse, ParseStream},
};

use crate::case;

/// 展开 [`array_struct`](macro@array_struct) 宏：解析输入并生成输出 TokenStream。
pub(crate) fn expand(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let expanded = match syn::parse2::<Input>(input.into()) {
        Ok(input) => input.expand(),
        Err(err) => err.to_compile_error(),
    };
    expanded.into()
}

/// 一次 `array_struct` 调用的解析结果：可见性、结构体名、数组元素类型与长度、字段列表。
struct Input {
    vis: Visibility,
    name: Ident,
    elem: Type,
    len: Expr,
    fields: Vec<Field>,
}

/// 单个数组字段的声明：字段名、取值类型转换方式与数组下标。
struct Field {
    name: Ident,
    kind: FieldKind,
    index: Expr,
}

/// 字段取值类型转换方式。
enum FieldKind {
    /// 直接以数组元素类型读写。
    Plain,
    /// 以 `.into()` 转换为目标类型。
    Into(Type),
    /// 以 `.try_into()` 转换，`getter` 返回 `Result`。
    TryInto(Type),
}

impl Parse for Input {
    /// 解析 `vis struct Name([Elem; Len]) { FieldName[: [@try] ToType] => Index, ... }`。
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let vis: Visibility = input.parse()?;
        input.parse::<Token![struct]>()?;
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;

        let bracketed;
        syn::bracketed!(bracketed in input);
        let elem: Type = bracketed.parse()?;
        bracketed.parse::<Token![;]>()?;
        let len: Expr = bracketed.parse()?;

        let content;
        syn::braced!(content in input);

        let mut fields = Vec::new();
        while !content.is_empty() {
            let name: Ident = content.parse()?;

            // `name => index` / `name: ToType => index` / `name: @try ToType => index`。
            let kind = if content.peek(Token![:]) {
                content.parse::<Token![:]>()?;
                if content.peek(Token![@]) {
                    content.parse::<Token![@]>()?;
                    // `try` 是保留字，不能用 Ident 解析。
                    content.parse::<Token![try]>()?;
                    FieldKind::TryInto(content.parse()?)
                } else {
                    FieldKind::Into(content.parse()?)
                }
            } else {
                FieldKind::Plain
            };

            content.parse::<Token![=>]>()?;
            let index: Expr = content.parse()?;

            fields.push(Field { name, kind, index });

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
            elem,
            len,
            fields,
        })
    }
}

/// 展开数组包装结构体定义，生成 `#[repr(transparent)]` 结构体与字段访问器。
impl Input {
    /// 生成包装结构体定义与各字段 `getter` / `setter` 的展开结果。
    fn expand(&self) -> TokenStream {
        let Input {
            vis,
            name,
            elem,
            len,
            fields,
        } = self;

        let methods = fields.iter().map(|Field { name, kind, index }| {
            let getter = case::to_snake(name);
            let setter = format_ident!("set_{}", getter);
            match kind {
                FieldKind::Plain => quote! {
                    pub fn #getter(&self) -> #elem {
                        self.0[#index]
                    }

                    pub fn #setter(&mut self, value: #elem) {
                        self.0[#index] = value;
                    }
                },
                FieldKind::Into(to_ty) => quote! {
                    pub fn #getter(&self) -> #to_ty {
                        self.0[#index].into()
                    }

                    pub fn #setter(&mut self, value: #to_ty) {
                        self.0[#index] = value.into();
                    }
                },
                FieldKind::TryInto(to_ty) => quote! {
                    pub fn #getter(&self) -> core::result::Result<#to_ty, <#to_ty as core::convert::TryFrom<#elem>>::Error> {
                        self.0[#index].try_into()
                    }

                    pub fn #setter(&mut self, value: #to_ty) {
                        self.0[#index] = value.into();
                    }
                },
            }
        });

        quote! {
            #[repr(transparent)]
            #[derive(Debug)]
            #vis struct #name(pub [#elem; #len]);

            #[allow(unused)]
            impl #name {
                #(#methods)*
            }
        }
    }
}
