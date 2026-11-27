//! goethite 测试入口。
//!
//! 用法（在仓库根目录）：
//!
//! ```bash
//! cargo xtest                       # 宿主机纯逻辑测试 + QEMU 测试
//! cargo xtest host                  # 只跑宿主机测试
//! cargo xtest qemu                  # 只跑 QEMU 测试
//! cargo xtest host --filter path    # 按子串过滤宿主机测试
//! cargo xtest qemu --filter spin    # 按子串过滤 QEMU 测试
//! cargo xtest qemu --verbose        # 输出完整 OpenSBI 启动日志
//! cargo xtest all --release
//! ```
//!
//! 宿主机测试直接使用 Rust 内置 `#[test]`，通过 `--target <host>` 覆盖仓库默认的
//! RISC-V 目标。QEMU 测试构建 `goethite-test-kernel`，由内核里的 `goethite-test`
//! 框架执行并以 SiFive test finisher 退出。

use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const FALLBACK_HOST_TARGET: &str = "x86_64-unknown-linux-gnu";
const RISCV_TARGET: &str = "riscv64gc-unknown-none-elf";
const QEMU_KERNEL: &str = "goethite-test-kernel";
const HARNESS_BANNER: &str = "goethite test harness:";
const SUMMARY_PREFIX: &str = "GOETHITE_TEST_SUMMARY";

/// 可在宿主机上直接测试的 crate。
///
/// 这里不包含 `goethite-console` / `goethite-syscall` / `goethite-test-kernel`
/// 等依赖 RISC-V 汇编或 SBI 的 crate；它们由 QEMU 测试覆盖。
const HOST_CRATES: &[&str] = &[
    "goethite-abi",
    "goethite-bitmap",
    "goethite-config",
    "goethite-elf",
    "goethite-fs",
    "goethite-id-alloc",
    "goethite-intrusive-list",
    "goethite-libc",
    "goethite-librs",
    "goethite-macros",
    "goethite-macros-impl",
    "goethite-minix",
    "goethite-riscv64",
    "goethite-riscv64-asm",
    "goethite-stack",
    "goethite-sync",
    "goethite-test",
];

fn main() {
    let options = match Options::parse() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("error: {message}\n\n{}", Options::usage());
            std::process::exit(2);
        }
    };

    let ok = match options.mode {
        Mode::Host => run_host(&options),
        Mode::Qemu => run_qemu(&options),
        Mode::All => {
            let host = run_host(&options);
            let qemu = run_qemu(&options);
            host && qemu
        }
    };

    if !ok {
        std::process::exit(1);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Host,
    Qemu,
    All,
}

struct Options {
    mode: Mode,
    release: bool,
    verbose: bool,
    filter: Option<String>,
}

impl Options {
    fn usage() -> &'static str {
        "usage: cargo xtest [host|qemu|all] [--release] [--verbose|-v] [--filter <substring>]"
    }

    fn parse() -> Result<Self, String> {
        let mut args = env::args().skip(1);
        let mut mode = Mode::All;
        let mut release = false;
        let mut verbose = false;
        let mut filter = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "host" => mode = Mode::Host,
                "qemu" => mode = Mode::Qemu,
                "all" => mode = Mode::All,
                "--release" => release = true,
                "-v" | "--verbose" => verbose = true,
                "--filter" => {
                    filter = Some(
                        args.next()
                            .ok_or_else(|| "--filter requires a value".to_owned())?,
                    );
                }
                "-h" | "--help" => {
                    println!("{}", Self::usage());
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument `{other}`")),
            }
        }

        Ok(Self {
            mode,
            release,
            verbose,
            filter,
        })
    }
}

fn root_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must live directly under the repository root")
        .to_path_buf()
}

fn cargo() -> OsString {
    if let Some(val) = env::var_os("CARGO") {
        let path = Path::new(&val);
        // 仅当 CARGO 指向实际存在的可执行文件（或不含路径分隔符的命令名）时使用；
        // 某些环境中 cargo alias 注入的 CARGO 路径不可直接执行，回退到 PATH 中的 `cargo`。
        if path.components().count() == 1 || path.is_file() {
            return val;
        }
    }
    OsString::from("cargo")
}

fn host_target() -> String {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    if let Ok(output) = Command::new(rustc).arg("-vV").output()
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(triple) = line.strip_prefix("host:") {
                let triple = triple.trim();
                if !triple.is_empty() {
                    return triple.to_owned();
                }
            }
        }
    }
    FALLBACK_HOST_TARGET.to_owned()
}

fn run_host(options: &Options) -> bool {
    let host = host_target();
    println!("==> host tests (target: {host})");

    let mut command = Command::new(cargo());
    command
        .current_dir(root_dir())
        .arg("test")
        .arg("--target")
        .arg(&host)
        .arg("--tests")
        .arg("--no-fail-fast")
        // 宿主机测试需要 `goethite-stack` 的分配版接口（`Stack`）。
        .arg("--features")
        .arg("goethite-stack/alloc");

    if options.release {
        command.arg("--release");
    }
    for package in HOST_CRATES {
        command.arg("-p").arg(package);
    }
    if let Some(filter) = &options.filter {
        command.arg(filter);
    }

    match command.status() {
        Ok(status) if status.success() => {
            println!("==> host tests: PASSED");
            true
        }
        Ok(status) => {
            eprintln!("==> host tests: FAILED ({status})");
            false
        }
        Err(error) => {
            eprintln!("==> host tests: could not run cargo: {error}");
            false
        }
    }
}

