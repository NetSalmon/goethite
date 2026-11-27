//! `goethite-macros` 生成代码的行为测试。
//!
//! 这里通过真实使用 `numeric!` / `bits!` / `array_struct!` / `const_val`
//! 宏，验证生成的常量、访问器、转换与调试输出。

use goethite_macros::{array_struct, bits, const_val, numeric};

numeric! {
    pub enum Color : u8 {
        RED = 1,
        GREEN = 2,
        BLUE = 4,
    }
}

numeric! {
    pub enum Code : u16 {
        OK = 0,
        CLIENT_ERROR = 400 => 499,
        SERVER_ERROR = 500 => 599,
    }
}

numeric! {
    pub enum Offset with ops : i32 {
        ZERO = 0,
        FOUR = 4,
    }
}

numeric! {
    pub enum Level : u32 {
        LOW = 0,
        MEDIUM = 1,
        HIGH = 2,
    }
}

bits! {
    pub type Flags : u32 {
        ready: 0,
        busy: 1,
        level: 2 => 5,
        kind: Level : 6 => 9,
    }
}

array_struct! {
    pub struct Ident : [u8; 8] {
        Class: Color => 0,
        OsAbi: @try u8 => 1,
        Tag => 2,
        NIndex => 3,
    }
}

#[const_val(min = 3, max = 12, multiple_of = 2)]
const EVEN_VALUE: usize = 6;

#[test]
fn numeric_constants_and_conversions() {
    assert_eq!(Color::RED.0, 1);
    assert_eq!(Color::GREEN.0, 2);
    assert_eq!(Color::BLUE.0, 4);

    let raw: u8 = Color::GREEN.into();
    assert_eq!(raw, 2);
    assert_eq!(Color::from(4u8), Color::BLUE);
    assert_eq!(format!("{:?}", Color::RED), "Color::RED");
    assert_eq!(format!("{:?}", Color::from(9u8)), "unknown");
}

#[test]
fn numeric_ranges_and_predicates() {
    assert!(Code::is_client_error(400));
    assert!(Code::is_client_error(499));
    assert!(!Code::is_client_error(399));
    assert!(!Code::is_client_error(500));
    assert!(Code::is_server_error(500));
    assert!(Code::is_server_error(599));
    assert!(!Code::is_server_error(600));

    assert_eq!(format!("{:?}", Code::OK), "Code::OK");
    assert_eq!(format!("{:?}", Code::from(404u16)), "Code::CLIENT_ERROR");
    assert_eq!(format!("{:?}", Code::from(503u16)), "Code::SERVER_ERROR");
}

#[test]
fn numeric_ops() {
    assert_eq!(Offset::FOUR + Offset::ZERO, Offset::FOUR);
    assert_eq!(Offset::FOUR - Offset::FOUR, Offset::ZERO);

    let mixed: i32 = Offset::FOUR + 1;
    assert_eq!(mixed, 5);
    assert_eq!(1 + Offset::FOUR, 5);
    assert_eq!(Offset::FOUR - 2, 2);
    assert_eq!(10 - Offset::FOUR, 6);
    assert_eq!(Offset::FOUR * 2, 8);
    assert_eq!(2 * Offset::FOUR, 8);
    assert_eq!(Offset::FOUR / 2, 2);
    assert_eq!(8 / Offset::FOUR, 2);
}

#[test]
fn bits_accessors_and_builder() {
    let mut flags = Flags::new();
    assert_eq!(u32::from(flags), 0);
    assert!(!flags.ready());
    assert_eq!(flags.level(), 0);

    flags.set_ready(true);
    flags.set_busy(true);
    flags.set_level(0b1010);
    flags.set_kind(Level::HIGH);
    assert!(flags.ready());
    assert!(flags.busy());
    assert_eq!(flags.level(), 0b1010);
    assert_eq!(flags.kind(), Level::HIGH);
    assert_eq!(u32::from(flags), 1 | (1 << 1) | (0b1010 << 2) | (2 << 6));

    let built = Flags::builder()
        .set_ready(true)
        .set_level(3)
        .set_kind(Level::MEDIUM)
        .build();
    assert!(built.ready());
    assert!(!built.busy());
    assert_eq!(built.level(), 3);
    assert_eq!(built.kind(), Level::MEDIUM);

    // `Deref` 暴露底层整数。
    assert_eq!(*built, u32::from(built));
    assert!(format!("{built:?}").contains("raw:"));
}

#[test]
fn array_struct_accessors_use_snake_case() {
    let mut ident = Ident([0u8; 8]);
    ident.set_class(Color::GREEN);
    ident.set_os_abi(9);
    ident.set_tag(7);
    ident.set_n_index(8);

    assert_eq!(ident.class(), Color::GREEN);
    assert_eq!(ident.os_abi().unwrap(), 9);
    assert_eq!(ident.tag(), 7);
    assert_eq!(ident.n_index(), 8);
    assert_eq!(ident.0, [2, 9, 7, 8, 0, 0, 0, 0]);
}

#[test]
fn const_val_keeps_valid_value() {
    assert_eq!(EVEN_VALUE, 6);
}
