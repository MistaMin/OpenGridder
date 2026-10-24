//! Port of `Common/Source/Utils.hpp`, `Logger.hpp`, `Tracer.hpp`, and `WindowPositions.hpp`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

static LOGGER_ENABLED: AtomicBool = AtomicBool::new(true);
static TRACER_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_logger_enabled(enabled: bool) {
    LOGGER_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_logger_enabled() -> bool {
    LOGGER_ENABLED.load(Ordering::Relaxed)
}

pub fn set_tracer_enabled(enabled: bool) {
    TRACER_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_tracer_enabled() -> bool {
    TRACER_ENABLED.load(Ordering::Relaxed)
}

pub fn log_msg(tag: &str, msg: &str) {
    if is_logger_enabled() {
        eprintln!("[{tag}] {msg}");
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub host: String,
    pub name: String,
    pub is_ipv6: bool,
    pub id: i32,
    pub uuid: String,
    pub load: f32,
    pub local_mode: bool,
    pub version: String,
    pub updated_epoch_ms: u64,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            host: String::new(),
            name: String::new(),
            is_ipv6: false,
            id: -1,
            uuid: String::new(),
            load: 0.0,
            local_mode: false,
            version: String::new(),
            updated_epoch_ms: now_epoch_ms(),
        }
    }
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl ServerInfo {
    pub fn parse(s: &str) -> Self {
        let parts: Vec<&str> = s.split(':').collect();
        let mut info = Self::default();
        if parts.len() > 1 {
            info.host = parts[0].to_string();
            info.id = parts[1].parse::<i32>().unwrap_or(0);
            if parts.len() > 2 {
                info.name = parts[2].to_string();
            }
            if parts.len() > 3 {
                info.version = parts[3].to_string();
            }
            if parts.len() > 4 {
                info.is_ipv6 = parts[4] == "1";
            }
            if parts.len() > 5 {
                info.local_mode = parts[5] == "1";
            }
            if parts.len() > 6 {
                info.uuid = parts[6].to_string();
            }
        } else if !s.is_empty() {
            info.host = s.to_string();
            info.id = 0;
        }
        info.refresh();
        info
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        host: impl Into<String>,
        name: impl Into<String>,
        is_ipv6: bool,
        id: i32,
        uuid: impl Into<String>,
        load: f32,
        local_mode: bool,
        version: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            name: name.into(),
            is_ipv6,
            id,
            uuid: uuid.into(),
            load,
            local_mode,
            version: version.into(),
            updated_epoch_ms: now_epoch_ms(),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.id > -1 && !self.host.is_empty()
    }

    pub fn matches(&self, other: &Self) -> bool {
        if !self.uuid.is_empty() && !other.uuid.is_empty() {
            return self.uuid == other.uuid;
        }
        self.get_name_and_id() == other.get_name_and_id()
    }

    pub fn get_host_and_id(&self) -> String {
        if self.id > 0 {
            format!("{}:{}", self.host, self.id)
        } else {
            self.host.clone()
        }
    }

    pub fn get_name_and_id(&self) -> String {
        let base = if self.name.is_empty() {
            &self.host
        } else {
            &self.name
        };
        if self.id > 0 {
            format!("{base}:{}", self.id)
        } else {
            base.clone()
        }
    }

    pub fn serialize(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            self.host,
            self.id,
            self.name,
            self.version,
            if self.is_ipv6 { 1 } else { 0 },
            if self.local_mode { 1 } else { 0 },
            self.uuid
        )
    }

    pub fn refresh(&mut self) {
        self.updated_epoch_ms = now_epoch_ms();
    }

    pub fn refresh_with_load(&mut self, load: f32) {
        self.refresh();
        self.load = load;
    }
}

pub fn json_read_file(path: &Path, binary: bool) -> Result<Value, String> {
    if !path.is_file() {
        return Err("file does not exist".to_string());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    if binary {
        rmp_serde::from_slice(&bytes).map_err(|e| e.to_string())
    } else {
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    }
}

pub fn json_write_file(path: &Path, value: &Value, binary: bool) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if binary {
        let bytes = rmp_serde::to_vec(value).map_err(|e| e.to_string())?;
        fs::write(path, bytes).map_err(|e| e.to_string())
    } else {
        let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        fs::write(path, format!("{text}\n")).map_err(|e| e.to_string())
    }
}

pub fn config_parse_file(path: &Path) -> Value {
    json_read_file(path, false).unwrap_or_else(|_| json!({}))
}

pub fn config_write_file(path: &Path, value: &Value) {
    let _ = json_write_file(path, value, false);
}

pub fn json_load_from_file<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn json_save_to_file<T: serde::Serialize>(path: &Path, val: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(val)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(path, format!("{text}\n"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BusesLayout {
    #[serde(rename = "inputBuses", default)]
    pub input_buses: Vec<String>,
    #[serde(rename = "outputBuses", default)]
    pub output_buses: Vec<String>,
}

impl BusesLayout {
    pub fn stereo() -> Self {
        Self {
            input_buses: vec!["L R".to_string()],
            output_buses: vec!["L R".to_string()],
        }
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn deserialize(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct SafeHashMap<K, V> {
    inner: Mutex<HashMap<K, V>>,
}

impl<K: std::cmp::Eq + std::hash::Hash + Clone, V: Clone> Default for SafeHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: std::cmp::Eq + std::hash::Hash + Clone, V: Clone> SafeHashMap<K, V> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn contains(&self, key: &K) -> bool {
        self.inner.lock().unwrap().contains_key(key)
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }

    pub fn insert(&self, key: K, val: V) {
        self.inner.lock().unwrap().insert(key, val);
    }

    pub fn get(&self, key: &K) -> Option<V> {
        self.inner.lock().unwrap().get(key).cloned()
    }

    pub fn get_and_remove(&self, key: &K) -> Option<V> {
        self.inner.lock().unwrap().remove(key)
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }
}
