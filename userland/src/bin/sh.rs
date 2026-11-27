#![no_std]
#![no_main]

use core::str;

use goethite_userland::{
    PosixErrno, STDIN, STDOUT, SyscallOps, Syscalls,
    alloc::string::String,
    env, eprintln, exit, print, println,
};

const MAX_LINE: usize = 256;
const MAX_ARGS: usize = 16;

fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0usize;
    let mut byte = [0u8; 1];

    loop {
        match Syscalls::read(STDIN, &mut byte) {
            Ok(0) => {
                let _ = Syscalls::yield_now();
            }
            Ok(_) => {
                let ch = byte[0];
                match ch {
                    b'\r' | b'\n' => {
                        let _ = Syscalls::write(STDOUT, b"\n");
                        break;
                    }
                    0x08 | 0x7f => {
                        if len > 0 {
                            len -= 1;
                            let _ = Syscalls::write(STDOUT, b"\x08 \x08");
                        }
                    }
                    0x20..=0x7e => {
                        if len < buf.len() {
                            buf[len] = ch;
                            len += 1;
                            let _ = Syscalls::write(STDOUT, &byte);
                        }
                    }
                    _ => {}
                }
            }
            Err(_) => {
                let _ = Syscalls::yield_now();
            }
        }
    }

    len
}

fn split_args<'a>(line: &'a str, out: &mut [&'a str; MAX_ARGS]) -> usize {
    let mut count = 0;
    for part in line.split_ascii_whitespace() {
        if count >= MAX_ARGS {
            break;
        }
        out[count] = part;
        count += 1;
    }
    count
}

fn run_external(argv: &[&str]) {
    let cmd = argv[0];

    match Syscalls::spawn_process() {
        Ok(0) => {
            if cmd.starts_with('/') {
                if let Err(err) = env::execve(cmd, argv) {
                    eprintln!("sh: {}: {:?}", cmd, err);
                    exit(127);
                }
            } else {
                let path_var = env::get("PATH").unwrap_or_else(|| String::from("/"));
                let mut last_err = PosixErrno::ENOENT;
                for dir in path_var.split(':').filter(|d| !d.is_empty()) {
                    let mut full = String::from(dir);
                    if !full.ends_with('/') {
                        full.push('/');
                    }
                    full.push_str(cmd);
                    match env::execve(&full, argv) {
                        Ok(_) => unreachable!(),
                        Err(err) => last_err = err,
                    }
                }
                eprintln!("sh: {}: {:?}", cmd, last_err);
                exit(127);
            }
        }
        Ok(child_tid) => loop {
            match Syscalls::wait(child_tid) {
                Ok(_) => break,
                Err(PosixErrno::EAGAIN) => {
                    let _ = Syscalls::yield_now();
                }
                Err(err) => {
                    eprintln!("sh: wait failed: {:?}", err);
                    break;
                }
            }
        },
        Err(err) => {
            eprintln!("sh: spawn_process failed: {:?}", err);
        }
    }
}

#[unsafe(no_mangle)]
pub fn main() {
    println!("goethite shell (type 'help' for commands)");

    let mut line_buf = [0u8; MAX_LINE];

    loop {
        print!("goethite# ");
        let n = read_line(&mut line_buf);
        let Ok(line) = str::from_utf8(&line_buf[..n]) else {
            continue;
        };

        let mut argv = [""; MAX_ARGS];
        let argc = split_args(line, &mut argv);
        if argc == 0 {
            continue;
        }

        match argv[0] {
            "help" => {
                println!("Built-in commands: help, pwd, cd [dir], env, export KEY=VAL, unset KEY, exit");
                println!("External tools:    ls [dir], cat <file>..., echo [args...], hello");
            }
            "pwd" => {
                let pwd = env::get("PWD").unwrap_or_else(|| String::from("/"));
                println!("{}", pwd);
            }
            "cd" => {
                let target = if argc >= 2 {
                    String::from(argv[1])
                } else {
                    env::get("HOME").unwrap_or_else(|| String::from("/"))
                };
                if let Some(old) = env::get("PWD") {
                    env::set("OLDPWD", old);
                }
                env::set("PWD", target);
            }
            "env" => {
                for (k, v) in env::vars() {
                    println!("{}={}", k, v);
                }
            }
            "export" => {
                if argc < 2 {
                    eprintln!("usage: export KEY=VALUE");
                } else {
                    for &item in &argv[1..argc] {
                        if let Some((k, v)) = item.split_once('=') {
                            if !k.is_empty() {
                                env::set(k, v);
                            }
                        } else {
                            eprintln!("export: expected KEY=VALUE, got '{}'", item);
                        }
                    }
                }
            }
            "unset" => {
                if argc < 2 {
                    eprintln!("usage: unset KEY...");
                } else {
                    for &k in &argv[1..argc] {
                        let _ = env::remove(k);
                    }
                }
            }
            "exit" => {
                println!("bye");
                exit(0);
            }
            _ => run_external(&argv[..argc]),
        }
    }
}
