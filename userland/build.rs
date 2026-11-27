fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    println!("cargo:rustc-link-arg-bins=-T{manifest_dir}/src/link-scripts/userland.ld");
    println!("cargo:rerun-if-changed=src/link-scripts/userland.ld");
}
