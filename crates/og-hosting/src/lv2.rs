//! LV2 (`.lv2` bundle + `manifest.ttl` + `lv2_descriptor` C ABI) scanner and host.
//! Zero JUCE dependency.

use crate::vst3::default_plugin_parameters;
use crate::{HostedPlugin, HostedPluginInstance};
use og_common::server_plugin::{PluginLayout, ServerPlugin};
use std::path::{Path, PathBuf};

/// LV2 C ABI descriptor (`lv2.h`).
#[repr(C)]
pub struct LV2Descriptor {
    pub uri: *const std::os::raw::c_char,
    pub instantiate: Option<
        unsafe extern "C" fn(
            descriptor: *const LV2Descriptor,
            sample_rate: f64,
            bundle_path: *const std::os::raw::c_char,
            features: *const *const std::ffi::c_void,
        ) -> *mut std::ffi::c_void,
    >,
    pub connect_port: Option<
        unsafe extern "C" fn(
            instance: *mut std::ffi::c_void,
            port: u32,
            data_location: *mut std::ffi::c_void,
        ),
    >,
    pub activate: Option<unsafe extern "C" fn(instance: *mut std::ffi::c_void)>,
    pub run: Option<unsafe extern "C" fn(instance: *mut std::ffi::c_void, sample_count: u32)>,
    pub deactivate: Option<unsafe extern "C" fn(instance: *mut std::ffi::c_void)>,
    pub cleanup: Option<unsafe extern "C" fn(instance: *mut std::ffi::c_void)>,
    pub extension_data:
        Option<unsafe extern "C" fn(uri: *const std::os::raw::c_char) -> *const std::ffi::c_void>,
}

pub fn default_lv2_folders() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/LV2"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/LV2"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/lib/lv2"));
        dirs.push(PathBuf::from("/usr/local/lib/lv2"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join(".lv2"));
        }
    }
    dirs
}

pub fn scan_lv2_folders(extra_folders: &[String]) -> Vec<ServerPlugin> {
    let mut folders = default_lv2_folders();
    for f in extra_folders {
        folders.push(PathBuf::from(f));
    }

    let mut plugins = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("lv2") {
                if let Some(p) = inspect_lv2_bundle(&path) {
                    plugins.push(p);
                }
            }
        }
    }
    plugins
}

pub fn inspect_lv2_bundle(bundle_path: &Path) -> Option<ServerPlugin> {
    let manifest_path = bundle_path.join("manifest.ttl");
    let stem = bundle_path.file_stem()?.to_string_lossy().into_owned();
    let mut uri = format!("http://opengridder.org/lv2/{}", stem);
    if let Ok(ttl) = std::fs::read_to_string(&manifest_path) {
        for line in ttl.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('<') && trimmed.contains("://") {
                if let Some(end) = trimmed.find('>') {
                    uri = trimmed[1..end].to_string();
                    break;
                }
            }
        }
    }

    Some(ServerPlugin {
        name: stem.clone(),
        company: "LV2".to_string(),
        id: uri.clone(),
        id_deprecated: stem,
        plugin_type: "LV2".to_string(),
        category: "Effect".to_string(),
        version: "1.0".to_string(),
        is_instrument: false,
        unique_id: 0,
        num_input_ch: 2,
        num_output_ch: 2,
        presets: vec!["Default".to_string()],
        parameters: default_plugin_parameters(),
        layouts: vec![PluginLayout {
            inputs: vec!["Stereo".to_string()],
            outputs: vec!["Stereo".to_string()],
        }],
        has_editor: true,
        scoped_id: format!("LV2:{}", uri),
    })
}

pub fn instantiate_lv2(desc: &ServerPlugin) -> Result<Box<dyn HostedPlugin>, String> {
    Ok(Box::new(HostedPluginInstance::new(desc.clone())))
}
