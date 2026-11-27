use std::ffi::{CStr, CString, c_char, c_void};

use c::{
    alloc::{calloc, free, malloc, realloc},
    env::{clearenv, environ, getenv, init_from_stack, putenv, setenv, unsetenv},
    errno::get_errno,
    stdlib::atoi,
    string::{memcmp, memcpy, memmove, memset, strchr, strcmp, strcpy, strlen, strncmp},
};
use goethite_abi::PosixErrno;
use goethite_stack::{Input, StackRef, required_len, write};

#[test]
fn malloc_calloc_realloc_free_work() {
    unsafe {
        let p = malloc(64) as *mut u8;
        assert!(!p.is_null());
        for i in 0..64 {
            *p.add(i) = i as u8;
        }

        let p2 = realloc(p as *mut c_void, 256) as *mut u8;
        assert!(!p2.is_null());
        for i in 0..64 {
            assert_eq!(*p2.add(i), i as u8);
        }
        free(p2 as *mut c_void);

        let z = calloc(16, 8) as *mut u8;
        assert!(!z.is_null());
        for i in 0..128 {
            assert_eq!(*z.add(i), 0);
        }
        free(z as *mut c_void);
    }
}

#[test]
fn string_and_mem_functions() {
    let a = CString::new("hello").unwrap();
    let b = CString::new("hello").unwrap();
    let d = CString::new("world").unwrap();
    unsafe {
        assert_eq!(strlen(a.as_ptr()), 5);
        assert_eq!(strcmp(a.as_ptr(), b.as_ptr()), 0);
        assert!(strcmp(a.as_ptr(), d.as_ptr()) < 0);
        assert_eq!(strncmp(a.as_ptr(), d.as_ptr(), 0), 0);

        let mut dst = [0 as c_char; 16];
        strcpy(dst.as_mut_ptr(), a.as_ptr());
        assert_eq!(strcmp(dst.as_ptr(), a.as_ptr()), 0);

        let pos = strchr(a.as_ptr(), b'l' as i32);
        assert_eq!(pos.offset_from(a.as_ptr()), 2);

        let mut buf = *b"abcdef";
        memmove(
            buf.as_mut_ptr().add(2) as *mut c_void,
            buf.as_ptr() as *const c_void,
            4,
        );
        assert_eq!(&buf, b"ababcd");

        let mut dst2 = [0u8; 6];
        memcpy(dst2.as_mut_ptr() as *mut c_void, buf.as_ptr() as *const c_void, 6);
        assert_eq!(
            memcmp(dst2.as_ptr() as *const c_void, buf.as_ptr() as *const c_void, 6),
            0
        );

        memset(dst2.as_mut_ptr() as *mut c_void, b'x' as i32, 6);
        assert_eq!(&dst2, b"xxxxxx");

        let num = CString::new("  -42abc").unwrap();
        assert_eq!(atoi(num.as_ptr()), -42);
    }
}

#[test]
fn posix_common_env_vars_pwd_path_home_user() {
    let input = Input {
        argv: &["/sh"],
        env: &[
            ("PATH", "/bin:/usr/bin:/"),
            ("PWD", "/"),
            ("HOME", "/root"),
            ("USER", "root"),
            ("SHELL", "/sh"),
        ],
        auxv: &[],
    };
    let len = required_len(&input);
    let mut raw = vec![0u8; len + 16];
    let off = (16 - (raw.as_ptr() as usize % 16)) % 16;
    let slice = &mut raw[off..off + len];
    write(slice, &input).unwrap();
    let view = StackRef::parse(slice).unwrap();

    unsafe {
        init_from_stack(&view);

        let k_path = CString::new("PATH").unwrap();
        let k_pwd = CString::new("PWD").unwrap();
        let k_oldpwd = CString::new("OLDPWD").unwrap();
        let k_home = CString::new("HOME").unwrap();
        let k_user = CString::new("USER").unwrap();
        let k_shell = CString::new("SHELL").unwrap();

        assert_eq!(
            CStr::from_ptr(getenv(k_path.as_ptr())).to_str().unwrap(),
            "/bin:/usr/bin:/"
        );
        assert_eq!(
            CStr::from_ptr(getenv(k_pwd.as_ptr())).to_str().unwrap(),
            "/"
        );
        assert_eq!(
            CStr::from_ptr(getenv(k_home.as_ptr())).to_str().unwrap(),
            "/root"
        );
        assert_eq!(
            CStr::from_ptr(getenv(k_user.as_ptr())).to_str().unwrap(),
            "root"
        );
        assert_eq!(
            CStr::from_ptr(getenv(k_shell.as_ptr())).to_str().unwrap(),
            "/sh"
        );

        // 模拟切换目录：更新 PWD 与新增 OLDPWD
        let v_root = CString::new("/").unwrap();
        let v_etc = CString::new("/etc").unwrap();
        assert_eq!(setenv(k_oldpwd.as_ptr(), v_root.as_ptr(), 1), 0);
        assert_eq!(setenv(k_pwd.as_ptr(), v_etc.as_ptr(), 1), 0);
        assert_eq!(
            CStr::from_ptr(getenv(k_oldpwd.as_ptr())).to_str().unwrap(),
            "/"
        );
        assert_eq!(
            CStr::from_ptr(getenv(k_pwd.as_ptr())).to_str().unwrap(),
            "/etc"
        );

        // PATH overwrite = 0 保持原值；overwrite = 1 覆盖
        let v_path2 = CString::new("/usr/local/bin:/bin:/").unwrap();
        assert_eq!(setenv(k_path.as_ptr(), v_path2.as_ptr(), 0), 0);
        assert_eq!(
            CStr::from_ptr(getenv(k_path.as_ptr())).to_str().unwrap(),
            "/bin:/usr/bin:/"
        );
        assert_eq!(setenv(k_path.as_ptr(), v_path2.as_ptr(), 1), 0);
        assert_eq!(
            CStr::from_ptr(getenv(k_path.as_ptr())).to_str().unwrap(),
            "/usr/local/bin:/bin:/"
        );

        // 非法键名返回 -1 且设置 EINVAL
        let bad = CString::new("PATH=/bad").unwrap();
        assert_eq!(setenv(bad.as_ptr(), v_etc.as_ptr(), 1), -1);
        assert_eq!(get_errno(), PosixErrno::EINVAL.0 as i32);

        // putenv 新增 TERM
        let mut kv_term = b"TERM=xterm-256color\0".to_vec();
        assert_eq!(putenv(kv_term.as_mut_ptr() as *mut c_char), 0);
        let k_term = CString::new("TERM").unwrap();
        assert_eq!(
            CStr::from_ptr(getenv(k_term.as_ptr())).to_str().unwrap(),
            "xterm-256color"
        );

        // unsetenv 删除 USER
        assert_eq!(unsetenv(k_user.as_ptr()), 0);
        assert!(getenv(k_user.as_ptr()).is_null());

        // 验证 environ 以 NULL 结尾且包含 6 项（PATH, PWD, HOME, SHELL, OLDPWD, TERM）
        let mut count = 0usize;
        let arr = environ;
        assert!(!arr.is_null());
        while !(*arr.add(count)).is_null() {
            count += 1;
        }
        assert_eq!(count, 6);

        assert_eq!(clearenv(), 0);
        assert!(environ.is_null());
    }
}
