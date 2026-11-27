//! `goethite-sync` 的 QEMU 测试。
//!
//! 同步原语在宿主机上也能运行（`riscv64` 提供关中断空实现），但 QEMU 测试
//! 会走真实的 CSR 关中断路径，因此保留一组目标机冒烟测试。

use core::sync::atomic::{AtomicUsize, Ordering};

use goethite_test::{assert_eq, test_suite};

use crate::{LazyLock, OnceLock, PerCpu, RwLock, SpinLock};

static ONCE: OnceLock<u32> = OnceLock::new();
static LAZY_INIT_COUNT: AtomicUsize = AtomicUsize::new(0);

fn lazy_init() -> usize {
    LAZY_INIT_COUNT.fetch_add(1, Ordering::SeqCst);
    0xfeed_face
}

test_suite! {
    fn once_lock_initialises_once() {
        assert_eq!(ONCE.get(), None);
        assert_eq!(*ONCE.get_or_init(|| 42), 42);
        assert_eq!(ONCE.get(), Some(&42));
        // 再次调用不会覆盖已有值。
        assert_eq!(*ONCE.get_or_init(|| 99), 42);
    }

    fn lazy_lock_caches_value() {
        let lazy = LazyLock::new(lazy_init);
        assert_eq!(*lazy, 0xfeed_face);
        assert_eq!(*lazy, 0xfeed_face);
        assert_eq!(LAZY_INIT_COUNT.load(Ordering::SeqCst), 1);
    }

    fn spin_lock_guards_value() {
        let lock = SpinLock::new(0u32);
        {
            let mut guard = lock.lock();
            *guard += 1;
        }
        {
            let mut guard = lock.lock();
            *guard += 1;
        }
        assert_eq!(*lock.lock(), 2);
    }

    fn rwlock_allows_read_write() {
        let lock = RwLock::new(7u32);
        assert_eq!(*lock.read(), 7);
        {
            let mut guard = lock.write();
            *guard = 11;
        }
        assert_eq!(*lock.read(), 11);
    }

    fn per_cpu_current_slot_roundtrip() {
        let per_cpu = PerCpu::new([5u32]);
        {
            let mut value = per_cpu.current();
            assert_eq!(*value, 5);
            *value = 6;
        }
        assert_eq!(*per_cpu.current(), 6);
    }
}
