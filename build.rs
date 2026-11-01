use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=native/photokit.m");
    println!("cargo:rerun-if-changed=native/drag_export.m");
    println!("cargo:rerun-if-changed=native/Info.plist");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || (env::var_os("CARGO_FEATURE_APPLE_PHOTOS").is_none()
            && env::var_os("CARGO_FEATURE_NATIVE_UI").is_none())
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
    let mut objects = vec![];
    for (feature, name) in [
        ("CARGO_FEATURE_APPLE_PHOTOS", "photokit"),
        ("CARGO_FEATURE_NATIVE_UI", "drag_export"),
    ] {
        if env::var_os(feature).is_none() {
            continue;
        }
        let object = out.join(format!("{name}.o"));
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
            ])
            .arg(format!("native/{name}.m"))
            .arg("-o")
            .arg(&object)
            .status()
            .expect("Cannot compile macOS bridge");
        assert!(status.success(), "{name} bridge compilation failed");
        objects.push(object);
    }
    let archive = out.join("libphotos_native.a");
    if archive.exists() {
        std::fs::remove_file(&archive).expect("Cannot replace macOS bridge archive");
    }
    assert!(
        Command::new("ar")
            .arg("rcs")
            .arg(&archive)
            .args(objects)
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=photos_native");
    for framework in ["Photos", "Foundation", "AppKit", "UniformTypeIdentifiers"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
