use std::env;
use std::path::PathBuf;
use std::process::Command;

/// Build identity. The build scripts set OG_* (scripts/build-info.sh) so the app bundle and
/// this binary agree; a plain `cargo build` falls back to git and the current time.
fn build_info(manifest: &std::path::Path) {
    let root = manifest.join("../..");
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").arg("-C").arg(&root).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let version = env::var("OG_VERSION").unwrap_or_else(|_| {
        std::fs::read_to_string(root.join("VERSION")).map(|v| v.trim().to_string()).unwrap_or_else(|_| env::var("CARGO_PKG_VERSION").unwrap_or_default())
    });
    let number = env::var("OG_BUILD_NUMBER").unwrap_or_else(|_| git(&["rev-list", "--count", "HEAD"]).unwrap_or_else(|| "0".into()));
    let commit = env::var("OG_GIT_COMMIT").unwrap_or_else(|_| {
        let hash = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
        let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
        if dirty { format!("{hash}+dirty") } else { hash }
    });
    let date = env::var("OG_BUILD_DATE").unwrap_or_else(|_| {
        Command::new("date").arg("+%Y-%m-%d %H:%M %Z").output().ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".into())
    });
    println!("cargo:rustc-env=OG_VERSION={version}");
    println!("cargo:rustc-env=OG_BUILD_NUMBER={number}");
    println!("cargo:rustc-env=OG_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=OG_BUILD_DATE={date}");
    for var in ["OG_VERSION", "OG_BUILD_NUMBER", "OG_GIT_COMMIT", "OG_BUILD_DATE"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed={}", root.join("VERSION").display());
    println!("cargo:rerun-if-changed={}", root.join(".git/HEAD").display());
    println!("cargo:rerun-if-changed={}", root.join(".git/index").display());
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    build_info(&manifest);
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
