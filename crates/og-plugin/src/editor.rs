//! GoodLookinUI-powered Plugin Editor (`PluginEditor` in `Plugin/Source/PluginEditor.hpp` & `.cpp`).
//!
//! Renders the plugin chain sidebar (`PluginButton`s with `1x`/`2x`/`4x` oversampling toggle),
//! generic parameter console knobs, server status bar, and handles mouse interactions.

use crate::processor::PluginProcessor;
use og_dsp::OversamplingFactor;
use og_goodlookinui::{
    render_plugin_editor_rack, Canvas, GenericParameterSlot, PluginButton, PluginButtonArea, Rect,
};

pub struct PluginEditor {
    pub width: u32,
    pub height: u32,
    pub canvas: Canvas,
    pub buttons: Vec<PluginButton>,
}

impl Default for PluginEditor {
    fn default() -> Self {
        Self::new(720, 420)
    }
}

impl PluginEditor {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            canvas: Canvas::new(width, height),
            buttons: Vec::new(),
        }
    }

    /// Rebuild `PluginButton` layout from `processor.loaded_plugins` and render the GoodLookinUI rack.
    pub fn render(&mut self, processor: &PluginProcessor) {
        self.buttons.clear();
        for (idx, lp) in processor.loaded_plugins.iter().enumerate() {
            let bounds = Rect::new(28.0, 54.0 + (idx as f32) * 32.0, 190.0, 28.0);
            let mut btn = PluginButton::new(&lp.id, &lp.name, bounds);
            btn.active = processor.active_plugin == idx as i32;
            btn.bypassed = lp.bypassed;
            btn.ok = lp.ok;
            btn.oversampling_factor = OversamplingFactor::from_factor(lp.oversampling_factor);
            self.buttons.push(btn);
        }

        let mut param_slots = Vec::new();
        if processor.active_plugin >= 0 {
            if let Some(lp) = processor.loaded_plugins.get(processor.active_plugin as usize) {
                let mut sorted_params: Vec<_> = lp.params.values().collect();
                sorted_params.sort_by_key(|p| p.idx);
                for p in sorted_params {
                    param_slots.push(GenericParameterSlot {
                        param_idx: p.idx,
                        name: p.name.clone(),
                        label: p.label.clone(),
                        normalized_value: p.current_value,
                        text_value: format!("{:.0}%", p.current_value * 100.0),
                        is_boolean: p.is_boolean,
                    });
                }
            }
        }

        render_plugin_editor_rack(
            &mut self.canvas,
            &self.buttons,
            &param_slots,
            &processor.active_server,
            processor.client.server_cpu_load(),
        );
    }

    /// Handle a click at `(x, y)` on the plugin stack, cycling `1x -> 2x -> 4x -> 1x` when the
    /// oversampling badge is clicked (matching `PluginEditor::buttonClicked` in `PluginEditor.cpp`, commit `559863f`).
    pub fn handle_mouse_click(&mut self, processor: &mut PluginProcessor, x: f32, y: f32) -> bool {
        for (idx, btn) in self.buttons.iter().enumerate() {
            if let Some(area) = btn.hit_test(x, y) {
                match area {
                    PluginButtonArea::Main => {
                        processor.active_plugin = idx as i32;
                    }
                    PluginButtonArea::Bypass => {
                        if btn.bypassed {
                            let _ = processor.unbypass_plugin(idx);
                        } else {
                            let _ = processor.bypass_plugin(idx);
                        }
                    }
                    PluginButtonArea::Oversampling => {
                        let cur = OversamplingFactor::from_factor(
                            processor.get_plugin_oversampling(idx),
                        );
                        let next = cur.next_cycle().as_i32();
                        let _ = processor.set_plugin_oversampling(idx, next);
                    }
                    PluginButtonArea::MoveUp => {
                        if idx > 0 {
                            let _ = processor.exchange_plugins(idx, idx - 1);
                        }
                    }
                    PluginButtonArea::MoveDown => {
                        if idx + 1 < processor.loaded_plugins.len() {
                            let _ = processor.exchange_plugins(idx, idx + 1);
                        }
                    }
                    PluginButtonArea::Delete => {
                        let _ = processor.unload_plugin(idx);
                    }
                }
                self.render(processor);
                return true;
            }
        }
        false
    }
}
