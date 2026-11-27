//! 标识符大小写转换辅助函数。
//!
//! 被 `array_struct` / `bits` 等过程宏使用，把字段名从上游命名风格（`CamelCase`）
//! 规范化为 snake_case 的访问器名。当前提供 [`to_snake`]。

use proc_macro2::Ident;

/// `CamelCase` -> `camel_case`；已经是 snake_case 的标识符原样保留。
pub(crate) fn to_snake(ident: &Ident) -> Ident {
    let name = ident.to_string();
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);

    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            // 大写字母前插入下划线：前一个字符是小写/数字（OsAbi），或
            // 前一个是大写且后一个是小写（HTTPServer、ABIVersion）。
            let boundary = i > 0
                && (chars[i - 1].is_ascii_lowercase()
                    || chars[i - 1].is_ascii_digit()
                    || chars.get(i + 1).is_some_and(|&n| n.is_ascii_lowercase()));
            if boundary {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }

    Ident::new(&out, ident.span())
}

#[cfg(test)]
mod tests {
    use proc_macro2::{Ident, Span};

    use super::to_snake;

    fn snake(name: &str) -> String {
        to_snake(&Ident::new(name, Span::call_site())).to_string()
    }

    #[test]
    fn converts_camel_case() {
        assert_eq!(snake("CamelCase"), "camel_case");
        assert_eq!(snake("Hello"), "hello");
        assert_eq!(snake("X"), "x");
        assert_eq!(snake("A1B2"), "a1_b2");
    }

    #[test]
    fn handles_acronyms() {
        assert_eq!(snake("OsAbi"), "os_abi");
        assert_eq!(snake("HTTPServer"), "http_server");
        assert_eq!(snake("ABIVersion"), "abi_version");
        assert_eq!(snake("PHDR"), "phdr");
    }

    #[test]
    fn keeps_snake_case() {
        assert_eq!(snake("already_snake"), "already_snake");
        assert_eq!(snake("raw"), "raw");
        assert_eq!(snake("field2"), "field2");
    }
}
