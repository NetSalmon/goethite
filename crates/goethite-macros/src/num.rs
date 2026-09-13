//! 编译期常量字符串解析。
//!
//! [`parse_digit_*`](parse_digit_usize) 系列 `const fn` 把字符串按
//! 十进制 / 十六进制（`0x`）/ 八进制（`0o`）/ 二进制（`0b`）解析为
//! 整数；[`parse_bool`] 解析布尔值。均供 `#[const_val]` 宏在编译期求值。

/// 数字基底。
pub enum NumberBase {
    /// 十进制（无前缀）。
    Decimal,
    /// 十六进制（`0x` 前缀）。
    Hexadecimal,
    /// 八进制（`0o` 前缀）。
    Octal,
    /// 二进制（`0b` 前缀）。
    Binary,
}

/// 解析状态机。
pub enum Fsm {
    /// 初始状态。
    Start,
    /// 已见 `+` / `-` 符号。
    Sign,
    /// 已见前导 `0`，可能引出进制前缀。
    LeadingZero,
    /// 已见进制前缀（`0x` / `0o` / `0b`）。
    Base,
    /// 正在读取数字主体。
    Number,
}

/// 生成无符号整数的数字字符串解析 `const fn`（[`parse_digit_u8`] 等）。
macro_rules! define_parse_digit {
    ($name:ident, $t:ty) => {
        /// 编译期将数字字符串解析为对应无符号整数。
        ///
        /// 支持十进制与 `0x` / `0o` / `0b` 前缀，允许前导零与 `+` 号；
        /// 字符串为空或包含非法字符时回退到 `default`。
        pub const fn $name(source: &str, default: $t) -> $t {
            if source.is_empty() {
                return default;
            }
            let source = source.as_bytes();
            let mut status = Fsm::Start;

            let mut result: $t = 0;
            let mut form = NumberBase::Decimal;

            let mut i = 0;
            while i < source.len() {
                match (source[i], &status) {
                    (b'+', Fsm::Sign) => {
                        status = Fsm::Sign;
                    }
                    (b'0', Fsm::Sign | Fsm::Start) => {
                        status = Fsm::LeadingZero;
                    }
                    (b'x' | b'X', Fsm::LeadingZero) => {
                        status = Fsm::Base;
                        form = NumberBase::Hexadecimal;
                    }
                    (b'b' | b'B', Fsm::LeadingZero) => {
                        status = Fsm::Base;
                        form = NumberBase::Binary;
                    }
                    (b'o' | b'O', Fsm::LeadingZero) => {
                        status = Fsm::Base;
                        form = NumberBase::Octal;
                    }
                    (
                        b'0'..=b'1',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Decimal => {
                                result = result * 10 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Octal => {
                                result = result * 8 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Binary => {
                                result = result * 2 + (source[i] - b'0') as $t;
                            }
                        }
                    }
                    (
                        b'0'..=b'7',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Decimal => {
                                result = result * 10 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Octal => {
                                result = result * 8 + (source[i] - b'0') as $t;
                            }
                            _ => return default,
                        }
                    }
                    (
                        b'0'..=b'9',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Decimal => {
                                result = result * 10 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'0') as $t;
                            }
                            _ => return default,
                        }
                    }
                    (
                        b'a'..=b'f',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'a' + 10) as $t;
                            }
                            _ => return default,
                        }
                    }
                    (
                        b'A'..=b'F',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'A' + 10) as $t;
                            }
                            _ => return default,
                        }
                    }
                    _ => return default,
                }

                i += 1;
            }

            result
        }
    };
}

