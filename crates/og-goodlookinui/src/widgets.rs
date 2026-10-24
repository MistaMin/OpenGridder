//! AudioGridder UI widgets built on top of `GoodLookinUI`:
//! - `PluginButton` (ports `Plugin/Source/PluginButton.hpp` & `.cpp` including `marcosfork` oversampling `[1x]/[2x]/[4x]` badge)
//! - `GenericEditorView` (ports `Plugin/Source/GenericEditor.hpp` & `.cpp` using GoodLookinUI console knobs & toggle keys)
//! - `ServerSettingsView` & `StatisticsView`

use crate::canvas::{draw_console_knob, draw_key, draw_panel, Canvas, Color, Rect};
use crate::design::Item;
use og_dsp::OversamplingFactor;

/// Clickable hit areas on a `PluginButton` in the plugin chain stack,
/// matching `PluginButton::AreaType` in `Plugin/Source/PluginButton.hpp` (commit `559863f`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginButtonArea {
    Main,
    Bypass,
    Oversampling,
    MoveUp,
    MoveDown,
    Delete,
}

/// State and layout of a single plugin row in the AudioGridder plugin stack (`PluginButton`).
#[derive(Debug, Clone)]
pub struct PluginButton {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub bypassed: bool,
    pub ok: bool,
    pub oversampling_factor: OversamplingFactor,
    pub bounds: Rect,
}

impl PluginButton {
    pub fn new(id: impl Into<String>, name: impl Into<String>, bounds: Rect) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            active: false,
            bypassed: false,
            ok: true,
            oversampling_factor: OversamplingFactor::X1,
            bounds,
        }
    }

    /// Hit-test a mouse click `(x, y)` against the button sub-controls, matching `PluginButton.cpp`.
    /// Layout from right to left (each control square is `h x h`, except oversampling which is `28px` wide):
    /// `[ Main Label ... | Oversampling (28px) | Bypass | MoveUp | MoveDown | Delete ]`
    pub fn hit_test(&self, x: f32, y: f32) -> Option<PluginButtonArea> {
        if !self.bounds.contains(x, y) {
            return None;
        }
        let h = self.bounds.h;
        let right = self.bounds.right();
        let del_x = right - h;
        let down_x = del_x - h;
        let up_x = down_x - h;
        let byp_x = up_x - h;
        let os_w = 28.0;
        let os_x = byp_x - os_w;

        if x >= del_x {
            Some(PluginButtonArea::Delete)
        } else if x >= down_x {
            Some(PluginButtonArea::MoveDown)
        } else if x >= up_x {
            Some(PluginButtonArea::MoveUp)
        } else if x >= byp_x {
            Some(PluginButtonArea::Bypass)
        } else if x >= os_x {
            Some(PluginButtonArea::Oversampling)
        } else {
            Some(PluginButtonArea::Main)
        }
    }

    /// Render the `PluginButton` onto `canvas` with GoodLookinUI styling and the `1x`/`2x`/`4x` oversampling badge.
    pub fn draw(&self, canvas: &mut Canvas) {
        let r = self.bounds.reduced(1.5);
        let bg_top = if !self.ok {
            Color::from_argb(0xff4a1f1f)
        } else if self.active {
            Color::from_argb(0xff2d3d36)
        } else {
            Color::from_argb(0xff1e2422)
        };
        let bg_bot = bg_top.darker(0.25);
        canvas.fill_vertical_gradient(r, bg_top, bg_bot, 5.0);

        let border_col = if self.active {
            Color::from_argb(0xffffc107)
        } else {
            Color::rgba(255, 255, 255, 32)
        };
        canvas.stroke_rounded_rect(r, 5.0, 1.0, border_col);

        let h = self.bounds.h;
        let right = self.bounds.right();
        let del_x = right - h;
        let down_x = del_x - h;
        let up_x = down_x - h;
        let byp_x = up_x - h;
        let os_w = 28.0;
        let os_x = byp_x - os_w;

        // Main plugin title
        let main_rect = Rect::new(self.bounds.x + 8.0, self.bounds.y, os_x - self.bounds.x - 10.0, h);
        let text_col = if !self.ok {
            Color::from_argb(0xffff6b6b)
        } else if self.bypassed {
            Color::from_argb(0xff888888)
        } else {
            Color::from_argb(0xffece3d3)
        };
        canvas.draw_text_left(&self.name, main_rect, 1, text_col);

        // Oversampling badge (`1x`, `2x`, `4x`) — highlighted in amber when > 1x (matches PluginButton.cpp in 559863f)
        let os_rect = Rect::new(os_x + 1.0, self.bounds.y + 3.0, os_w - 2.0, h - 6.0);
        let os_active = self.oversampling_factor != OversamplingFactor::X1;
        let os_bg = if os_active {
            Color::from_argb(0xffb8860b)
        } else {
            Color::from_argb(0xff141817)
        };
        canvas.fill_rounded_rect(os_rect, 3.0, os_bg);
        canvas.stroke_rounded_rect(
            os_rect,
            3.0,
            0.8,
            if os_active {
                Color::from_argb(0xffffd54f)
            } else {
                Color::rgba(255, 255, 255, 35)
            },
        );
        canvas.draw_text_centered(
            self.oversampling_factor.label(),
            os_rect,
            1,
            if os_active {
                Color::from_argb(0xfffff8e1)
            } else {
                Color::from_argb(0xff9ea7a3)
            },
        );

        // Bypass button
        let byp_rect = Rect::new(byp_x + 2.0, self.bounds.y + 3.0, h - 4.0, h - 6.0);
        let byp_col = if self.bypassed {
            Color::from_argb(0xff2c302e)
        } else {
            Color::from_argb(0xff2e7d32)
        };
        canvas.fill_rounded_rect(byp_rect, 3.0, byp_col);
        canvas.draw_text_centered("B", byp_rect, 1, Color::from_argb(0xfff5f5f5));

        // Move Up
        let up_rect = Rect::new(up_x + 2.0, self.bounds.y + 3.0, h - 4.0, h - 6.0);
        canvas.draw_text_centered("^", up_rect, 1, Color::from_argb(0xffcfd8dc));

        // Move Down
        let down_rect = Rect::new(down_x + 2.0, self.bounds.y + 3.0, h - 4.0, h - 6.0);
        canvas.draw_text_centered("v", down_rect, 1, Color::from_argb(0xffcfd8dc));

        // Delete
        let del_rect = Rect::new(del_x + 2.0, self.bounds.y + 3.0, h - 4.0, h - 6.0);
        canvas.draw_text_centered("X", del_rect, 1, Color::from_argb(0xffef5350));
    }
}

