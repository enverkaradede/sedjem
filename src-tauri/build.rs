use std::path::PathBuf;
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        link_clang_runtime();
    }
    tauri_build::build();
}

/// whisper.cpp's Metal backend (ggml-metal-device.m) uses `@available`, which
/// compiles to calls into `___isPlatformVersionAtLeast` from clang's runtime.
/// rustc links with `-nodefaultlibs`; debug builds happen to resolve the
/// symbol from std, but release LTO internalises std's copy and the link
/// fails. Linking the static clang runtime makes the dependency explicit and
/// LTO-independent (an archive member is only pulled in when still needed).
fn link_clang_runtime() {
    let output = Command::new("xcrun")
        .args(["clang", "--print-resource-dir"])
        .output();
    let resource_dir = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        Ok(out) => panic!(
            "xcrun clang --print-resource-dir failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(err) => panic!("could not run xcrun to locate the clang runtime: {err}"),
    };
    let runtime_dir = PathBuf::from(resource_dir).join("lib").join("darwin");
    assert!(
        runtime_dir.join("libclang_rt.osx.a").is_file(),
        "libclang_rt.osx.a not found in {}",
        runtime_dir.display()
    );
    println!("cargo:rustc-link-search=native={}", runtime_dir.display());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
