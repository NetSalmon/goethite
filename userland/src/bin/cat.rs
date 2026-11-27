#![no_std]
#![no_main]

use goethite_userland::{STDOUT, SyscallOps, Syscalls, args, eprintln, exit};

fn cat_file(path: &str) {
    let fd = match Syscalls::open(path, 0, 0) {
        Ok(fd) => fd,
        Err(err) => {
            eprintln!("cat: {}: {:?}", path, err);
            exit(1);
        }
    };

    let mut buf = [0u8; 256];
    loop {
        match Syscalls::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(err) = Syscalls::write(STDOUT, &buf[..n]) {
                    eprintln!("cat: write error: {:?}", err);
                    let _ = Syscalls::close(fd);
                    exit(1);
                }
            }
            Err(err) => {
                eprintln!("cat: read '{}': {:?}", path, err);
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
            cat_file(path);
        }
    }
    if !had_arg {
        eprintln!("usage: cat <file>...");
        exit(1);
    }
}
