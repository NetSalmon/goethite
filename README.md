# goethite

当前开发进度、验证结果和接续路线见 [STATUS.md](STATUS.md)。

面向 RISC-V 64（`riscv64gc`）的宏内核，采用 `#![no_std]` / `#![no_main]` 设计，可运行于
QEMU（OpenSBI 之上）。目标是 rCore / uCore 风格的**单体式教学内核**：含 MINIX v1 文件系统、
virtio 驱动、ELF 加载、进程调度（fork / execve）等，当前覆盖单 CPU。

## 架构概览

内核本体是一个 Cargo workspace，主体是 `src/`，并拆分为多个职责单一的外围 crate（`crates/`），
保证**高内聚、低耦合**——底层硬件 / 同步原语 / 数据结构被尽可能独立出来，便于单独验证与复用。

### 顶层：`src/`（内核本体，bin crate `goethite`）

| 模块      | 职责                                                                        |
|-----------|-----------------------------------------------------------------------------|
| `main.rs` | 内核入口 `__kernel_entry` / `main`，全局错误类型 `KernelError`，panic 处理  |
| `boot.rs` | 启动阶段：低地址建立 Sv39 页表、恒等映射并跳转到高地址内核                  |
| `arch.rs` | 架构相关辅助（占位，具体见 `goethite-riscv64`）                             |
| `mm/`     | 内存管理：页表、伙伴系统、SLUB 堆分配器、地址空间、物理块 (`Region`)        |
| `device/` | 设备子系统：设备树解析 (`desc`)、驱动注册表 (`driver`)、设备抽象 (`Device`) |
| `proc/`   | 进程 / 线程：id 分配、上下文、虚拟内存区域 (`vma`)、ELF 加载 (`exec`)       |

### 外围 crate：`crates/`

| crate                     | 职责                                                                     |
|---------------------------|--------------------------------------------------------------------------|
| `goethite-riscv64`        | RISC-V 硬件抽象：地址类型、CSR / 寄存器、SBI 封装                        |
| `goethite-config`         | 编译期常量汇总（CPU / 文件系统 / 内存布局 / 链接符号 / 任务 / 版本）     |
| `goethite-sync`           | 同步原语：`LazyLock`、`OnceLock`、`PerCpu`、`RwLock`、`SpinLock`         |
| `goethite-macros`         | 通用宏与编译期常量辅助（`bits` / `numeric` / `lazy` / `load_symbol` 等） |
| `goethite-macros-impl`    | `goethite-macros` 的过程宏实现（`proc-macro = true`，不导出普通项）      |
| `goethite-id-alloc`       | 两级 ID 分配器（类 `dev_t` 的 major/minor 模型）                         |
| `goethite-intrusive-list` | 侵入式单链表（伙伴系统 / SLUB 空闲链表复用）                             |
| `goethite-console`        | 内核控制台输出与 `print` 系列宏（经 SBI 调试控制台）                     |
| `goethite-elf`            | ELF64 解析（文件头 / 程序头，供用户程序加载）                            |
| `goethite-stack`          | 进程栈映像（argv / env / auxv）的零拷贝解析与序列化                      |
| `goethite-abi`        | 系统调用接口雏形（`PosixErrno` / `Syscall` 形状）                             |

### 依赖关系（上游 → 下游）

底层 crate（`goethite-riscv64`、`goethite-intrusive-list`、`goethite-id-alloc`、`goethite-macros-impl`）
几乎无依赖；`goethite-sync` / `goethite-config` / `goethite-console` 依赖底层；内核本体 `goethite`
依赖上述全部。`goethite-macros` 与 `goethite-macros-impl` 是「重新导出 + 过程宏」的两段式拆分
（过程宏 crate 无法导出普通项），二者协作提供 `bits!` 等宏。

## 目录结构

```text
.
├── Cargo.toml            # workspace 根：bin crate + 成员声明
├── src/                  # 内核本体
│   ├── main.rs
│   ├── boot.rs
│   ├── arch.rs
│   ├── device/  mm/  proc/
│   └── link-scripts/linker.ld   # 链接脚本（低地址 + 高地址段映射）
├── crates/               # 外围 crate（见上表）
└── workspace/            # 调试用资源（如 QEMU 设备树 qemu.dtb / qemu.dts）
```

## 构建

目标为 `riscv64gc-unknown-none-elf`（见 `.cargo/config.toml`）。链接脚本按包注入：
内核由根 `build.rs` 指定 `src/link-scripts/linker.ld`（`ENTRY(_boot)`），
用户态由 `userland/build.rs` 指定
`userland/src/link-scripts/userland.ld`（`ENTRY(_start)`），
避免全局 rustflags 把内核脚本误用于用户程序：

```bash
rustup target add riscv64gc-unknown-none-elf
cargo build                # 或 cargo check 做编译期验证
```

也可仅对单个 crate 检查：`cargo check -p goethite-sync` 等。

## 测试

测试分两层，统一由 `xtask` 驱动（`cargo xtest` 别名见 `.cargo/config.toml`）：

| 命令 | 内容 |
|------|------|
| `cargo xtest` / `cargo xtest all` | 宿主机测试 + QEMU 测试 |
| `cargo xtest host` | 只跑宿主机纯逻辑测试（`x86_64-unknown-linux-gnu`） |
| `cargo xtest qemu` | 只跑 QEMU 硬件相关测试（`riscv64gc-unknown-none-elf`） |
| `cargo xtest host --filter <子串>` | 按名字过滤宿主机测试 |
| `cargo xtest --release` | 使用 release 构建 |

