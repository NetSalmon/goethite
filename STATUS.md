# 开发状态与接续路线

更新：2026-10-18；代码基线：`master` / `5a68435`。这是该提交的状态快照。

## 当前阶段

内核启动、内存管理、设备探测以及块设备 → MINIX → VFS 读取链路已经接通。下一阶段是启动第一个用户程序。ELF 加载、上下文恢复和调度结构已有实现，但尚未由启动流程串联。

当前启动路径：`boot` → `main` → 伙伴系统 → 页表 → 设备 → PLIC → 根挂载 → 读取 `/hello` → 内核断点 → 自旋。

## 模块进度

| 范围 | 已有实现 | 当前边界 |
| --- | --- | --- |
| 内存 | Sv39 高半区映射、伙伴系统、SLUB、地址空间、VMA | 启动通过；用户地址空间切换、缺页处理未做运行验证 |
| 设备 | 设备树、驱动注册、UART、virtio-blk/RNG、PLIC；另有 fw_cfg/ramfb/reboot 模块 | 启动注册 UART、块设备、RNG；本次验证块读取，未验证全部驱动和中断收发 |
| 文件系统 | VFS、MINIX v1、根挂载、内核 read_file；MINIX 有写入和分配实现 | 本次只验证读取；进程 FileDesc 仍为空结构 |
| 用户程序 | ELF 段解析/装载/BSS 清零、用户运行时、hello 示例、栈映像工具 | 启动未调用 load_elf，尚未组装用户栈和任务 |
| 调度 | 任务状态、运行队列、上下文保存/恢复、Scheduler::restore | schedule() 为空；主入口未创建或切入任务，定时抢占未接通 |
| 系统调用 | ABI、用户侧 write、内核 write 的局部实现 | U_MODE_ECALL 为 todo!()；分派未接陷阱，未回填返回值 |
| 进程接口 | open/close/read/stat/fork/execve 接口轮廓 | 内核实现仍是 todo!()，ABI 默认方法多返回 ENOSYS |

## 本次验证

- `cargo check --workspace`、`cargo build --workspace` 均通过，包括用户示例链接。
- 环境：`rustc 1.100.0-nightly (67854e511 2026-08-15)`；已安装 QEMU。仓库未固定工具链版本。
- 构建存在警告（内核 11 条，主要为未使用变量/导入），本次未修改实现。
- QEMU 单 hart、128 MiB、默认 OpenSBI、现代 virtio-mmio：观察到如下输出，随后在预设 8 秒超时后终止。

```text
plic: 95 interrupt sources at 0xc000000, supervisor context 1
mounted minix v1 root filesystem
/hello: hello world
breakpoint at ...
```

这些结果证明启动和文件读取链路可运行；用户态、抢占调度、文件系统写入尚未据此验证。自动化测试覆盖见下方“测试框架增量”。

## 建议接续顺序

1. **接通 ecall 最小闭环**：在 `src/trap.rs` 分派系统调用，将返回值或负 errno 写回 a0，并让 ecall 的 sepc 前进 4 字节；未知调用返回 ENOSYS。核对用户侧汇编的寄存器输入/输出约束，并处理用户缓冲区的校验与访问权限。
2. **启动首个用户任务**：经 VFS 读 ELF → load_elf → 用户栈 → Context/Task → 入队并切入。栈布局须匹配 `goethite-userland::_start` 的栈映像指针约定；目标页表需保留内核映射；避免跨越不返回的 restore 永久持有调度锁。
3. **建立用户态验收**：保留文本 `/hello` 作为现有冒烟，将 ELF 命名为 `/init`。验收目标为用户态输出 `Hello, world!`，并完成系统调用返回。示例当前 main 返回后自旋，后续再补 exit。
4. **补任务生命周期和调度**：实现 exit、任务移除和资源回收，再接定时器和多任务。当前 schedule()、Todo::Yield 尚未形成完整切换路径。
5. **扩展文件与进程接口**：实现进程 fd 表、open/read/close，再推进 execve、fork、写入验证。VMA 已有缺页方法，但 trap 遇加载/存储缺页仍直接 panic，需要接线。

## 代码入口

- `src/main.rs`、`src/init.rs`：启动与现有冒烟检查。
- `src/trap.rs`、`src/trap/entry.rs`、`src/syscall.rs`：陷阱与系统调用。
- `src/proc/exec.rs`、`src/proc/vma.rs`、`src/mm/address_space.rs`：ELF、内存与页表。
- `src/proc.rs`、`src/proc/schedule.rs`：任务、上下文和队列。
- `crates/goethite-abi`、`crates/goethite-syscall`、`goethite-userland`、`crates/goethite-stack`：ABI、用户运行时和栈约定。
- `src/fs.rs`、`crates/goethite-fs`、`crates/goethite-minix`：文件系统。

