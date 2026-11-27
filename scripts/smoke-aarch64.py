#!/usr/bin/env python3
"""Build and boot the AArch64 kernel on QEMU virt; no guest disk needed."""
import pathlib
import struct
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
RELEASE = "--release" in sys.argv[1:]
PROFILE = "release" if RELEASE else "debug"
OUT = ROOT / "target" / "aarch64-unknown-none" / PROFILE


def main():
    subprocess.run(
        ["cargo", "build", "-p", "goethite", "--target", "aarch64-unknown-none"]
        + (["--release"] if RELEASE else []), cwd=ROOT, check=True,
    )
    # GNU objcopy's generic little-endian ELF reader also works on hosts whose
    # binutils package has no named AArch64 backend.
    subprocess.run(["objcopy", "-I", "elf64-little", "-O", "binary",
                    str(OUT / "goethite"), str(OUT / "Image")], check=True)
    header = (OUT / "Image").read_bytes()[:64]
    assert header[56:60] == b"ARM\x64", "invalid Image magic"
    offset, image_size = struct.unpack_from("<QQ", header, 8)
    assert offset == 0x80000 and image_size >= (OUT / "Image").stat().st_size
    for cpu, virtualization, ram in [
        ("cortex-a53", "off", "128M"),
        ("cortex-a72", "on", "256M"),
        ("max", "on", "512M"),
    ]:
        command = ["qemu-system-aarch64", "-machine", f"virt,virtualization={virtualization}",
                   "-cpu", cpu, "-m", ram, "-smp", "1", "-nographic", "-monitor", "none",
                   "-kernel", str(OUT / "Image")]
        try:
            result = subprocess.run(command, capture_output=True, timeout=5)
        except subprocess.TimeoutExpired as exc:
            output = ((exc.stdout or b"") + (exc.stderr or b"")).decode(errors="replace")
        else:
            raise RuntimeError(f"QEMU exited unexpectedly: {result.returncode}\n{result.stderr.decode()}")
        entry_el = 2 if virtualization == "on" else 1
        for marker in [f"Firmware entry: EL{entry_el}", "AArch64 boot at EL1", "RAM: 0x40000000", "DTB:",
                       "AArch64 early boot ready"]:
            assert marker in output, f"{cpu}: missing {marker!r}\n{output}"
        assert "panic" not in output and "exception:" not in output, output
        print(f"PASS {cpu}, virtualization={virtualization}, RAM={ram}")
        print(output.strip())


if __name__ == "__main__":
    main()
