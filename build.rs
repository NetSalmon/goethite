fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let script = match arch.as_str() {
        "riscv64" => "src/link-scripts/linker.ld",
        _ => panic!("unsupported kernel architecture: {arch}"),
    };
    println!("cargo:rustc-link-arg-bin=goethite=-T{manifest_dir}/{script}");
    println!("cargo:rerun-if-changed={script}");
}
