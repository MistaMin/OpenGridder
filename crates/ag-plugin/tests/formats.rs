//! Insert format entry points. A missing bundle or a file without the format's
//! export is a failure. This does not load the bundle in a DAW.

use std::path::{Path, PathBuf};
use std::process::Command;

fn dist() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dist/formats")
}

fn assert_symbol(binary: &Path, symbol: &str) {
    assert!(binary.is_file(), "missing {}", binary.display());
    let len = std::fs::metadata(binary).unwrap().len();
    assert!(len > 8_192, "{} is {} bytes", binary.display(), len);
    let listed = Command::new("nm").args(["-gU", binary.to_str().unwrap()]).output().expect("nm");
    let text = String::from_utf8_lossy(&listed.stdout);
    assert!(listed.status.success(), "nm failed for {}: {}", binary.display(), String::from_utf8_lossy(&listed.stderr));
    assert!(text.contains(symbol), "{} does not export {symbol}\n{text}", binary.display());
}

#[test]
fn insert_formats_export_real_entry_points() {
    let root = dist();
    assert_symbol(&root.join("AudioGridder.component/Contents/MacOS/AudioGridder"), "AudioComponentFactoryFunction");
    assert_symbol(&root.join("AudioGridder.vst3/Contents/MacOS/AudioGridder"), "GetPluginFactory");
    assert_symbol(&root.join("AudioGridder.clap"), "clap_entry");
    assert_symbol(&root.join("AudioGridder.lv2/AudioGridder.dylib"), "lv2_descriptor");
    assert_symbol(&root.join("AudioGridder.aaxplugin/Contents/MacOS/AudioGridder"), "GetEffectDescriptions");

    let note_path = root.join("vst2-sdk.txt");
    let note = std::fs::read_to_string(&note_path).unwrap_or_else(|_| panic!("missing {}", note_path.display()));
    let lower = note.to_ascii_lowercase();
    assert!(lower.contains("absent") || lower.contains("present"), "vst2 record must say absent or present:\n{note}");
    if lower.contains("present") {
        let path = note.lines().nth(1).unwrap_or("").trim();
        assert!(Path::new(path).exists(), "recorded VST2 SDK path does not exist: {path}");
    }
}
