use goethite_stack::{
    AT_ENTRY, AT_PAGESZ, AT_PHDR, AT_PHNUM, Input, Stack, StackRef, required_len, write,
};

#[test]
fn test_stack_serialize_and_parse() {
    let argv = ["/bin/init", "--verbose", "arg2"];
    let env = [("PATH", "/bin:/usr/bin"), ("USER", "root")];
    let auxv = [
        (AT_PHDR, 0x0040_0000_u64),
        (AT_PAGESZ, 4096_u64),
        (AT_PHNUM, 5_u64),
        (AT_ENTRY, 0x0040_1000_u64),
        // 测试超过 32 位的大地址
        (256, 0xffff_8000_0010_0000_u64),
    ];

    let input = Input {
        argv: &argv,
        env: &env,
        auxv: &auxv,
    };

    let len = required_len(&input);
    let mut buf = vec![0u8; len];
    let written = write(&mut buf, &input).expect("write failed");
    assert_eq!(written, len);

    // 零拷贝视图
    let view = StackRef::parse(&buf).expect("parse view failed");
    assert_eq!(view.argc(), 3);
    let view_args: Vec<&str> = view.argv().collect();
    assert_eq!(view_args, argv);

    assert_eq!(view.env_count(), 2);
    let view_env: Vec<(&str, &str)> = view.env().collect();
    assert_eq!(view_env, env);

    assert_eq!(view.auxv_count(), 5);
    let view_aux: Vec<(u64, u64)> = view.auxv().collect();
    assert_eq!(view_aux, auxv);

    assert_eq!(view.find_aux(AT_PHDR), Some(0x0040_0000));
    assert_eq!(view.find_aux(AT_PAGESZ), Some(4096));
    assert_eq!(view.find_aux(AT_PHNUM), Some(5));
    assert_eq!(view.find_aux(AT_ENTRY), Some(0x0040_1000));
    assert_eq!(view.find_aux(256), Some(0xffff_8000_0010_0000));
    assert_eq!(view.find_aux(999), None);

    // 拥有所有权 Stack
    let owned = Stack::parse(&buf).expect("parse owned failed");
    assert_eq!(owned.argv, argv);
    assert_eq!(owned.env.get("PATH").unwrap(), "/bin:/usr/bin");
    assert_eq!(owned.env.get("USER").unwrap(), "root");
    assert_eq!(owned.auxv.get(&AT_PHDR), Some(&0x0040_0000));
    assert_eq!(owned.auxv.get(&AT_ENTRY), Some(&0x0040_1000));
    assert_eq!(owned.auxv.get(&256), Some(&0xffff_8000_0010_0000));

    // 再次序列化
    let reserialized = owned.to_bytes().expect("to_bytes failed");
    let view2 = StackRef::parse(&reserialized).expect("re-parse failed");
    assert_eq!(view2.find_aux(AT_PHDR), Some(0x0040_0000));
    assert_eq!(view2.find_aux(AT_ENTRY), Some(0x0040_1000));
}

#[test]
fn test_empty_auxv() {
    let input = Input {
        argv: &["test"],
        env: &[],
        auxv: &[],
    };
    let len = required_len(&input);
    let mut buf = vec![0u8; len];
    write(&mut buf, &input).unwrap();

    let view = StackRef::parse(&buf).unwrap();
    assert_eq!(view.auxv_count(), 0);
    assert_eq!(view.find_aux(AT_PHDR), None);
    assert_eq!(view.auxv().count(), 0);
}
