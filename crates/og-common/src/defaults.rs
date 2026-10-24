//! Port of `Common/Source/Defaults.hpp` and `Common/Source/Defaults.cpp`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub const SERVER_PORT: u16 = 55056;
pub const DEFAULT_SERVER_PORT: u16 = SERVER_PORT;
pub const CLIENT_PORT: u16 = 55088;
pub const SANDBOX_PLUGIN_PORT: u16 = 56056;
pub const PLUGIN_TRAY_PORT: u16 = 55055;
pub const DEFAULT_TRAY_PORT: u16 = PLUGIN_TRAY_PORT;

pub const SANDBOX_CMD_PREFIX: &str = "sandbox";

pub const SANDBOX_PLUGIN_SOCK: &str = "sandbox-plugin-{n}.sock";
pub const PLUGIN_TRAY_SOCK: &str = "plugin-tray.sock";
pub const SERVER_SOCK: &str = "server-{id}.sock";
pub const WORKER_SOCK: &str = "worker-{id}-{n}.sock";

pub const SCAN_WORKERS: usize = 8;
pub const SCAN_ID_START: i32 = 1000;

pub const SCAREA_STEPS: i32 = 30;
pub const SCAREA_FULLSCREEN: i32 = 0xFFFF;

pub const PLUGIN_CHANNELS_MAX: usize = 64;

pub const PLUGIN_FX_CHANNELS_IN: usize = 30;
pub const PLUGIN_FX_CHANNELS_OUT: usize = 32;
pub const PLUGIN_FX_CHANNELS_SC: usize = 2;

pub const PLUGIN_INST_CHANNELS_IN: usize = 0;
pub const PLUGIN_INST_CHANNELS_OUT: usize = 64;
pub const PLUGIN_INST_CHANNELS_SC: usize = 0;

pub const DEFAULT_NUM_OF_BUFFERS: i32 = 2;
pub const DEFAULT_NUM_RECENTS: usize = 10;
pub const DEFAULT_LOAD_PLUGIN_TIMEOUT: u64 = 15000;

pub const BG_COLOR: u32 = 0xff22_2222;
pub const BG_ACTIVE_COLOR: u32 = 0xff32_3232;
pub const TOOLTIP_BG_COLOR: u32 = 0xe032_3232;
pub const BUTTON_COLOR: u32 = 0xff33_3333;
pub const SLIDERTRACK_COLOR: u32 = 0xffff_c13b;
pub const SLIDERTHUMB_COLOR: u32 = 0xaaff_ffff;
pub const SLIDERBG_COLOR: u32 = 0xff60_6060;
pub const ACTIVE_COLOR: u32 = 0xffff_c13b;
pub const CPU_LOW_COLOR: u32 = 0xff00_ff00;
pub const CPU_MEDIUM_COLOR: u32 = 0xffff_ff00;
pub const CPU_HIGH_COLOR: u32 = 0xffff_0000;
pub const PLUGIN_OK_COLOR: u32 = 0xff00_8000;
pub const PLUGIN_NOTCONNECTED_COLOR: u32 = 0xff8b_0000;
pub const PLUGIN_NOTLOADED_COLOR: u32 = 0xff8b_4f00;

pub const MDNS_SERVICE_NAME: &str = "_audiogridder._tcp.local.";

pub const SERVER_CONFIG_FILE_OLD: &str = "~/.audiogridderserver";
pub const PLUGIN_CONFIG_FILE_OLD: &str = "~/.audiogridder";
pub const KNOWN_PLUGINS_FILE_OLD: &str = "~/.audiogridderserver.cache";

