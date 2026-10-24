//! Steinberg VST3 bundle scanner and dynamic library host (`GetPluginFactory` / `bundleEntry`).
//! Zero JUCE dependency.

use crate::HostedPlugin;
use og_common::server_plugin::{PluginParameter, ServerPlugin};
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
            out.extend(inspect_vst3_classes(&path));
        } else if path.is_dir() {
            scan_vst3_dir(&path, out);
        }
    }
}

pub fn hosting_available() -> bool {
    cfg!(og_vst3_host)
}

pub fn inspect_vst3_bundle(bundle_path: &Path) -> Option<ServerPlugin> {
    inspect_vst3_classes(bundle_path).into_iter().next()
}

pub fn inspect_vst3_classes(bundle_path: &Path) -> Vec<ServerPlugin> {
    #[cfg(og_vst3_host)]
    {
        let result = if let Some(executable) = std::env::var_os("OPENGRIDDER_VST3_PROBE") {
            probe_bundle(&executable, bundle_path)
        } else {
            crate::vst3_native::scan_bundle(bundle_path)
        };
        match result {
            Ok(plugins) => plugins,
            Err(error) => {
                log::warn!("Could not scan {}: {error}", bundle_path.display());
                Vec::new()
            }
        }
    }
    #[cfg(not(og_vst3_host))]
    {
        let _ = bundle_path;
        Vec::new()
    }
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

pub fn instantiate_vst3(desc: &ServerPlugin) -> Result<Box<dyn HostedPlugin>, String> {
    #[cfg(og_vst3_host)]
    {
        Ok(Box::new(crate::vst3_native::Vst3Plugin::new(desc)?))
    }
    #[cfg(not(og_vst3_host))]
    {
        let _ = desc;
        Err("VST3 hosting was not compiled; rebuild with OPENGRIDDER_VST3_SDK".into())
    }
}

#[cfg(og_vst3_host)]
fn probe_bundle(executable: &std::ffi::OsStr, bundle: &Path) -> Result<Vec<ServerPlugin>, String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut child = Command::new(executable)
        .arg("--probe-vst3")
        .arg(bundle)
        .env_remove("OPENGRIDDER_VST3_PROBE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("Missing scan pipe")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(format!("VST3 scanner exited: {status}"))
                }
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("VST3 scan timed out".to_string());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error.to_string());
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "VST3 scan reader panicked")?
        .map_err(|e| e.to_string())?;
    result?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("VST3 scan response too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid VST3 scan response: {e}"))
}
