use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rustc-check-cfg=cfg(og_vst3_host)");
    println!("cargo:rerun-if-env-changed=OPENGRIDDER_VST3_SDK");
    println!("cargo:rerun-if-changed=native/vst3_bridge.cpp");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let Some(sdk) = env::var_os("OPENGRIDDER_VST3_SDK").map(PathBuf::from) else {
        println!("cargo:warning=VST3 hosting disabled: set OPENGRIDDER_VST3_SDK to the external Steinberg SDK root");
        return;
    };
    assert!(
        sdk.join("pluginterfaces/vst/ivstaudioprocessor.h")
            .is_file(),
        "Invalid OPENGRIDDER_VST3_SDK"
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let sources = [
        "pluginterfaces/base/funknown.cpp",
        "pluginterfaces/base/coreiids.cpp",
        "public.sdk/source/vst/vstinitiids.cpp",
        "public.sdk/source/vst/hosting/hostclasses.cpp",
        "public.sdk/source/vst/hosting/pluginterfacesupport.cpp",
        "public.sdk/source/vst/hosting/parameterchanges.cpp",
        "public.sdk/source/vst/utility/stringconvert.cpp",
        "public.sdk/source/common/commonstringconvert.cpp",
    ];
    let mut objects = Vec::new();
    let arch = if env::var("CARGO_CFG_TARGET_ARCH").unwrap() == "x86_64" {
        "x86_64"
    } else {
        "arm64"
    };
    for (index, source) in std::iter::once(PathBuf::from("native/vst3_bridge.cpp"))
        .chain(sources.iter().map(|s| sdk.join(s)))
        .enumerate()
    {
        let object = out.join(format!("vst3-{index}.o"));
        let result = Command::new("clang++")
            .args([
                "-std=c++17",
                "-O2",
                "-fPIC",
                "-DNDEBUG",
                "-arch",
                arch,
                "-Wno-deprecated-declarations",
                if arch == "x86_64" {
                    "-mmacosx-version-min=10.12"
                } else {
                    "-mmacosx-version-min=11.0"
                },
                "-c",
            ])
            .arg(&source)
            .arg("-I")
            .arg(&sdk)
            .arg("-o")
            .arg(&object)
            .status()
            .expect("Could not launch clang++");
        assert!(
            result.success(),
            "VST3 bridge compilation failed: {}",
            source.display()
        );
        objects.push(object);
    }
    let archive = out.join("libog_vst3_host.a");
    assert!(Command::new("ar")
        .arg("crs")
        .arg(&archive)
        .args(&objects)
        .status()
        .unwrap()
        .success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=og_vst3_host");
    println!("cargo:rustc-link-lib=c++");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    println!("cargo:rustc-cfg=og_vst3_host");
}
