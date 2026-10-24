//! `og-goodlookinui`: Pure-Rust port of Marcos Deida's (`MistaMin`) `GoodLookinUI`
//! hardware UI library (`Design.h` + `GoodLookinUI.h`) and AudioGridder's UI components.

pub mod canvas;
pub mod design;
pub mod widgets;

pub use canvas::{
    draw_console_knob, draw_key, draw_knob, draw_panel, draw_screw, Canvas, Color, Rect,
};
pub use design::{
    escape_csv, read_design, split_csv_line, valid_colour, valid_style, validate, write_design,
    Item, Motion, VALID_STYLES,
};
pub use widgets::{
    render_plugin_editor_rack, GenericParameterSlot, PluginButton, PluginButtonArea,
};

#[cfg(test)]
mod tests {
    use super::*;
    use og_dsp::OversamplingFactor;

    #[test]
    fn test_design_csv_roundtrip_and_validation() {
        let items = vec![
            Item {
                id: "drive".to_string(),
                parameter: "drive_db".to_string(),
                label: "Drive".to_string(),
                style: "console".to_string(),
                colour: "#5F7896".to_string(),
                x: 20,
                y: 20,
                width: 120,
                height: 140,
                font_size: 14.0,
            },
            Item {
                id: "mix".to_string(),
                parameter: "mix_pct".to_string(),
                label: "Mix, Wet/Dry".to_string(),
                style: "bakelite".to_string(),
                colour: "#C47D3B".to_string(),
                x: 160,
                y: 20,
                width: 120,
                height: 140,
                font_size: 14.0,
            },
        ];
        let csv = write_design(&items).expect("write_design should succeed");
        let parsed = read_design(&csv).expect("read_design should succeed");
        assert_eq!(items, parsed);
    }

    #[test]
    fn test_motion_critically_damped_convergence() {
        let mut m = Motion::default();
        m.reset(0.0);
        m.set_target(1.0);
        for _ in 0..120 {
            m.update(1.0 / 60.0);
        }
        assert!((m.value - 1.0).abs() < 1e-3);
    }

    #[test]
    fn test_plugin_button_oversampling_hit_test_and_render() {
        let mut btn = PluginButton::new("vst3-eq", "FabFilter Pro-Q", Rect::new(20.0, 60.0, 200.0, 26.0));
        btn.oversampling_factor = OversamplingFactor::X4;

        // Right edge = 220, h = 26:
        // Delete: 194..220
        // MoveDown: 168..194
        // MoveUp: 142..168
        // Bypass: 116..142
        // Oversampling: 88..116
        // Main: 20..88
        assert_eq!(btn.hit_test(50.0, 70.0), Some(PluginButtonArea::Main));
        assert_eq!(btn.hit_test(100.0, 70.0), Some(PluginButtonArea::Oversampling));
        assert_eq!(btn.hit_test(125.0, 70.0), Some(PluginButtonArea::Bypass));
        assert_eq!(btn.hit_test(150.0, 70.0), Some(PluginButtonArea::MoveUp));
        assert_eq!(btn.hit_test(180.0, 70.0), Some(PluginButtonArea::MoveDown));
        assert_eq!(btn.hit_test(205.0, 70.0), Some(PluginButtonArea::Delete));

        let mut canvas = Canvas::new(480, 260);
        let params = vec![GenericParameterSlot {
            param_idx: 0,
            name: "Output Gain".to_string(),
            label: "dB".to_string(),
            normalized_value: 0.75,
            text_value: "+3.0 dB".to_string(),
            is_boolean: false,
        }];
        render_plugin_editor_rack(&mut canvas, &[btn], &params, "127.0.0.1:55056", 12.4);
        // Verify non-empty rendered pixels
        assert!(canvas.pixels.iter().any(|&b| b > 0));
    }
}
