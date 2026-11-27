#![no_std]
#![no_main]

use goethite_userland::{PosixErrno, SyscallOps, Syscalls, eprintln, exit, println};

#[unsafe(no_mangle)]
pub fn main() {
    println!("[init] goethite init started");

    loop {
        match Syscalls::spawn_process() {
            Ok(0) => {
                // 子进程：拉起交互式 shell，并注入常用结构化环境变量
                let args = ["/sh"];
                let env = [
                    ("PATH", "/"),
                    ("PWD", "/"),
                    ("HOME", "/"),
                    ("USER", "root"),
                    ("SHELL", "/sh"),
                ];
                if let Err(err) = Syscalls::execve("/sh", &args, &env) {
                    eprintln!("[init] failed to exec /sh: {:?}", err);
                    exit(127);
                }
            }
            Ok(child_tid) => {
                // 父进程：等待 shell 退出后自动重启
                loop {
                    match Syscalls::wait(child_tid) {
                        Ok(_) => {
                            println!("[init] /sh (tid {}) exited, restarting...", child_tid);
                            break;
                        }
                        Err(PosixErrno::EAGAIN) => {
                            let _ = Syscalls::yield_now();
                        }
                        Err(err) => {
                            eprintln!("[init] wait failed: {:?}", err);
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                eprintln!("[init] spawn_process failed: {:?}", err);
                let _ = Syscalls::yield_now();
            }
        }
    }
}
