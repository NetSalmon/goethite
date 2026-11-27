#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="riscv64gc-unknown-none-elf"
PROFILE="debug"
BUILD=1
REBUILD_IMG=0
GDB=0
SNAPSHOT=0
TIMEOUT=""
IMG="$ROOT/workspace/minix.img"
QEMU="${QEMU:-qemu-system-riscv64}"
MEM="${MEM:-128M}"
SMP="${SMP:-1}"
EXTRA_QEMU_ARGS=()

usage() {
    cat <<'EOF'
用法: scripts/run.sh [选项] [-- <QEMU 额外参数>]

选项:
  --release         使用 release 构建并启动
  --no-build        跳过 cargo build
  --rebuild-img     重新编译 userland 并重新生成 MINIX 根文件系统镜像
  --img <路径>      指定根文件系统镜像（默认: workspace/minix.img）
  --snapshot        以 QEMU snapshot=on 挂载镜像（不写回磁盘）
  --gdb             开启 GDB 调试桩（-s -S，监听 tcp::1234）
  --timeout <秒>    限时运行 QEMU（超时退出）
  -h, --help        显示帮助

环境变量:
  QEMU              指定 QEMU 可执行文件（默认: qemu-system-riscv64）
  MEM               内存大小（默认: 128M）
  SMP               CPU 核心数（默认: 1）
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release)
            PROFILE="release"
            shift
            ;;
        --no-build)
            BUILD=0
            shift
            ;;
        --rebuild-img)
            REBUILD_IMG=1
            shift
            ;;
        --img)
            IMG="${2:?--img 需要参数}"
            shift 2
            ;;
        --snapshot)
            SNAPSHOT=1
            shift
            ;;
        --gdb)
            GDB=1
            shift
            ;;
        --timeout)
            TIMEOUT="${2:?--timeout 需要参数}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            EXTRA_QEMU_ARGS+=("$@")
            break
            ;;
        *)
            echo "未知参数: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

CARGO_ARGS=(--target "$TARGET")
if [[ "$PROFILE" == "release" ]]; then
    CARGO_ARGS+=(--release)
fi

if [[ "$BUILD" -eq 1 ]]; then
    echo "==> 构建内核 goethite ($PROFILE)"
    cargo build -p goethite "${CARGO_ARGS[@]}" --manifest-path "$ROOT/Cargo.toml"
fi

if [[ "$REBUILD_IMG" -eq 1 || ! -f "$IMG" ]]; then
    MKMINIX="$ROOT/workspace/mkminix.py"
    if [[ ! -f "$MKMINIX" ]]; then
        echo "错误: 找不到镜像 $IMG，且不存在生成器 $MKMINIX" >&2
        exit 1
    fi
    echo "==> 构建 userland 并生成根文件系统镜像: $IMG"
    cargo build -p goethite-userland --bins "${CARGO_ARGS[@]}" --manifest-path "$ROOT/Cargo.toml"
    BIN_DIR="$ROOT/target/$TARGET/$PROFILE"
    STAGE_DIR="$(mktemp -d)"
    trap 'rm -rf "$STAGE_DIR"' EXIT
    OBJCOPY="$(find "$(rustc --print sysroot)" -name llvm-objcopy 2>/dev/null | head -n 1 || true)"
    BINS=(init sh ls cat echo hello)
    PACK_ARGS=()
    for name in "${BINS[@]}"; do
        if [[ -n "$OBJCOPY" ]]; then
            "$OBJCOPY" --strip-all "$BIN_DIR/$name" "$STAGE_DIR/$name"
            PACK_ARGS+=("$name=$STAGE_DIR/$name")
        else
            PACK_ARGS+=("$name=$BIN_DIR/$name")
        fi
    done
    printf 'Welcome to goethite OS!\n' > "$STAGE_DIR/motd"
    PACK_ARGS+=("motd=$STAGE_DIR/motd")
    mkdir -p "$(dirname "$IMG")"
    python3 "$MKMINIX" "$IMG" "${PACK_ARGS[@]}"
fi

KERNEL="$ROOT/target/$TARGET/$PROFILE/goethite"
if [[ ! -f "$KERNEL" ]]; then
    echo "错误: 找不到内核 ELF: $KERNEL" >&2
    exit 1
fi

DRIVE_OPT="file=$IMG,format=raw,if=none,id=rootfs"
if [[ "$SNAPSHOT" -eq 1 ]]; then
    DRIVE_OPT+=",snapshot=on"
fi

CMD=(
    "$QEMU"
    -machine virt
    -m "$MEM"
    -smp "$SMP"
    -nographic
    -bios default
    -kernel "$KERNEL"
    -global virtio-mmio.force-legacy=false
    -drive "$DRIVE_OPT"
    -device virtio-blk-device,drive=rootfs
)

if [[ "$GDB" -eq 1 ]]; then
    CMD+=(-s -S)
fi

if [[ ${#EXTRA_QEMU_ARGS[@]} -gt 0 ]]; then
    CMD+=("${EXTRA_QEMU_ARGS[@]}")
fi

echo "==> 启动 QEMU: ${CMD[*]}"
if [[ -n "$TIMEOUT" ]]; then
    exec timeout "${TIMEOUT}s" "${CMD[@]}"
else
    exec "${CMD[@]}"
fi
