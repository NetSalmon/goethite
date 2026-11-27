use goethite_abi::{ForkFlags, PosixErrno, SysResult, Syscall, SyscallOps};

#[test]
fn errno_values_match_linux() {
    assert_eq!(PosixErrno::EPERM.0, 1);
    assert_eq!(PosixErrno::ENOENT.0, 2);
    assert_eq!(PosixErrno::EIO.0, 5);
    assert_eq!(PosixErrno::EAGAIN.0, 11);
    assert_eq!(PosixErrno::ENOMEM.0, 12);
    assert_eq!(PosixErrno::EINVAL.0, 22);
    assert_eq!(PosixErrno::ENOSYS.0, 38);
    assert_eq!(PosixErrno::ENOTSUP.0, 95);
    assert_eq!(PosixErrno::EHWPOISON.0, 133);
}

#[test]
fn errno_aliases_share_values() {
    assert_eq!(PosixErrno::EAGAIN, PosixErrno::EWOULDBLOCK);
    assert_eq!(PosixErrno::EDEADLK, PosixErrno::EDEADLOCK);
    assert_eq!(PosixErrno::EOPNOTSUPP, PosixErrno::ENOTSUP);
}

#[test]
fn errno_conversions_and_debug() {
    let errno: i64 = PosixErrno::ENOENT.into();
    assert_eq!(errno, 2);
    assert_eq!(PosixErrno::from(22i64), PosixErrno::EINVAL);
    assert_eq!(format!("{:?}", PosixErrno::EACCES), "PosixErrno::EACCES");
    assert_eq!(format!("{:?}", PosixErrno::from(41i64)), "unknown");
}

#[test]
fn syscall_numbers() {
    assert_eq!(Syscall::OPEN.0, 0);
    assert_eq!(Syscall::CLOSE.0, 1);
    assert_eq!(Syscall::READ.0, 2);
    assert_eq!(Syscall::WRITE.0, 3);
    assert_eq!(Syscall::EXECVE.0, 7);
    assert_eq!(Syscall::SPAWN_PROCESS.0, 8);
    assert_eq!(Syscall::SPAWN_THREAD.0, 9);

    let number: u64 = Syscall::WRITE.into();
    assert_eq!(number, 3);
    assert_eq!(Syscall::from(6u64), Syscall::FORK);
}

#[test]
fn fork_flags_presets() {
    let proc_flags = ForkFlags::process();
    assert!(!proc_flags.share_vm());
    assert!(!proc_flags.share_files());
    assert!(!proc_flags.share_tgid());
    assert!(!proc_flags.share_stack());

    let thread_flags = ForkFlags::thread();
    assert!(thread_flags.share_vm());
    assert!(thread_flags.share_files());
    assert!(thread_flags.share_tgid());
    assert!(!thread_flags.share_stack());
}

struct Unimplemented;

impl SyscallOps for Unimplemented {}

#[test]
fn default_syscall_methods_report_enosys() {
    assert_eq!(Unimplemented::open("/init", 0, 0), Err(PosixErrno::ENOSYS));
    assert_eq!(Unimplemented::close(3), Err(PosixErrno::ENOSYS));

    let mut buffer = [0u8; 4];
    assert_eq!(Unimplemented::read(3, &mut buffer), Err(PosixErrno::ENOSYS));
    assert_eq!(Unimplemented::write(3, b"data"), Err(PosixErrno::ENOSYS));
    assert_eq!(
        Unimplemented::read_dir(3, &mut buffer),
        Err(PosixErrno::ENOSYS)
    );
    assert_eq!(Unimplemented::exit(0), Err(PosixErrno::ENOSYS));
    assert_eq!(Unimplemented::wait(0), Err(PosixErrno::ENOSYS));
    assert_eq!(Unimplemented::yield_now(), Err(PosixErrno::ENOSYS));
    assert_eq!(Unimplemented::stat(3), Err(PosixErrno::ENOSYS));
    assert_eq!(
        Unimplemented::file_stat("/etc/passwd"),
        Err(PosixErrno::ENOSYS)
    );
    assert_eq!(
        Unimplemented::fork(ForkFlags::process(), 0, 0, 0),
        Err(PosixErrno::ENOSYS)
    );
    assert_eq!(Unimplemented::spawn_process(), Err(PosixErrno::ENOSYS));
    assert_eq!(
        Unimplemented::spawn_thread(0x1000, 0x2000, 42),
        Err(PosixErrno::ENOSYS)
    );
    assert_eq!(
        Unimplemented::execve("/bin/sh", &["sh"], &[("PATH", "/")]),
        Err(PosixErrno::ENOSYS)
    );
    assert_eq!(
        Unimplemented::anonymous_map(0x1000, 0, 0x1000, 0),
        Err(PosixErrno::ENOSYS)
    );
    assert_eq!(
        Unimplemented::file_map(0x1000, 0, 0x1000, 0, "/init", 0),
        Err(PosixErrno::ENOSYS)
    );

    let result: SysResult<()> = Err(PosixErrno::EINVAL);
    assert_eq!(result, Err(PosixErrno::EINVAL));
}
