use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=native/bridge.cpp");
    println!("cargo:rerun-if-changed=native/bridge.h");
    println!("cargo:rerun-if-env-changed=TSDUCK_HOME");
    println!("cargo:rerun-if-env-changed=ProgramFiles");
    if env::var_os("CARGO_FEATURE_NATIVE").is_none() {
        return;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo target OS is missing");
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let host = env::var("HOST").expect("Cargo host is missing");
    let target = env::var("TARGET").expect("Cargo target is missing");
    let root = env::var_os("TSDUCK_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            (host == target && target_os == "windows")
                .then(|| env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("TSDuck")))
                .flatten()
        })
        .expect("set TSDUCK_HOME to the TSDuck SDK for the selected Cargo target");
    let headers = root.join("include");
    let library = if target_os == "windows" {
        root.join("lib").join("Release-Win64")
    } else {
        root.join("lib")
    };
    assert!(
        headers.join("tsduck").join("tsTSPacket.h").is_file(),
        "TSDuck headers missing from {}",
        headers.display()
    );
    assert!(
        library.is_dir(),
        "TSDuck library directory missing: {}",
        library.display()
    );

    cc::Build::new()
        .cpp(true)
        .file("native/bridge.cpp")
        .include(headers.join("tsduck"))
        .include(headers.join("tscore"))
        .define("_TSDUCKDLL_USE", None)
        .define("_TSCOREDLL_USE", None)
        .flag_if_supported(if target_env == "msvc" {
            "/std:c++20"
        } else {
            "-std=c++20"
        })
        .compile("tsan_tsduck_bridge");
    println!("cargo:rustc-link-search=native={}", library.display());
    println!("cargo:rustc-link-lib=dylib=tsduck");
    println!("cargo:rustc-link-lib=dylib=tscore");
}
