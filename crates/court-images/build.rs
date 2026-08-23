fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-changed={dir}/image.ld");
    println!("cargo:rustc-link-arg=-T{dir}/image.ld");
    println!("cargo:rustc-link-arg=--nmagic");
}
