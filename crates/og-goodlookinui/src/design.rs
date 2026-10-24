//! Port of `GoodLookinUI/include/goodlookinui/Design.h` (by Marcos Deida / MistaMin).
//!
//! Includes:
//! - `Item` schema (`id`, `parameter`, `label`, `style`, `colour`, `x`, `y`, `width`, `height`, `font_size`)
//! - `validate()` with exact error messages and bounds checks (`0..=2000`, `40..=320`, `8..=36`, `#RRGGBB`)
//! - CSV serialization (`read_design`, `write_design`)
//! - Critically damped spring physics `Motion` (`update(dt)` with natural frequency `omega = 28.0`)

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const VALID_STYLES: [&str; 4] = ["metal", "bakelite", "ivory", "console"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub parameter: String,
    pub label: String,
    pub style: String,
    pub colour: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub font_size: f32,
}

impl Default for Item {
    fn default() -> Self {
        Self {
            id: "knob_1".to_string(),
            parameter: "gain".to_string(),
            label: "Gain".to_string(),
            style: "metal".to_string(),
            colour: "#5F7896".to_string(),
            x: 24,
            y: 24,
            width: 140,
            height: 164,
            font_size: 15.0,
        }
    }
}

pub fn valid_colour(colour: &str) -> bool {
    let bytes = colour.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return false;
    }
    bytes[1..].iter().all(|b| b.is_ascii_hexdigit())
}

pub fn valid_style(style: &str) -> bool {
    VALID_STYLES.contains(&style)
}

/// Validates a slice of `Item`s with 1:1 parity to `goodlookinui::validate()` in `Design.h`.
pub fn validate(items: &[Item]) -> Result<(), String> {
    if items.is_empty() {
        return Err("design must contain at least one item".to_string());
    }

    let mut ids = HashSet::new();
    for item in items {
        if item.id.is_empty() {
            return Err("item id is required".to_string());
        }
        if !ids.insert(item.id.as_str()) {
            return Err(format!("duplicate item id: {}", item.id));
        }
        if item.parameter.is_empty() {
            return Err(format!("parameter is required for {}", item.id));
        }
        if item.label.is_empty() {
            return Err(format!("label is required for {}", item.id));
        }
        if !valid_style(&item.style) {
            return Err(format!("unsupported style for {}", item.id));
        }
        if !valid_colour(&item.colour) {
            return Err(format!("colour must be #RRGGBB for {}", item.id));
        }
        if item.x < 0 || item.y < 0 || item.x > 2000 || item.y > 2000 {
            return Err(format!("position out of range for {}", item.id));
        }
        if item.width < 40 || item.height < 40 || item.width > 320 || item.height > 320 {
            return Err(format!("size out of range for {}", item.id));
        }
        if !(8.0..=36.0).contains(&item.font_size) {
            return Err(format!("fontSize out of range for {}", item.id));
        }
    }

    Ok(())
}

pub fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if in_quotes {
            if ch == '"' {
                if i + 1 < chars.len() && chars[i + 1] == '"' {
                    current.push('"');
                    i += 1;
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(ch);
            }
        } else if ch == '"' {
            in_quotes = true;
        } else if ch == ',' {
            fields.push(current.clone());
            current.clear();
        } else {
            current.push(ch);
        }
        i += 1;
    }
    fields.push(current);
    fields
}

pub fn escape_csv(value: &str) -> String {
    if !value.contains([',', '"', '\n']) {
        return value.to_string();
    }
    let mut out = String::from("\"");
    for ch in value.chars() {
        if ch == '"' {
            out.push_str("\"\"");
        } else {
            out.push(ch);
        }
    }
    out.push('"');
    out
}

pub fn write_design(items: &[Item]) -> Result<String, String> {
    validate(items)?;
    let mut out = String::from("id,parameter,label,style,colour,x,y,width,height,fontSize\n");
    for item in items {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            escape_csv(&item.id),
            escape_csv(&item.parameter),
            escape_csv(&item.label),
            escape_csv(&item.style),
            escape_csv(&item.colour),
            item.x,
            item.y,
            item.width,
            item.height,
            item.font_size
        ));
    }
    Ok(out)
}

pub fn read_design(text: &str) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    let mut saw_header = false;

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if !saw_header {
            saw_header = true;
            if line == "id,parameter,label,style,colour,x,y,width,height,fontSize" {
                continue;
            }
        }
        let fields = split_csv_line(line);
        if fields.len() != 10 {
            return Err("expected 10 CSV columns".to_string());
        }
        let x = fields[5]
            .parse::<i32>()
            .map_err(|_| "invalid numeric field in design CSV".to_string())?;
        let y = fields[6]
            .parse::<i32>()
            .map_err(|_| "invalid numeric field in design CSV".to_string())?;
        let width = fields[7]
            .parse::<i32>()
            .map_err(|_| "invalid numeric field in design CSV".to_string())?;
        let height = fields[8]
            .parse::<i32>()
            .map_err(|_| "invalid numeric field in design CSV".to_string())?;
        let font_size = fields[9]
            .parse::<f32>()
            .map_err(|_| "invalid numeric field in design CSV".to_string())?;

        items.push(Item {
            id: fields[0].clone(),
            parameter: fields[1].clone(),
            label: fields[2].clone(),
            style: fields[3].clone(),
            colour: fields[4].clone(),
            x,
            y,
            width,
            height,
            font_size,
        });
    }

    validate(&items)?;
    Ok(items)
}

/// Critically damped spring physics state matching `goodlookinui::Motion` in `Design.h`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Motion {
    pub value: f32,
    pub target: f32,
    pub velocity: f32,
}

impl Motion {
    pub fn reset(&mut self, next: f32) {
        let clamped = next.clamp(0.0, 1.0);
        self.value = clamped;
        self.target = clamped;
        self.velocity = 0.0;
    }

    pub fn set_target(&mut self, next: f32) {
        self.target = next.clamp(0.0, 1.0);
    }

    pub fn update(&mut self, dt: f32) -> bool {
        let step = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        let omega = 28.0_f32;
        let force = (self.target - self.value) * (omega * omega) - 2.0 * omega * self.velocity;
        self.velocity += force * step;
        self.value = (self.value + self.velocity * step).clamp(0.0, 1.0);

        if (self.target - self.value).abs() < 0.0005 && self.velocity.abs() < 0.0005 {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        true
    }
}
