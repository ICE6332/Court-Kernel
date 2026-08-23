use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let linker = manifest_dir.join("linker.ld");
    println!("cargo:rerun-if-changed={}", linker.display());
    println!("cargo:rustc-link-arg=-T{}", linker.display());

    let workspace = manifest_dir.parent().unwrap().parent().unwrap();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let images_target = out.join("court-images-target");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    // x86_64-unknown-none defaults to the kernel code model. Court Images
    // live in the lower half, so the nested build must override that.
    let status = Command::new(&cargo)
        .current_dir(workspace)
        .env("CARGO_TARGET_DIR", &images_target)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        // Config rustflags still inject kernel code-model; RUSTFLAGS is appended
        // and the last -C code-model wins, which must be small for lower-half images.
        .env(
            "RUSTFLAGS",
            "-C relocation-model=static -C code-model=small -C panic=abort -C strip=symbols",
        )
        .args([
            "build",
            "-p",
            "court-images",
            "--release",
            "--target",
            "x86_64-unknown-none",
        ])
        .status()
        .expect("failed to spawn court-images build");
    if !status.success() {
        panic!("court-images build failed");
    }

    let bin_dir = images_target.join("x86_64-unknown-none/release");
    for name in ["court-image-app", "court-image-net"] {
        let src = bin_dir.join(name);
        let dst = out.join(name);
        std::fs::copy(&src, &dst)
            .unwrap_or_else(|err| panic!("copy {} -> {}: {err}", src.display(), dst.display()));
        println!("cargo:rerun-if-changed={}", src.display());
    }
    println!(
        "cargo:rerun-if-changed={}/crates/court-images",
        workspace.display()
    );
    println!(
        "cargo:rerun-if-changed={}/crates/court-abi",
        workspace.display()
    );
}
