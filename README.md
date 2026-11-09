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
| `goethite-stdio`          | 内核控制台输出与 `print` 系列宏（经 SBI 调试控制台）                     |
| `goethite-elf`            | ELF64 解析（文件头 / 程序头，供用户程序加载）                            |
| `goethite-stack`          | 进程栈映像（argv / env / auxv）的零拷贝解析与序列化                      |
| `goethite-abi`        | 系统调用接口雏形（`PosixErrno` / `Syscall` 形状）                             |

### 依赖关系（上游 → 下游）

底层 crate（`goethite-riscv64`、`goethite-intrusive-list`、`goethite-id-alloc`、`goethite-macros-impl`）
几乎无依赖；`goethite-sync` / `goethite-config` / `goethite-stdio` 依赖底层；内核本体 `goethite`
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
用户态由 `goethite-userland/build.rs` 指定
`goethite-userland/src/link-scripts/userland.ld`（`ENTRY(_start)`），
避免全局 rustflags 把内核脚本误用于用户程序：

```bash
rustup target add riscv64gc-unknown-none-elf
cargo build                # 或 cargo check 做编译期验证
```

也可仅对单个 crate 检查：`cargo check -p goethite-sync` 等。

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
  `ASpaceError`、`MemsetError`、`DeviceError`），并在 `From` 转换间向上传播，避免丢失语义。
- **类型安全**：用 `bits!` / `numeric!` 生成位域与整数新类型；用 typestate（`PageObject<T, Uninit/Init>`）
  在编译期保证未初始化内存不被读取。
- **最小依赖**：尽量复用 workspace 内已有 crate，不为单点功能新增第三方依赖。

## 许可

GPL-3.0-or-later（见 `LICENSE`）。第三方许可清单由 `about.toml` / `licenses.html` 维护。
