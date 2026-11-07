use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let sdk = PathBuf::from("/Volumes/Developer 2/Dependencies/clap-wrapper/build/cpm/vst3sdk");
    let header = sdk.join("pluginterfaces/vst/ivstaudioprocessor.h");
    if !header.is_file() {
        panic!("VST3 SDK header missing at {}", header.display());
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("ag_native_host.o");
    let source = manifest.join("native/host.cpp");
    let compiled = Command::new("c++")
        .args(["-std=c++17", "-DINIT_CLASS_IID", "-O2", "-fPIC", "-c"])
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .arg("-I")
        .arg(&sdk)
        .arg("-I")
        .arg("/Volumes/Developer 2/SDKs/clap-sdk/include")
        .arg("-I")
        .arg("/opt/homebrew/include")
        .arg("-I")
        .arg("/opt/homebrew/include/lilv-0")
        .status()
        .expect("invoke c++");
    if !compiled.success() {
        panic!("c++ failed to compile {}", source.display());
    }
    let archive = out.join("libagnative.a");
    let archived = Command::new("ar")
        .arg("rcs")
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("invoke ar");
    if !archived.success() {
        panic!("ar failed to archive {}", archive.display());
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=agnative");
    println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
    println!("cargo:rustc-link-lib=dylib=lilv-0");
    println!("cargo:rustc-link-lib=c++");
    println!("cargo:rerun-if-changed={}", source.display());
}
