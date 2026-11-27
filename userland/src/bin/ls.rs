#![no_std]
#![no_main]

use core::str;

use goethite_userland::{SyscallOps, Syscalls, args, eprintln, exit, println};

fn list_dir(path: &str) {
    let fd = match Syscalls::open(path, 0, 0) {
        Ok(fd) => fd,
        Err(err) => {
            eprintln!("ls: cannot open '{}': {:?}", path, err);
            exit(1);
        }
    };

    let mut name_buf = [0u8; 64];
    loop {
        match Syscalls::read_dir(fd, &mut name_buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Ok(name) = str::from_utf8(&name_buf[..n]) {
                    println!("{}", name);
                }
            }
            Err(err) => {
                eprintln!("ls: read_dir '{}': {:?}", path, err);
                let _ = Syscalls::close(fd);
                exit(1);
            }
        }
    }

    let _ = Syscalls::close(fd);
}

#[unsafe(no_mangle)]
pub fn main() {
    let mut had_arg = false;
    if let Some(iter) = args() {
        for path in iter.skip(1) {
            had_arg = true;
            list_dir(path);
        }
    }
    if !had_arg {
        list_dir("/");
    }
}
