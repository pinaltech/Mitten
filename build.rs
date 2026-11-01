use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=native/photokit.m");
    println!("cargo:rerun-if-changed=native/Info.plist");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || env::var_os("CARGO_FEATURE_APPLE_PHOTOS").is_none()
    {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    if env::var_os("CARGO_FEATURE_NATIVE_UI").is_some() {
        let plist =
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("native/Info.plist");
        println!(
            "cargo:rustc-link-arg-bin=desert-photos=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
    let sdk = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .expect("Xcode command-line tools are required for PhotoKit");
    assert!(sdk.status.success(), "Cannot locate macOS SDK");
    let sdk = String::from_utf8(sdk.stdout).unwrap();
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        arch => panic!("Unsupported macOS architecture: {arch}"),
    };
    let status = Command::new("xcrun")
        .args([
            "clang",
            "-arch",
            arch,
            "-isysroot",
            sdk.trim(),
            "-mmacosx-version-min=12.0",
            "-fobjc-arc",
            "-fblocks",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-c",
            "native/photokit.m",
            "-o",
        ])
        .arg(out.join("photokit.o"))
        .status()
        .expect("Cannot compile PhotoKit bridge");
    assert!(status.success(), "PhotoKit bridge compilation failed");
    assert!(
        Command::new("ar")
            .arg("rcs")
            .arg(out.join("libphotokit_bridge.a"))
            .arg(out.join("photokit.o"))
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=photokit_bridge");
    for framework in ["Photos", "Foundation", "AppKit"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
