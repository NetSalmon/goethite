fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    println!("cargo:rustc-link-arg=-T{manifest_dir}/link.ld");
    println!("cargo:rerun-if-changed=link.ld");
    println!("cargo:rerun-if-changed=src/boot.s");
    println!("cargo:rerun-if-env-changed=GOETHITE_TEST_FILTER");
}
