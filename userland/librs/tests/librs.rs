use std::alloc::{GlobalAlloc, Layout};

use goethite_stack::{Input, StackRef, required_len, write};
use rs::{
    env::{self, EnvStore},
    heap::LockedHeap,
};

#[test]
fn linked_list_heap_alloc_dealloc_and_coalesce() {
    let heap: LockedHeap<4096> = LockedHeap::new();
    assert_eq!(heap.size(), 4096);
    assert_eq!(heap.used(), 0);

    let l1 = Layout::from_size_align(128, 16).unwrap();
    let l2 = Layout::from_size_align(256, 64).unwrap();
    let l3 = Layout::from_size_align(512, 8).unwrap();

    unsafe {
        let p1 = heap.alloc(l1);
        let p2 = heap.alloc(l2);
        let p3 = heap.alloc(l3);
        assert!(!p1.is_null());
        assert!(!p2.is_null());
        assert!(!p3.is_null());
        assert_eq!(p1 as usize % 16, 0);
        assert_eq!(p2 as usize % 64, 0);
        assert!(heap.used() >= 128 + 256 + 512);

        p1.write_bytes(0xAA, 128);
        p2.write_bytes(0xBB, 256);
        p3.write_bytes(0xCC, 512);

        // 乱序释放并验证相邻块合并后可分配大块
        heap.dealloc(p2, l2);
        heap.dealloc(p1, l1);
        heap.dealloc(p3, l3);
        assert_eq!(heap.used(), 0);

        let big = Layout::from_size_align(3500, 16).unwrap();
        let p_big = heap.alloc(big);
        assert!(!p_big.is_null(), "coalescing should allow large allocation");
        heap.dealloc(p_big, big);
        assert_eq!(heap.used(), 0);
    }
}

#[test]
fn linked_list_heap_oom_returns_null() {
    let heap: LockedHeap<256> = LockedHeap::new();
    let too_big = Layout::from_size_align(512, 8).unwrap();
    assert!(unsafe { heap.alloc(too_big) }.is_null());
}

#[test]
fn common_env_vars_pwd_path_home_user_shell() {
    let input = Input {
        argv: &["/sh", "-i"],
        env: &[
            ("PATH", "/bin:/usr/bin:/"),
            ("PWD", "/"),
            ("HOME", "/root"),
            ("USER", "root"),
            ("SHELL", "/sh"),
            ("TERM", "vt100"),
        ],
        auxv: &[(6, 4096)],
    };
    let len = required_len(&input);
    let mut raw = vec![0u8; len + 16];
    let align_off = (16 - (raw.as_ptr() as usize % 16)) % 16;
    let slice = &mut raw[align_off..align_off + len];
    write(slice, &input).unwrap();

    // 1. 测试独立 EnvStore
    let view = StackRef::parse(slice).unwrap();
    let mut store = EnvStore::from_stack(&view);
    assert_eq!(store.get("PATH"), Some("/bin:/usr/bin:/"));
    assert_eq!(store.get("PWD"), Some("/"));
    assert_eq!(store.get("HOME"), Some("/root"));
    assert_eq!(store.get("USER"), Some("root"));
    assert_eq!(store.get("SHELL"), Some("/sh"));
    assert_eq!(store.get("TERM"), Some("vt100"));

    // 模拟 PATH 解析
    let dirs: Vec<&str> = store.get("PATH").unwrap().split(':').collect();
    assert_eq!(dirs, vec!["/bin", "/usr/bin", "/"]);

    // 模拟 cd /etc 更新 PWD / OLDPWD
    let old_pwd = store.get("PWD").unwrap().to_string();
    store.set("OLDPWD", old_pwd);
    store.set("PWD", "/etc");
    assert_eq!(store.get("OLDPWD"), Some("/"));
    assert_eq!(store.get("PWD"), Some("/etc"));

    // 2. 测试全局懒加载 env API
    unsafe { rs::set_stack_image(slice.as_ptr()) };
    env::reload_from_stack();

    assert_eq!(env::get("PATH").as_deref(), Some("/bin:/usr/bin:/"));
    assert_eq!(env::get("PWD").as_deref(), Some("/"));
    assert_eq!(env::get("HOME").as_deref(), Some("/root"));

    // 修改 PATH 与 PWD，新增自定义变量
    env::set("PATH", "/usr/local/bin:/bin:/");
    env::set("OLDPWD", "/");
    env::set("PWD", "/home/user");
    env::set("LANG", "C.UTF-8");

    assert_eq!(env::get("PATH").as_deref(), Some("/usr/local/bin:/bin:/"));
    assert_eq!(env::get("PWD").as_deref(), Some("/home/user"));
    assert_eq!(env::get("OLDPWD").as_deref(), Some("/"));
    assert_eq!(env::get("LANG").as_deref(), Some("C.UTF-8"));

    // 删除 TERM 并验证快照有序且不包含已删除项
    assert_eq!(env::remove("TERM").as_deref(), Some("vt100"));
    assert!(!env::contains("TERM"));

    let snapshot = env::vars();
    assert!(snapshot.contains(&("PATH".into(), "/usr/local/bin:/bin:/".into())));
    assert!(snapshot.contains(&("PWD".into(), "/home/user".into())));
    assert!(snapshot.contains(&("OLDPWD".into(), "/".into())));
    assert!(snapshot.contains(&("HOME".into(), "/root".into())));
    assert!(!snapshot.iter().any(|(k, _)| k == "TERM"));

    env::clear();
    assert!(env::vars().is_empty());
    unsafe { rs::set_stack_image(core::ptr::null()) };
}