/// Parameter descriptor for `GenericEditorView` (ports `Plugin/Source/GenericEditor.hpp`).
#[derive(Debug, Clone)]
pub struct GenericParameterSlot {
    pub param_idx: i32,
    pub name: String,
    pub label: String,
    pub normalized_value: f32,
    pub text_value: String,
    pub is_boolean: bool,
}

/// Renders a full GoodLookinUI hardware rack panel for the AudioGridder plugin editor
/// (chain sidebar + generic parameter console knobs or remote screen view).
pub fn render_plugin_editor_rack(
    canvas: &mut Canvas,
    buttons: &[PluginButton],
    params: &[GenericParameterSlot],
    server_status: &str,
    cpu_load: f32,
) {
    let full = Rect::new(0.0, 0.0, canvas.width as f32, canvas.height as f32);
    draw_panel(canvas, full);

    // Top status header bar
    let header = Rect::new(28.0, 20.0, full.w - 56.0, 24.0);
    canvas.fill_rounded_rect(header, 4.0, Color::from_argb(0xff0f1412));
    let status_text = format!("OPENGRIDDER  |  {}  |  CPU {:.1}%", server_status, cpu_load);
    canvas.draw_text_left(&status_text, header.reduced_xy(10.0, 2.0), 1, Color::from_argb(0xffffca28));

    // Left plugin stack
    for btn in buttons {
        btn.draw(canvas);
    }

    // Right-hand GoodLookinUI Console Knobs for GenericEditor parameters
    let start_x = 230.0;
    let start_y = 54.0;
    let cols = ((full.w - start_x - 28.0) / 96.0).floor().max(1.0) as usize;
    for (idx, param) in params.iter().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let cell = Rect::new(
            start_x + (col as f32) * 96.0,
            start_y + (row as f32) * 108.0,
            90.0,
            102.0,
        );
        if cell.bottom() > full.h - 20.0 {
            break;
        }
        if param.is_boolean {
            draw_key(
                canvas,
                cell,
                &param.name,
                param.normalized_value >= 0.5,
                Color::from_argb(0xffd49b35),
            );
        } else {
            let item = Item {
                id: format!("param_{}", param.param_idx),
                parameter: param.name.clone(),
                label: param.name.chars().take(11).collect(),
                style: "console".to_string(),
                colour: "#5F7896".to_string(),
                x: cell.x as i32,
                y: cell.y as i32,
                width: cell.w as i32,
                height: cell.h as i32,
                font_size: 13.0,
            };
            let val_str = if param.text_value.is_empty() {
                format!("{:.0}%", param.normalized_value * 100.0)
            } else {
                param.text_value.clone()
            };
            draw_console_knob(canvas, cell, &item, param.normalized_value, &val_str, false);
        }
    }
}

/// Minimal 5x7 uppercase/lowercase/digit/symbol bitmap font table.
pub(crate) fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
        ':' => [0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b01100, 0b00000],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '+' => [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000],
        '%' => [0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011],
        '|' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        '^' => [0b00100, 0b01010, 0b10001, 0b00000, 0b00000, 0b00000, 0b00000],
        '/' => [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000],
        '(' => [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010],
        ')' => [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000],
        _ => [0b00000; 7],
    }
}