## 复现启动

注意：`workspace/` 整体被 Git 忽略。本机的 `workspace/mkminix.py` 和磁盘镜像未纳入版本控制，新克隆没有这些文件。后续应将生成器移至受版本管理的 `scripts/`，继续忽略生成镜像。

当前工作目录可执行以下命令（新建临时镜像，不覆盖开发镜像）：

```bash
cargo check --workspace
cargo build --workspace
python3 workspace/mkminix.py /tmp/goethite-status-minix.img
timeout 8s qemu-system-riscv64 \
  -machine virt -m 128M -smp 1 -nographic -bios default \
  -kernel target/riscv64gc-unknown-none-elf/debug/goethite \
  -global virtio-mmio.force-legacy=false \
  -drive file=/tmp/goethite-status-minix.img,format=raw,if=none,id=rootfs \
  -device virtio-blk-device,drive=rootfs
```

内核当前不会自动退出，timeout 的退出码 124 属于预期，仍应检查串口是否达到上述检查点。

## 工程待办

- README 提及的 fork/execve 等属于目标，实际进度参见本文和代码。
- 固定可用工具链，纳管镜像生成器，补可重复的 QEMU 冒烟脚本。
- 后续实现变化时更新本状态快照。

## AArch64 分支增量（2026-11-09）

新增独立 AArch64 早期启动路径：`src/aarch64/` 与
`src/link-scripts/aarch64.ld`。`src/main.rs` 按架构选择入口，原 RISC-V
入口移至 `src/riscv64.rs`；根包的 RISC-V 依赖仅在对应目标启用。

QEMU virt 已验证单核 EL1 直接启动和 EL2→EL1 启动，包含 Image 头、
BSS/栈、串口输出、FDT 内存读取和致命异常处理入口。当前 MMU 关闭，
中断屏蔽，尚未接入通用内核子系统；异常向量已链接，尚未执行故障注入测试。
验证脚本：`python3 scripts/smoke-aarch64.py`（可附加 `--release`）。
详细命令、入口约束与后续工作见 README 的 AArch64 节。

本次结果：AArch64 debug/release 均在 Cortex-A53、Cortex-A72 和 max 上达到
ready 标记，发布版额外核对固件入口 EL1/EL2；`cargo build --workspace`
通过（保留现有未使用项警告）。RISC-V 使用本机 MINIX 镜像的快照启动，
达到根文件系统挂载及原有 breakpoint；该镜像的 `/hello` 为二进制内容。

## 测试框架增量（2026-11-18）

新增统一测试入口 `cargo xtest`（`xtask/` 独立 workspace，别名见
`.cargo/config.toml`），按运行环境分成两层：

- **宿主机层**：纯逻辑 crate 的 `tests/*.rs` 使用标准 `#[test]`，在
  `x86_64-unknown-linux-gnu` 上运行。已覆盖 `goethite-abi`、`goethite-bitmap`、
  `goethite-config`、`goethite-elf`、`goethite-fs`、`goethite-id-alloc`、
  `goethite-intrusive-list`、`goethite-macros`、`goethite-macros-impl`、
  `goethite-minix`、`goethite-riscv64`、`goethite-riscv64-asm`、
  `goethite-stack`、`goethite-sync`。为支持宿主机编译，`goethite-riscv64`
  在非 RISC-V 目标下提供寄存器 / CSR 数值桩。
- **QEMU 层**：`goethite-test`（`no_std` 测试框架）负责注册、断言和汇总；
  `goethite-test-kernel` 在 QEMU virt 上运行 `goethite-riscv64`、
  `goethite-sync`、`goethite-stdio` 的 `qemu-tests` 套件，经 SiFive test
  finisher 退出并返回真实退出码。汇总行格式为
  `GOETHITE_TEST_SUMMARY passed=N failed=M`。

当前验证结果：`cargo xtest host` 与 `cargo xtest qemu` 均通过（宿主机 14 个
crate、QEMU 11 项测试）；`cargo check --workspace` 在默认 RISC-V 目标下通过。
`goethite-syscall` / `goethite-userland` 需要陷阱或用户态执行环境，尚未接入
自动化测试，是后续建立用户态验收时的一并工作。使用方法见 README 的“测试”节。