/// 生成有符号整数的数字字符串解析 `const fn`（[`parse_digit_i8`] 等）。
macro_rules! define_parse_digit_signed {
    ($name:ident, $t:ty) => {
        /// 编译期将数字字符串解析为对应有符号整数。
        ///
        /// 支持 `-` / `+` 符号与十进制及 `0x` / `0o` / `0b` 前缀；
        /// 字符串为空或包含非法字符时回退到 `default`。
        pub const fn $name(source: &str, default: $t) -> $t {
            if source.is_empty() {
                return default;
            }
            let source = source.as_bytes();
            let mut status = Fsm::Start;

            let mut result: $t = 0;
            let mut is_neg = false;
            let mut form = NumberBase::Decimal;

            let mut i = 0;
            while i < source.len() {
                match (source[i], &status) {
                    (b'-', Fsm::Start) => {
                        is_neg = true;
                        status = Fsm::Sign;
                    }
                    (b'+', Fsm::Sign) => {
                        is_neg = false;
                        status = Fsm::Sign;
                    }
                    (b'0', Fsm::Sign | Fsm::Start) => {
                        status = Fsm::LeadingZero;
                    }
                    (b'x' | b'X', Fsm::LeadingZero) => {
                        status = Fsm::Base;
                        form = NumberBase::Hexadecimal;
                    }
                    (b'b' | b'B', Fsm::LeadingZero) => {
                        status = Fsm::Base;
                        form = NumberBase::Binary;
                    }
                    (b'o' | b'O', Fsm::LeadingZero) => {
                        status = Fsm::Base;
                        form = NumberBase::Octal;
                    }
                    (
                        b'0'..=b'1',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Decimal => {
                                result = result * 10 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Octal => {
                                result = result * 8 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Binary => {
                                result = result * 2 + (source[i] - b'0') as $t;
                            }
                        }
                    }
                    (
                        b'0'..=b'7',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Decimal => {
                                result = result * 10 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Octal => {
                                result = result * 8 + (source[i] - b'0') as $t;
                            }
                            _ => return default,
                        }
                    }
                    (
                        b'0'..=b'9',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Decimal => {
                                result = result * 10 + (source[i] - b'0') as $t;
                            }
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'0') as $t;
                            }
                            _ => return default,
                        }
                    }
                    (
                        b'a'..=b'f',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'a' + 10) as $t;
                            }
                            _ => return default,
                        }
                    }
                    (
                        b'A'..=b'F',
                        Fsm::Base | Fsm::Number | Fsm::LeadingZero | Fsm::Sign | Fsm::Start,
                    ) => {
                        status = Fsm::Number;
                        match form {
                            NumberBase::Hexadecimal => {
                                result = result * 16 + (source[i] - b'A' + 10) as $t;
                            }
                            _ => return default,
                        }
                    }
                    _ => return default,
                }

                i += 1;
            }

            if is_neg { -result } else { result }
        }
    };
}

define_parse_digit!(parse_digit_u8, u8);
define_parse_digit!(parse_digit_u16, u16);
define_parse_digit!(parse_digit_u32, u32);
define_parse_digit!(parse_digit_u64, u64);
define_parse_digit!(parse_digit_u128, u128);
define_parse_digit!(parse_digit_usize, usize);

define_parse_digit_signed!(parse_digit_i8, i8);
define_parse_digit_signed!(parse_digit_i16, i16);
define_parse_digit_signed!(parse_digit_i32, i32);
define_parse_digit_signed!(parse_digit_i64, i64);
define_parse_digit_signed!(parse_digit_i128, i128);
define_parse_digit_signed!(parse_digit_isize, isize);

/// 把字符串解析为布尔值，供 `#[const_val]` 在编译期求值。
///
/// 接受大小写不敏感的 `true` / `false` 与 `1` / `0`；无法识别时返回
/// `default`（与数字解析失败时回退默认值的行为一致）。
pub const fn parse_bool(source: &str, default: bool) -> bool {
    let bytes = source.as_bytes();
    if source.eq_ignore_ascii_case("true") || (bytes.len() == 1 && bytes[0] == b'1') {
        true
    } else if source.eq_ignore_ascii_case("false") || (bytes.len() == 1 && bytes[0] == b'0') {
        false
    } else {
        default
    }
}
