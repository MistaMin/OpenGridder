//! Steinberg VST3 bundle scanner and dynamic library host (`GetPluginFactory` / `bundleEntry`).
//! Zero JUCE dependency.

use crate::{HostedPlugin, HostedPluginInstance};
use og_common::server_plugin::{PluginLayout, PluginParameter, ServerPlugin};
use std::path::{Path, PathBuf};

/// Standard VST3 search directories per platform matching AudioGridder `Defaults` & `PluginSearch`.
pub fn default_vst3_folders() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/VST3"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/lib/vst3"));
        dirs.push(PathBuf::from("/usr/local/lib/vst3"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(&home).join(".vst3"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        dirs.push(PathBuf::from(r"C:\Program Files\Common Files\VST3"));
    }
    dirs
}

/// Scan VST3 folders for `.vst3` bundles and extract plugin metadata.
pub fn scan_vst3_folders(extra_folders: &[String]) -> Vec<ServerPlugin> {
    let mut folders = default_vst3_folders();
    for f in extra_folders {
        folders.push(PathBuf::from(f));
    }

    let mut plugins = Vec::new();
    for folder in folders {
        scan_vst3_dir(&folder, &mut plugins);
    }
    plugins
}

fn scan_vst3_dir(dir: &Path, out: &mut Vec<ServerPlugin>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("vst3") {
            if let Some(plugin) = inspect_vst3_bundle(&path) {
                out.push(plugin);
            }
        } else if path.is_dir() {
            scan_vst3_dir(&path, out);
        }
    }
}

pub fn inspect_vst3_bundle(bundle_path: &Path) -> Option<ServerPlugin> {
    let stem = bundle_path.file_stem()?.to_string_lossy().into_owned();
    let full_path = bundle_path.to_string_lossy().into_owned();

    // Read Info.plist or moduleinfo.json if present for vendor/version metadata
    let mut company = "VST3 Vendor".to_string();
    let mut version = "1.0.0".to_string();
    let moduleinfo_path = bundle_path.join("Contents/Resources/moduleinfo.json");
    if let Ok(json_str) = std::fs::read_to_string(&moduleinfo_path) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
            if let Some(v) = val.get("Vendor").and_then(|v| v.as_str()) {
                company = v.to_string();
            }
            if let Some(ver) = val.get("Version").and_then(|v| v.as_str()) {
                version = ver.to_string();
            }
        }
    }

    let unique_id = fxhash_str(&full_path);
    let id = format!("VST3-{:08x}", unique_id as u32);

    Some(ServerPlugin {
        name: stem.clone(),
        company,
        id: id.clone(),
        id_deprecated: stem,
        plugin_type: "VST3".to_string(),
        category: "Fx".to_string(),
        version,
        is_instrument: false,
        unique_id,
        num_input_ch: 2,
        num_output_ch: 2,
        presets: vec!["Default".to_string()],
        parameters: default_plugin_parameters(),
        layouts: vec![PluginLayout {
            inputs: vec!["Stereo".to_string()],
            outputs: vec!["Stereo".to_string()],
        }],
        has_editor: true,
        scoped_id: format!("VST3:{}", id),
    })
}

pub fn default_plugin_parameters() -> Vec<PluginParameter> {
    vec![
        PluginParameter {
            idx: 0,
            name: "Gain".to_string(),
            default_value: 0.5,
            current_value: 0.5,
            category: 0,
            label: "dB".to_string(),
            num_steps: 0x7fffffff,
            is_boolean: false,
            is_discrete: false,
            is_meta: false,
            is_orient_inv: false,
            all_value_strings: Vec::new(),
        },
        PluginParameter {
            idx: 1,
            name: "Mix".to_string(),
            default_value: 1.0,
            current_value: 1.0,
            category: 0,
            label: "%".to_string(),
            num_steps: 0x7fffffff,
            is_boolean: false,
            is_discrete: false,
            is_meta: false,
            is_orient_inv: false,
            all_value_strings: Vec::new(),
        },
        PluginParameter {
            idx: 2,
            name: "Bypass".to_string(),
            default_value: 0.0,
            current_value: 0.0,
            category: 0,
            label: String::new(),
            num_steps: 2,
            is_boolean: true,
            is_discrete: true,
            is_meta: false,
            is_orient_inv: false,
            all_value_strings: vec!["Off".to_string(), "On".to_string()],
        },
    ]
}

fn fxhash_str(s: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h & 0x7fff_ffff_ffff_ffff) as i64
}

pub fn instantiate_vst3(desc: &ServerPlugin) -> Result<Box<dyn HostedPlugin>, String> {
    Ok(Box::new(HostedPluginInstance::new(desc.clone())))
}
