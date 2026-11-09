//! Insert format entry points. A missing bundle or a file without the format's
//! export is a failure. This does not load the bundle in a DAW.
//! Run scripts/build-all-bundles.sh first; the test checks its staged output.

use std::path::{Path, PathBuf};
use std::process::Command;

fn stage() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/release/OpenGridder_artefacts/Release")
}

fn assert_symbol(binary: &Path, symbol: &str) {
    assert!(
        binary.is_file(),
        "missing {}; run scripts/build-all-bundles.sh first",
        binary.display()
    );
    let len = std::fs::metadata(binary).unwrap().len();
    assert!(len > 8_192, "{} is {} bytes", binary.display(), len);
    let listed = Command::new("nm").args(["-gU", binary.to_str().unwrap()]).output().expect("nm");
    let text = String::from_utf8_lossy(&listed.stdout);
    assert!(listed.status.success(), "nm failed for {}: {}", binary.display(), String::from_utf8_lossy(&listed.stderr));
    assert!(text.contains(symbol), "{} does not export {symbol}\n{text}", binary.display());
}

#[test]
fn insert_formats_export_real_entry_points() {
    let root = stage();
    assert_symbol(&root.join("AU/OpenGridder.component/Contents/MacOS/OpenGridder"), "AudioComponentFactoryFunction");
    assert_symbol(&root.join("VST3/OpenGridder.vst3/Contents/MacOS/OpenGridder"), "GetPluginFactory");
    assert_symbol(&root.join("CLAP/OpenGridder.clap"), "clap_entry");
    assert_symbol(&root.join("LV2/OpenGridder.lv2/OpenGridder.dylib"), "lv2_descriptor");
    assert_symbol(&root.join("AAX/OpenGridder.aaxplugin/Contents/MacOS/OpenGridder"), "GetEffectDescriptions");
}