pub const SERVER_CONFIG_FILE: &str = "~/.audiogridder/audiogridderserver{id}.cfg";
pub const SERVERS_STARTUP_FILE: &str = "~/.audiogridder/serverstartup.cfg";
pub const PLUGIN_CONFIG_FILE: &str = "~/.audiogridder/audiogridderplugin.cfg";
pub const PLUGIN_TRAY_CONFIG_FILE: &str = "~/.audiogridder/audiogridderplugintray.cfg";
pub const KNOWN_PLUGINS_FILE: &str = "~/.audiogridder/audiogridderserver{id}.cache";
pub const DEAD_MANS_FILE: &str = "~/.audiogridder/audiogridderserver{id}.crash";
pub const SCAN_ERROR_FILE: &str = "~/.audiogridder/audiogridderserver{id}.scanerror";
pub const SCAN_LAYOUT_ERROR_FILE: &str = "~/.audiogridder/audiogridderserver{id}.scanlayout";
pub const PLUGIN_LAYOUTS_FILE: &str = "~/.audiogridder/audiogridderserver{id}.layouts";
pub const SERVER_RUN_FILE: &str = "~/.audiogridder/audiogridderserver{id}.running";
pub const SERVER_WINDOW_POSITIONS_FILE: &str = "~/.audiogridder/audiogridderserver{id}.winpos";
pub const PLUGIN_WINDOW_POSITIONS_FILE: &str = "~/.audiogridder/audiogridderplugin.winpos";
pub const DOMAIN_SOCKETS_DIR: &str = "~/.audiogridder/sockets";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFileType {
    ConfigServer,
    ConfigServerStartup,
    ConfigServerRun,
    ConfigPlugin,
    ConfigPluginCache,
    ConfigPluginTray,
    ConfigDeadMan,
    WindowPositionsServer,
    WindowPositionsPlugin,
    ScanError,
    ScanLayoutError,
    PluginLayouts,
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    } else if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(path)
}

pub fn presets_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join("Documents").join("AudioGridder Presets")
    } else {
        PathBuf::from("AudioGridder Presets")
    }
}

pub fn get_log_dir_name() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join("Library/Logs/AudioGridder");
        }
    }
    expand_tilde("~/.audiogridder/log")
}

pub type ConfigFileKind = ConfigFileType;

pub fn get_config_file_name(
    file_type: ConfigFileType,
    replace: &[(&str, &str)],
) -> PathBuf {
    let (mut file, file_old) = match file_type {
        ConfigFileType::ConfigServer => (SERVER_CONFIG_FILE.to_string(), Some(SERVER_CONFIG_FILE_OLD)),
        ConfigFileType::ConfigServerStartup => (SERVERS_STARTUP_FILE.to_string(), None),
        ConfigFileType::ConfigPlugin => (PLUGIN_CONFIG_FILE.to_string(), Some(PLUGIN_CONFIG_FILE_OLD)),
        ConfigFileType::ConfigPluginCache => (KNOWN_PLUGINS_FILE.to_string(), Some(KNOWN_PLUGINS_FILE_OLD)),
        ConfigFileType::ConfigPluginTray => (PLUGIN_TRAY_CONFIG_FILE.to_string(), None),
        ConfigFileType::ConfigServerRun => (SERVER_RUN_FILE.to_string(), None),
        ConfigFileType::ConfigDeadMan => (DEAD_MANS_FILE.to_string(), None),
        ConfigFileType::WindowPositionsServer => (SERVER_WINDOW_POSITIONS_FILE.to_string(), None),
        ConfigFileType::WindowPositionsPlugin => (PLUGIN_WINDOW_POSITIONS_FILE.to_string(), None),
        ConfigFileType::ScanError => (SCAN_ERROR_FILE.to_string(), None),
        ConfigFileType::ScanLayoutError => (SCAN_LAYOUT_ERROR_FILE.to_string(), None),
        ConfigFileType::PluginLayouts => (PLUGIN_LAYOUTS_FILE.to_string(), None),
    };

    for (k, v) in replace {
        let src = format!("{{{k}}}");
        let dst = if *k == "id" && *v == "0" {
            ""
        } else {
            *v
        };
        file = file.replace(&src, dst);
    }
    file = file.replace("{id}", "");

    let new_path = expand_tilde(&file);

    if let Some(old_str) = file_old {
        let old_path = expand_tilde(old_str);
        if old_path.is_file() {
            if let Some(parent) = new_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if fs::copy(&old_path, &new_path).is_ok() {
                let _ = fs::remove_file(&old_path);
            }
        }
    }

    new_path
}

pub fn get_socket_path(
    socket_name: &str,
    replace: &HashMap<&str, String>,
    delete_if_exists: bool,
) -> PathBuf {
    let root = expand_tilde(DOMAIN_SOCKETS_DIR);
    let _ = fs::create_dir_all(&root);

    let mut name = socket_name.to_string();
    for (k, v) in replace {
        name = name.replace(&format!("{{{k}}}"), v);
    }

    let path = root.join(name);
    if delete_if_exists && path.exists() {
        let _ = fs::remove_file(&path);
    }
    path
}

pub fn unix_domain_sockets_supported() -> bool {
    cfg!(unix)
}