fn run_qemu(options: &Options) -> bool {
    println!("==> QEMU tests (target: {RISCV_TARGET})");

    let mut build = Command::new(cargo());
    build
        .current_dir(root_dir())
        .arg("build")
        .arg("--target")
        .arg(RISCV_TARGET)
        .arg("-p")
        .arg(QEMU_KERNEL);

    if options.release {
        build.arg("--release");
    }
    match &options.filter {
        Some(filter) => {
            build.env("GOETHITE_TEST_FILTER", filter);
        }
        None => {
            build.env_remove("GOETHITE_TEST_FILTER");
        }
    }

    let build_status = match build.status() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("==> QEMU tests: could not run cargo: {error}");
            return false;
        }
    };
    if !build_status.success() {
        eprintln!("==> QEMU tests: build FAILED");
        return false;
    }

    let profile = if options.release { "release" } else { "debug" };
    let kernel = root_dir()
        .join("target")
        .join(RISCV_TARGET)
        .join(profile)
        .join(QEMU_KERNEL);
    if !kernel.exists() {
        eprintln!("==> QEMU tests: kernel not found at {}", kernel.display());
        return false;
    }

    let qemu = env::var_os("QEMU").unwrap_or_else(|| OsString::from("qemu-system-riscv64"));
    let timeout = env::var("GOETHITE_QEMU_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30));

    let log_path = root_dir().join("target").join("goethite-qemu-test.log");
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let log = match File::create(&log_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("==> QEMU tests: cannot create log file: {error}");
            return false;
        }
    };
    let stderr = match log.try_clone() {
        Ok(file) => file,
        Err(error) => {
            eprintln!("==> QEMU tests: cannot duplicate log handle: {error}");
            return false;
        }
    };

    let mut child = match Command::new(&qemu)
        .current_dir(root_dir())
        .arg("-machine")
        .arg("virt")
        .arg("-m")
        .arg("128M")
        .arg("-smp")
        .arg("1")
        .arg("-nographic")
        .arg("-bios")
        .arg("default")
        .arg("-kernel")
        .arg(&kernel)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!(
                "==> QEMU tests: cannot start {}: {error}\n\
                 hint: install qemu-system-riscv64 or set the QEMU environment variable",
                Path::new(&qemu).display()
            );
            return false;
        }
    };

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= timeout => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                eprintln!("==> QEMU tests: waiting for QEMU failed: {error}");
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    };

    let output = fs::read(&log_path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();

    let display_output = if options.verbose {
        output.as_str()
    } else if let Some(idx) = output.find(HARNESS_BANNER) {
        &output[idx..]
    } else {
        output.as_str()
    };
    print!("{display_output}");
    if !display_output.ends_with('\n') && !display_output.is_empty() {
        println!();
    }

    if timed_out {
        eprintln!(
            "==> QEMU tests: TIMEOUT after {}s (log: {})",
            timeout.as_secs(),
            log_path.display()
        );
        return false;
    }

    let summary = output
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(SUMMARY_PREFIX));

    let Some(summary) = summary else {
        eprintln!(
            "==> QEMU tests: no `{SUMMARY_PREFIX}` line; the kernel likely crashed early (log: {})",
            log_path.display()
        );
        return false;
    };
    let Some((passed, failed, ignored)) = parse_summary(summary) else {
        eprintln!("==> QEMU tests: malformed summary line: {summary}");
        return false;
    };
    match status {
        Some(status) if status.success() && failed == 0 => {
            println!("==> QEMU tests: PASSED ({passed} passed, {failed} failed, {ignored} ignored)");
            true
        }
        Some(status) => {
            eprintln!(
                "==> QEMU tests: FAILED ({status}; {passed} passed, {failed} failed, {ignored} ignored)"
            );
            false
        }
        None => false,
    }
}

/// 从 `GOETHITE_TEST_SUMMARY passed=N failed=M [ignored=K]` 中解析计数。
fn parse_summary(summary: &str) -> Option<(usize, usize, usize)> {
    let mut passed = None;
    let mut failed = None;
    let mut ignored = 0usize;
    for field in summary.split_whitespace() {
        if let Some(value) = field.strip_prefix("passed=") {
            passed = value.parse().ok();
        } else if let Some(value) = field.strip_prefix("failed=") {
            failed = value.parse().ok();
        } else if let Some(value) = field.strip_prefix("ignored=") {
            ignored = value.parse().ok()?;
        }
    }
    Some((passed?, failed?, ignored))
}

#[cfg(test)]
mod tests {
    use super::parse_summary;

    #[test]
    fn parses_summary_counts() {
        assert_eq!(
            parse_summary("GOETHITE_TEST_SUMMARY passed=11 failed=0"),
            Some((11, 0, 0))
        );
        assert_eq!(
            parse_summary("GOETHITE_TEST_SUMMARY passed=11 failed=0 ignored=2"),
            Some((11, 0, 2))
        );
        assert_eq!(
            parse_summary("GOETHITE_TEST_SUMMARY passed=0 failed=3 ignored=1"),
            Some((0, 3, 1))
        );
        assert_eq!(parse_summary("GOETHITE_TEST_SUMMARY passed=1"), None);
        assert_eq!(
            parse_summary("GOETHITE_TEST_SUMMARY passed=x failed=0"),
            None
        );
        assert_eq!(
            parse_summary("GOETHITE_TEST_SUMMARY passed=1 failed=0 ignored=bad"),
            None
        );
    }
}
