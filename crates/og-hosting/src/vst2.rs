//! VST 2.4 bundle scanner and metadata inspector.
//! Zero-overhead pure-native implementation.

use crate::bundle_inspect::inspect_bundle;
use og_common::server_plugin::ServerPlugin;
use std::path::{Path, PathBuf};

/// Standard VST2 search directories per platform.
pub fn default_vst2_folders() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/VST"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            dirs.push(PathBuf::from(&pf).join("VstPlugins"));
            dirs.push(PathBuf::from(&pf).join("Steinberg/VstPlugins"));
            dirs.push(PathBuf::from(&pf).join("Common Files/VST2"));
        }
        if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
            dirs.push(PathBuf::from(&pf86).join("VstPlugins"));
            dirs.push(PathBuf::from(&pf86).join("Steinberg/VstPlugins"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/lib/vst"));
        dirs.push(PathBuf::from("/usr/local/lib/vst"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join(".vst"));
        }
    }
    dirs
}

/// Find all `.vst` plugin bundles or DLLs in default and extra folders.
pub fn find_vst2_bundles(extra_folders: &[String]) -> Vec<PathBuf> {
    let mut folders = default_vst2_folders();
    for f in extra_folders {
        folders.push(PathBuf::from(f));
    }
    let mut out = Vec::new();
    for folder in folders {
        if folder.is_dir() {
            collect_vst2_bundles(&folder, &mut out);
        }
    }
    out
}

fn collect_vst2_bundles(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("vst") {
            out.push(path);
        } else if path.extension().and_then(|e| e.to_str()) == Some("dll") {
            #[cfg(target_os = "windows")]
            out.push(path);
        } else if path.is_dir() {
            collect_vst2_bundles(&path, out);
        }
    }
}

/// Scan VST2 folders for `.vst` bundles and extract plugin metadata.
pub fn scan_vst2_folders(extra_folders: &[String]) -> Vec<ServerPlugin> {
    let bundles = find_vst2_bundles(extra_folders);
    let mut plugins = Vec::new();
    for bundle in bundles {
        plugins.extend(inspect_vst2_classes(&bundle));
    }
    plugins
}

pub fn inspect_vst2_classes(bundle_path: &Path) -> Vec<ServerPlugin> {
    inspect_bundle(bundle_path, "VST")
}