- **纯逻辑 crate**：在 `crates/<crate>/tests/` 下写普通 `#[test]`。`xtask` 用
  `--target x86_64-unknown-linux-gnu` 覆盖仓库默认的 RISC-V 目标，测试直接在本机运行。
- **硬件相关 crate**：在 crate 中增加 `qemu-tests` feature 与对应的测试模块，用
  `goethite_test::test_suite!` 注册测试；`goethite-test-kernel` 汇总各 crate 的套件，
  在 QEMU 中依次执行。目前覆盖 `goethite-riscv64`（CSR / trap / SBI）、
  `goethite-sync`（自旋锁 / 原子操作）与 `goethite-console`（控制台输出）。
- 测试内核通过 QEMU virt 的 SiFive test finisher 退出，`xtask` 依据退出码和
  `GOETHITE_TEST_SUMMARY passed=N failed=M` 行判定结果。日志写入
  `target/goethite-qemu-test.log`，超时可用 `GOETHITE_QEMU_TIMEOUT=<秒>` 调整。
- QEMU 测试需要 `qemu-system-riscv64`；可用 `QEMU` 环境变量指定其他可执行文件。

新增宿主机测试后，将其 crate 加入 `xtask/src/main.rs` 的 `HOST_CRATES`；新增 QEMU
测试套件则在 `goethite-test-kernel` 的 `rust_main` 中登记。`goethite-syscall` 与
`goethite-userland` 依赖陷阱或用户态执行环境，暂未纳入这两层测试。

## 运行（QEMU）

内核从 `0x8020_0000` 加载（OpenSBI / QEMU），启动汇编在 `.text.low` 建立页表并跳转至高地址。
典型启动命令（需 `qemu-system-riscv64` 与 OpenSBI 固件）：

```bash
qemu-system-riscv64 -machine virt -nographic \
  -bios <opensbi_fw_payload>.bin \
  -kernel target/riscv64gc-unknown-none-elf/debug/goethite \
  -append "console=ttyS0"
```

调试时可用 `rust-objdump` 反汇编内核 ELF，结合 QEMU 的 `-d int` 追踪定位缺陷。

## 设计与错误处理约定

- **分阶段启动**：低地址（MMU 关闭）与高地址（Sv39 开启）分段，见 `boot.rs` 与链接脚本。
- **错误类型**：各子系统用 `thiserror` 定义可组合的错误枚举（`KernelError`、`AllocError`、
  `AddressSpaceError`、`MemorySetError`、`DeviceError`），并在 `From` 转换间向上传播，避免丢失语义。
- **类型安全**：用 `bits!` / `numeric!` 生成位域与整数新类型；用 typestate（`PageObject<T, Uninit/Init>`）
  在编译期保证未初始化内存不被读取。
- **最小依赖**：尽量复用 workspace 内已有 crate，不为单点功能新增第三方依赖。

## 许可

GPL-3.0-or-later（见 `LICENSE`）。第三方许可清单由 `about.toml` / `licenses.html` 维护。

## AArch64 适配分支（早期启动）

`codex/modern-arm-support` 新增 `aarch64-unknown-none` 内核构建，当前支持
**QEMU virt 的单 CPU 早期启动**。原有 RISC-V 内核仍为默认构建目标。
AArch64 已实现 ARM64 Image 头、EL1 直接入口 / EL2 降至 EL1、栈和 BSS 初始化、
致命异常向量、PL011 串口，以及从设备树读取内存范围。

```bash
rustup target add aarch64-unknown-none
cargo build -p goethite --target aarch64-unknown-none
objcopy -I elf64-little -O binary \
  target/aarch64-unknown-none/debug/goethite \
  target/aarch64-unknown-none/debug/Image
qemu-system-aarch64 -machine virt,virtualization=on -cpu cortex-a72 \
  -m 256M -smp 1 -nographic -kernel target/aarch64-unknown-none/debug/Image
```

需安装 QEMU ARM 系统模拟器、GNU objcopy 和 Python 3。运行
`python3 scripts/smoke-aarch64.py` 自动构建并验证 Cortex-A53、Cortex-A72、`max`
三种 CPU 与 EL1/EL2 入口；可添加 `--release` 验证发布构建。
验收标记为 `AArch64 early boot ready`，随后内核在中断关闭状态下等待。

必须传入转换后的 **Image**：QEMU 的裸 ELF 启动不使用相同的 DTB 寄存器协议。
本阶段遵循 [ARM64 启动协议](https://www.kernel.org/doc/html/v6.16/arch/arm64/booting.html)
的 DTB / MMU 入口约定，平台布局参照
[QEMU virt 文档](https://www.qemu.org/docs/master/system/arm/virt)。
链接地址固定为 `0x40080000`，早期串口固定为 `0x09000000`；引导方须提供
有效 DTB、关闭 MMU 和数据缓存，并只启动一个 CPU，入口须为非安全 EL1 或 EL2。
不支持 EL3 直接入口、UEFI PE 加载、任意地址重定位或直接刷入手机/开发板。

AArch64 尚未接入页表/堆、GIC/定时器、virtio、文件系统、调度和用户态；
现有 RISC-V 子系统不能据此视为已移植。ARM 构建请使用 `-p goethite`，
不要使用 `--workspace`（外围硬件和用户态 crate 目前仍依赖 RISC-V）。
后续依次接入 AArch64 MMU 与内存管理、GICv3 与通用定时器，再复用设备和进程层；
真机支持需按具体 SoC、固件入口及设备树另行适配。
