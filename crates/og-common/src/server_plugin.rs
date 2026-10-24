//! Port of `Common/Source/ServerPlugin.hpp`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PluginParameter {
    pub idx: i32,
    pub name: String,
    #[serde(default)]
    pub default_value: f32,
    #[serde(default)]
    pub current_value: f32,
    #[serde(default)]
    pub category: i32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub num_steps: i32,
    #[serde(default)]
    pub is_boolean: bool,
    #[serde(default)]
    pub is_discrete: bool,
    #[serde(default)]
    pub is_meta: bool,
    #[serde(default)]
    pub is_orient_inv: bool,
    #[serde(default)]
    pub all_value_strings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginLayout {
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ServerPlugin {
    pub name: String,
    pub company: String,
    pub id: String,
    #[serde(rename = "idDeprecated", default)]
    pub id_deprecated: String,
    #[serde(rename = "type", default)]
    pub plugin_type: String,
    pub category: String,
    #[serde(default)]
    pub version: String,
    #[serde(rename = "isInstrument", default)]
    pub is_instrument: bool,
    #[serde(default)]
    pub unique_id: i64,
    #[serde(default = "default_two_ch")]
    pub num_input_ch: i32,
    #[serde(default = "default_two_ch")]
    pub num_output_ch: i32,
    #[serde(default)]
    pub presets: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<PluginParameter>,
    #[serde(default)]
    pub layouts: Vec<PluginLayout>,
    #[serde(default = "default_true")]
    pub has_editor: bool,
    #[serde(default)]
    pub scoped_id: String,
}

fn default_two_ch() -> i32 {
    2
}

fn default_true() -> bool {
    true
}

impl ServerPlugin {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        company: impl Into<String>,
        id: impl Into<String>,
        id_deprecated: impl Into<String>,
        plugin_type: impl Into<String>,
        category: impl Into<String>,
        is_instrument: bool,
        layouts: Vec<String>,
    ) -> Self {
        let mut id_str = id.into();
        let id_dep_str = id_deprecated.into();
        if id_str.is_empty() {
            id_str = id_dep_str.clone();
        }
        let mut cat_str = category.into();
        if cat_str.is_empty() {
            cat_str = "Unknown".to_string();
        }
        let ptype = plugin_type.into();
        let scoped_id = format!("{}:{}", ptype, id_str);
        let mapped_layouts = layouts
            .into_iter()
            .map(|l| PluginLayout {
                inputs: vec![l.clone()],
                outputs: vec![l],
            })
            .collect();
        Self {
            name: name.into(),
            company: company.into(),
            id: id_str,
            id_deprecated: id_dep_str,
            plugin_type: ptype,
            category: cat_str,
            version: "1.0".to_string(),
            is_instrument,
            unique_id: 0,
            num_input_ch: if is_instrument { 0 } else { 2 },
            num_output_ch: 2,
            presets: vec!["Default".to_string()],
            parameters: Vec::new(),
            layouts: mapped_layouts,
            has_editor: true,
            scoped_id,
        }
    }

    pub fn from_string(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(sp) = serde_json::from_str::<ServerPlugin>(trimmed) {
            return Some(sp);
        }
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            return Some(Self::from_json(&v));
        }
        let parts: Vec<&str> = trimmed.split(';').collect();
        if parts.len() < 3 {
            return None;
        }
        let get = |idx: usize| parts.get(idx).copied().unwrap_or("");
        Some(Self::new(
            get(0),
            get(1),
            get(2),
            get(2),
            get(3),
            get(4),
            false,
            Vec::new(),
        ))
    }

    pub fn from_json(j: &Value) -> Self {
        let str_val = |k: &str| j.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let id2 = str_val("id2");
        let id_raw = str_val("id");
        let id_dep = str_val("idDeprecated");

        let (id, deprecated) = if !id2.is_empty() {
            (id2, id_raw)
        } else {
            (id_raw, id_dep)
        };

        let layouts = j
            .get("layouts")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| {
                        if let Some(s) = x.as_str() {
                            Some(s.to_string())
                        } else {
                            x.get("outputs")
                                .and_then(|o| o.as_array())
                                .and_then(|a| a.first())
                                .and_then(|f| f.as_str())
                                .map(|s| s.to_string())
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self::new(
            str_val("name"),
            str_val("company"),
            id,
            deprecated,
            str_val("type"),
            str_val("category"),
            j.get("isInstrument").and_then(|v| v.as_bool()).unwrap_or(false),
            layouts,
        )
    }

    pub fn to_json_value(&self) -> Value {
        let layout_strs: Vec<String> = self
            .layouts
            .iter()
            .filter_map(|l| l.outputs.first().cloned())
            .collect();
        json!({
            "name": self.name,
            "company": self.company,
            "id": self.id,
            "id2": self.id,
            "idDeprecated": self.id_deprecated,
            "type": self.plugin_type,
            "category": self.category,
            "isInstrument": self.is_instrument,
            "layouts": layout_strs,
        })
    }

    pub fn to_string_serialized(&self) -> String {
        self.to_json_value().to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuLevelType {
    None,
    Format,
    Category,
    Company,
    Plugin,
}

#[derive(Debug, Clone)]
pub struct MenuLevel {
    pub level_type: MenuLevelType,
    pub entry_map: BTreeMap<String, ServerPlugin>,
    pub sub_map: BTreeMap<String, MenuLevel>,
}

impl Default for MenuLevel {
    fn default() -> Self {
        Self {
            level_type: MenuLevelType::None,
            entry_map: BTreeMap::new(),
            sub_map: BTreeMap::new(),
        }
    }
}
