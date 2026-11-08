use std::{env, path::{Path, PathBuf}, process::Command};
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let sdk = env::var_os("VST3_SDK_ROOT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/Volumes/Developer 2/Dependencies/clap-wrapper/build/cpm/vst3sdk"));
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let src = root.join("native/vst3.cpp");
    let obj = out.join("vst3.o");
    assert!(Command::new("c++").args(["-std=c++17", "-DINIT_CLASS_IID", "-O2", "-fPIC", "-mmacosx-version-min=11.0", "-c"]).arg(&src).arg("-I").arg(&sdk).arg("-o").arg(&obj).status().unwrap().success(), "VST3 interface compilation failed");
    let au_obj = out.join("au.o");
    assert!(Command::new("c++").args(["-std=c++17", "-O2", "-fPIC", "-mmacosx-version-min=11.0", "-c"]).arg(root.join("native/au.cpp")).arg("-o").arg(&au_obj).status().unwrap().success(), "AU interface compilation failed");
    let og_editor_obj = out.join("opengridder_editor.o");
    assert!(Command::new("clang++").args(["-std=c++17", "-fobjc-arc", "-O2", "-fPIC", "-mmacosx-version-min=11.0", "-I", root.join("native").to_str().unwrap(), "-c"]).arg(root.join("native/OpenGridderEditorView.mm")).arg("-o").arg(&og_editor_obj).status().unwrap().success(), "OpenGridderEditorView compilation failed");
    let editor_obj = out.join("editor.o");
    assert!(Command::new("clang++").args(["-std=c++17", "-fobjc-arc", "-O2", "-fPIC", "-mmacosx-version-min=11.0", "-I", root.join("native").to_str().unwrap(), "-c"]).arg(root.join("native/editor.mm")).arg("-o").arg(&editor_obj).status().unwrap().success(), "Native editor compilation failed");
    let lib = out.join("libagvst3.a");
    assert!(Command::new("ar").arg("rcs").arg(&lib).arg(&obj).arg(&au_obj).arg(&og_editor_obj).arg(&editor_obj).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=agvst3");
    println!("cargo:rustc-link-lib=dylib=c++");
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-env-changed=VST3_SDK_ROOT");
    println!("cargo:rerun-if-changed={}", root.join("native/au.cpp").display());
    println!("cargo:rerun-if-changed={}", root.join("native/bridge.h").display());
    println!("cargo:rustc-link-lib=framework=AudioToolbox");
    println!("cargo:rustc-link-lib=framework=AudioUnit");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    println!("cargo:rustc-link-lib=framework=Cocoa");
    println!("cargo:rustc-link-lib=framework=ApplicationServices");
    println!("cargo:rerun-if-changed={}", root.join("native/editor.mm").display());
    println!("cargo:rerun-if-changed={}", root.join("native/editor.h").display());
    println!("cargo:rerun-if-changed={}", root.join("native/OpenGridderEditorView.mm").display());
    println!("cargo:rerun-if-changed={}", root.join("native/OpenGridderEditorView.h").display());
    build_aax(&root, &out);
}

/// AAX layer (native/aax_*.cpp|mm) against the Avid AAX SDK. Set `AAX_SDK_ROOT` to override the
/// SDK location. Without the SDK (or on a non-arm64 target: the prebuilt AAXLibrary is arm64
/// only) the AAX layer is skipped and the other formats still build.
fn build_aax(root: &Path, out: &Path) {
    println!("cargo:rerun-if-env-changed=AAX_SDK_ROOT");
    for f in ["aax_defs.h", "aax_parameters.h", "aax_parameters.cpp", "aax_algorithm.cpp", "aax_describe.cpp", "aax_gui.h", "aax_gui.mm"] {
        println!("cargo:rerun-if-changed={}", root.join("native").join(f).display());
    }
    let candidates: Vec<PathBuf> = match env::var_os("AAX_SDK_ROOT") {
        Some(sdk) => vec![PathBuf::from(sdk)],
        None => vec![
            PathBuf::from("/Volumes/Developer 2/SDKs/aax-sdk-2-8-1"),
            PathBuf::from("/Volumes/Developer/Dev Projects/AAX Tools/aax-sdk-2-8-1"),
        ],
    };
    let lib_name = "libAAXLibrary_libcpp.a";
    let Some(sdk) = candidates
        .into_iter()
        .find(|sdk| sdk.join("Interfaces/AAX.h").is_file() && sdk.join("Libs/Release").join(lib_name).is_file())
    else {
        println!("cargo:warning=AAX SDK not found (set AAX_SDK_ROOT); building without the AAX layer");
        return;
    };
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") || env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("aarch64") {
        println!("cargo:warning=AAXLibrary is arm64 macOS only; building without the AAX layer");
        return;
    }
    let includes = [sdk.join("Interfaces"), sdk.join("Interfaces/ACF"), sdk.join("Libs/AAXLibrary/include"), root.join("native")];
    let mut objects = Vec::new();
    for (src, objc) in [("aax_parameters.cpp", false), ("aax_algorithm.cpp", false), ("aax_describe.cpp", false), ("aax_gui.mm", true)] {
        let obj = out.join(format!("{src}.o"));
        let mut cmd = Command::new("clang++");
        cmd.args(["-std=c++17", "-O2", "-fPIC", "-mmacosx-version-min=11.0", "-arch", "arm64", "-DMAC_VERSION=1", "-Wno-four-char-constants", "-Wno-multichar", "-Wno-undef-prefix"]);
        if objc {
            cmd.arg("-fobjc-arc");
        }
        for inc in &includes {
            cmd.arg("-I").arg(inc);
        }
        cmd.arg("-c").arg(root.join("native").join(src)).arg("-o").arg(&obj);
        assert!(cmd.status().unwrap().success(), "AAX compilation failed: {src}");
        objects.push(obj);
    }
    let lib = out.join("libagaax.a");
    let _ = std::fs::remove_file(&lib);
    assert!(Command::new("ar").arg("rcs").arg(&lib).args(&objects).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=agaax");
    println!("cargo:rustc-link-search=native={}", sdk.join("Libs/Release").display());
    println!("cargo:rustc-link-lib=static=AAXLibrary_libcpp");
    // rustc exports only Rust symbols from a cdylib. Pull the AAXLibrary ACF entry points
    // (AAX_Exports.o) into the image and export them, plus the C++ GetEffectDescriptions they call.
    for sym in [
        "_ACFRegisterPlugin",
        "_ACFRegisterComponent",
        "_ACFGetClassFactory",
        "_ACFCanUnloadNow",
        "_ACFStartup",
        "_ACFShutdown",
        "_ACFGetSDKVersion",
        "__Z21GetEffectDescriptionsP15AAX_ICollection",
    ] {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-u,{sym}");
        println!("cargo:rustc-cdylib-link-arg=-Wl,-exported_symbol,{sym}");
    }
}
