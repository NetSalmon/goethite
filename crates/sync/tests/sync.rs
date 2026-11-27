use std::sync::atomic::{AtomicUsize, Ordering};

use goethite_sync::{LazyLock, OnceLock, PerCpu, RwLock, SpinLock};

#[test]
fn once_lock_initializes_once() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);

    let cell = OnceLock::new();
    assert!(cell.get().is_none());

    let value = cell.get_or_init(|| {
        CALLS.fetch_add(1, Ordering::SeqCst);
        41 + 1
    });
    assert_eq!(*value, 42);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);

    let again = cell.get_or_init(|| {
        CALLS.fetch_add(1, Ordering::SeqCst);
        0
    });
    assert_eq!(*again, 42);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(*cell.get().unwrap(), 42);
}

#[test]
fn once_lock_drops_initialized_value() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Tracked;

    impl Drop for Tracked {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    {
        let cell = OnceLock::new();
        cell.get_or_init(|| Tracked);
        assert_eq!(DROPS.load(Ordering::SeqCst), 0);
    }
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);

    // 未初始化的容器在析构时不应运行初始化闭包或析构值。
    {
        let cell: OnceLock<Tracked> = OnceLock::new();
        drop(cell);
    }
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
}

#[test]
fn lazy_lock_initializes_on_first_access() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);

    let lazy = LazyLock::new(|| {
        CALLS.fetch_add(1, Ordering::SeqCst);
        String::from("initialized")
    });
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(lazy.force(), "initialized");
    assert_eq!(&*lazy, "initialized");
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);

    let first = lazy.force() as *const String;
    let second = lazy.force() as *const String;
    assert_eq!(first, second);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn spin_lock_serializes_access() {
    let lock = SpinLock::new(0u32);
    {
        let mut guard = lock.lock();
        assert_eq!(*guard, 0);
        *guard += 1;
        assert_eq!(*guard, 1);
    }
    {
        let mut guard = lock.lock();
        *guard += 1;
    }
    assert_eq!(*lock.lock(), 2);
}

#[test]
fn rwlock_readers_and_writer() {
    let lock = RwLock::new(vec![1, 2, 3]);

    {
        let first = lock.read();
        let second = lock.read();
        assert_eq!(*first, vec![1, 2, 3]);
        assert_eq!(*second, vec![1, 2, 3]);
    }

    {
        let mut writer = lock.write();
        writer.push(4);
        writer[0] = 10;
    }

    assert_eq!(*lock.read(), vec![10, 2, 3, 4]);
}

#[test]
fn per_cpu_uses_current_hart() {
    // 宿主机构建 `tp` 固定为 0，因此只会访问第 0 个槽位。
    let per_cpu = PerCpu::<u32, 1>::new([7]);
    {
        let mut guard = per_cpu.current();
        assert_eq!(*guard, 7);
        *guard += 1;
    }
    assert_eq!(*per_cpu.current(), 8);

    let per_cpu = PerCpu::<_, 2>::new([String::from("hart0"), String::from("hart1")]);
    assert_eq!(&*per_cpu.current(), "hart0");
}

#[test]
fn hart_id_is_zero_on_host() {
    assert_eq!(goethite_riscv64::hart_id(), 0);
}
