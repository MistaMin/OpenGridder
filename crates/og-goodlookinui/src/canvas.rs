//! Pure-Rust RGBA software canvas + GoodLookinUI procedural hardware rendering
//! ported 1:1 from `GoodLookinUI/GoodLookinUI.h`:
//! - `draw_screw` (oxidized brass slot screws)
//! - `draw_panel` (vintage dark green/charcoal brushed metal rack panel with 6 corner/edge screws)
//! - `draw_key` (ivory/amber mechanical toggle key with lamp indicator)
//! - `draw_console_knob` (vintage console stepped pot with skirt, flute grooves, indicator line, scale ticks)
//! - `draw_knob` (`metal`, `bakelite`, `ivory`, `console` styles with value readout badge)

use crate::design::Item;
use crate::widgets::glyph_5x7;
use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn from_argb(argb: u32) -> Self {
        Self {
            a: ((argb >> 24) & 0xFF) as u8,
            r: ((argb >> 16) & 0xFF) as u8,
            g: ((argb >> 8) & 0xFF) as u8,
            b: (argb & 0xFF) as u8,
        }
    }

    pub fn from_hex(hex: &str) -> Self {
        let s = hex.strip_prefix('#').unwrap_or(hex);
        if s.len() == 6 {
            let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0x5F);
            let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0x78);
            let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0x96);
            return Self::rgb(r, g, b);
        }
        Self::rgb(0x5F, 0x78, 0x96)
    }

    pub fn with_alpha(self, alpha: f32) -> Self {
        Self {
            a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            ..self
        }
    }

    pub fn brighter(self, amount: f32) -> Self {
        let f = 1.0 + amount.clamp(0.0, 2.0);
        Self {
            r: ((self.r as f32 * f).min(255.0)) as u8,
            g: ((self.g as f32 * f).min(255.0)) as u8,
            b: ((self.b as f32 * f).min(255.0)) as u8,
            a: self.a,
        }
    }

    pub fn darker(self, amount: f32) -> Self {
        let f = (1.0 - amount.clamp(0.0, 1.0)).max(0.0);
        Self {
            r: (self.r as f32 * f) as u8,
            g: (self.g as f32 * f) as u8,
            b: (self.b as f32 * f) as u8,
            a: self.a,
        }
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv = 1.0 - t;
        Self {
            r: (self.r as f32 * inv + other.r as f32 * t).round() as u8,
            g: (self.g as f32 * inv + other.g as f32 * t).round() as u8,
            b: (self.b as f32 * inv + other.b as f32 * t).round() as u8,
            a: (self.a as f32 * inv + other.a as f32 * t).round() as u8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn reduced(self, d: f32) -> Self {
        Self {
            x: self.x + d,
            y: self.y + d,
            w: (self.w - 2.0 * d).max(0.0),
            h: (self.h - 2.0 * d).max(0.0),
        }
    }

    pub fn reduced_xy(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            w: (self.w - 2.0 * dx).max(0.0),
            h: (self.h - 2.0 * dy).max(0.0),
        }
    }

    pub fn translated(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            w: self.w,
            h: self.h,
        }
    }

    pub fn center_x(self) -> f32 {
        self.x + self.w * 0.5
    }

    pub fn center_y(self) -> f32 {
        self.y + self.h * 0.5
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}

/// RGBA pixel buffer canvas supporting anti-aliased shapes, gradients, lines, and bitmap text.
#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize) * (height as usize) * 4;
        Self {
            width,
            height,
            pixels: vec![0; len],
        }
    }

    #[inline]
    pub fn blend_pixel(&mut self, x: i32, y: i32, c: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 || c.a == 0 {
            return;
        }
        let idx = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        if c.a == 255 {
            self.pixels[idx] = c.r;
            self.pixels[idx + 1] = c.g;
            self.pixels[idx + 2] = c.b;
            self.pixels[idx + 3] = 255;
            return;
        }
        let sa = c.a as u32;
        let inv_sa = 255 - sa;
        self.pixels[idx] = ((c.r as u32 * sa + self.pixels[idx] as u32 * inv_sa) / 255) as u8;
        self.pixels[idx + 1] =
            ((c.g as u32 * sa + self.pixels[idx + 1] as u32 * inv_sa) / 255) as u8;
        self.pixels[idx + 2] =
            ((c.b as u32 * sa + self.pixels[idx + 2] as u32 * inv_sa) / 255) as u8;
        self.pixels[idx + 3] =
            (sa + (self.pixels[idx + 3] as u32 * inv_sa) / 255).min(255) as u8;
    }

    pub fn clear(&mut self, c: Color) {
        for chunk in self.pixels.chunks_exact_mut(4) {
            chunk[0] = c.r;
            chunk[1] = c.g;
            chunk[2] = c.b;
            chunk[3] = c.a;
        }
    }

    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        let x0 = (r.x.floor() as i32).max(0);
        let y0 = (r.y.floor() as i32).max(0);
        let x1 = (r.right().ceil() as i32).min(self.width as i32);
        let y1 = (r.bottom().ceil() as i32).min(self.height as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend_pixel(x, y, c);
            }
        }
    }

    pub fn fill_vertical_gradient(&mut self, r: Rect, top: Color, bottom: Color, radius: f32) {
        let x0 = (r.x.floor() as i32).max(0);
        let y0 = (r.y.floor() as i32).max(0);
        let x1 = (r.right().ceil() as i32).min(self.width as i32);
        let y1 = (r.bottom().ceil() as i32).min(self.height as i32);
        let h = r.h.max(1.0);
        for y in y0..y1 {
            let t = ((y as f32 - r.y) / h).clamp(0.0, 1.0);
            let row_c = top.lerp(bottom, t);
            for x in x0..x1 {
                let cov = rounded_rect_coverage(x as f32 + 0.5, y as f32 + 0.5, r, radius);
                if cov > 0.0 {
                    self.blend_pixel(x, y, row_c.with_alpha((row_c.a as f32 / 255.0) * cov));
                }
            }
        }
    }

    pub fn fill_rounded_rect(&mut self, r: Rect, radius: f32, c: Color) {
        self.fill_vertical_gradient(r, c, c, radius);
    }

    pub fn stroke_rounded_rect(&mut self, r: Rect, radius: f32, thickness: f32, c: Color) {
        let half = thickness * 0.5;
        let outer = Rect::new(r.x - half, r.y - half, r.w + thickness, r.h + thickness);
        let inner = Rect::new(
            r.x + half,
            r.y + half,
            (r.w - thickness).max(0.0),
            (r.h - thickness).max(0.0),
        );
        let x0 = (outer.x.floor() as i32).max(0);
        let y0 = (outer.y.floor() as i32).max(0);
        let x1 = (outer.right().ceil() as i32).min(self.width as i32);
        let y1 = (outer.bottom().ceil() as i32).min(self.height as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let cov_out = rounded_rect_coverage(px, py, outer, radius + half);
                let cov_in = rounded_rect_coverage(px, py, inner, (radius - half).max(0.0));
                let cov = (cov_out - cov_in).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_pixel(x, y, c.with_alpha((c.a as f32 / 255.0) * cov));
                }
            }
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, c: Color) {
        let x0 = ((cx - radius - 1.0).floor() as i32).max(0);
        let y0 = ((cy - radius - 1.0).floor() as i32).max(0);
        let x1 = ((cx + radius + 1.0).ceil() as i32).min(self.width as i32);
        let y1 = ((cy + radius + 1.0).ceil() as i32).min(self.height as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = (x as f32 + 0.5) - cx;
                let dy = (y as f32 + 0.5) - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                let cov = (radius + 0.5 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_pixel(x, y, c.with_alpha((c.a as f32 / 255.0) * cov));
                }
            }
        }
    }

    pub fn stroke_circle(&mut self, cx: f32, cy: f32, radius: f32, thickness: f32, c: Color) {
        let half = thickness * 0.5;
        let x0 = ((cx - radius - half - 1.0).floor() as i32).max(0);
        let y0 = ((cy - radius - half - 1.0).floor() as i32).max(0);
        let x1 = ((cx + radius + half + 1.0).ceil() as i32).min(self.width as i32);
        let y1 = ((cy + radius + half + 1.0).ceil() as i32).min(self.height as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = (x as f32 + 0.5) - cx;
                let dy = (y as f32 + 0.5) - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                let cov = (half + 0.5 - (dist - radius).abs()).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_pixel(x, y, c.with_alpha((c.a as f32 / 255.0) * cov));
                }
            }
        }
    }

    pub fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, c: Color) {
        let half = (thickness * 0.5).max(0.5);
        let min_x = ((x1.min(x2) - half - 1.0).floor() as i32).max(0);
        let min_y = ((y1.min(y2) - half - 1.0).floor() as i32).max(0);
        let max_x = ((x1.max(x2) + half + 1.0).ceil() as i32).min(self.width as i32);
        let max_y = ((y1.max(y2) + half + 1.0).ceil() as i32).min(self.height as i32);
        let vx = x2 - x1;
        let vy = y2 - y1;
        let len_sq = vx * vx + vy * vy;
        for y in min_y..max_y {
            for x in min_x..max_x {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let t = if len_sq > 1e-6 {
                    (((px - x1) * vx + (py - y1) * vy) / len_sq).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let proj_x = x1 + t * vx;
                let proj_y = y1 + t * vy;
                let dist = ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt();
                let cov = (half + 0.5 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_pixel(x, y, c.with_alpha((c.a as f32 / 255.0) * cov));
                }
            }
        }
    }

    /// Crisp 5x7 embedded vector/bitmap text renderer with configurable scale and alignment.
    pub fn draw_text_centered(&mut self, text: &str, r: Rect, scale: u32, c: Color) {
        let scale = scale.max(1) as i32;
        let char_w = 6 * scale;
        let char_h = 7 * scale;
        let total_w = (text.chars().count() as i32) * char_w;
        let start_x = (r.center_x() as i32) - total_w / 2;
        let start_y = (r.center_y() as i32) - char_h / 2;
        self.draw_text_at(text, start_x, start_y, scale as u32, c);
    }

    pub fn draw_text_left(&mut self, text: &str, r: Rect, scale: u32, c: Color) {
        let scale = scale.max(1) as i32;
        let char_h = 7 * scale;
        let start_x = r.x as i32;
        let start_y = (r.center_y() as i32) - char_h / 2;
        self.draw_text_at(text, start_x, start_y, scale as u32, c);
    }

    pub fn draw_text_at(&mut self, text: &str, x: i32, y: i32, scale: u32, c: Color) {
        let s = scale.max(1) as i32;
        let mut cursor_x = x;
        for ch in text.chars() {
            let glyph = glyph_5x7(ch);
            for (row_idx, row_bits) in glyph.iter().enumerate() {
                for col_idx in 0..5 {
                    if (row_bits >> (4 - col_idx)) & 1 == 1 {
                        for dy in 0..s {
                            for dx in 0..s {
                                self.blend_pixel(
                                    cursor_x + col_idx * s + dx,
                                    y + (row_idx as i32) * s + dy,
                                    c,
                                );
                            }
                        }
                    }
                }
            }
            cursor_x += 6 * s;
        }
    }
}

fn rounded_rect_coverage(px: f32, py: f32, r: Rect, radius: f32) -> f32 {
    if r.w <= 0.0 || r.h <= 0.0 {
        return 0.0;
    }
    let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let cx = r.center_x();
    let cy = r.center_y();
    let qx = (px - cx).abs() - (r.w * 0.5 - rad);
    let qy = (py - cy).abs() - (r.h * 0.5 - rad);
    let dist = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad;
    (0.5 - dist).clamp(0.0, 1.0)
}

/// Port of `goodlookinui::drawScrew` from `GoodLookinUI/GoodLookinUI.h`.
pub fn draw_screw(canvas: &mut Canvas, cx: f32, cy: f32, radius: f32) {
    canvas.fill_circle(cx, cy + 1.2, radius, Color::rgba(0, 0, 0, 140));
    canvas.fill_circle(
        cx,
        cy,
        radius,
        Color::from_argb(0xff84755d).lerp(Color::from_argb(0xff231e18), 0.45),
    );
    canvas.stroke_circle(cx, cy, radius, 1.0, Color::from_argb(0xb012100d));
    let slot = radius * 0.62;
    canvas.draw_line(
        cx - slot,
        cy - slot * 0.35,
        cx + slot,
        cy + slot * 0.35,
        1.4,
        Color::from_argb(0xc018140f),
    );
    canvas.draw_line(
        cx - slot,
        cy - slot * 0.35 - 0.8,
        cx + slot,
        cy + slot * 0.35 - 0.8,
        0.7,
        Color::rgba(255, 255, 255, 56),
    );
}

/// Port of `goodlookinui::drawPanel` from `GoodLookinUI/GoodLookinUI.h`.
pub fn draw_panel(canvas: &mut Canvas, area: Rect) {
    canvas.clear(Color::from_argb(0xff0b0d0c));
    let outer = area.reduced(8.0);
    canvas.fill_vertical_gradient(
        outer,
        Color::from_argb(0xff313633),
        Color::from_argb(0xff141715),
        14.0,
    );

    let panel = outer.reduced(6.0);
    canvas.fill_vertical_gradient(
        panel,
        Color::from_argb(0xff26312d),
        Color::from_argb(0xff141b18),
        10.0,
    );

    // Subtle horizontal brushed scanlines
    let y_start = panel.y as i32 + 4;
    let y_end = panel.bottom() as i32 - 4;
    for y in (y_start..y_end).step_by(3) {
        canvas.draw_line(
            panel.x + 8.0,
            y as f32,
            panel.right() - 8.0,
            y as f32,
            1.0,
            Color::rgba(255, 255, 255, 5),
        );
    }

    canvas.stroke_rounded_rect(outer, 14.0, 1.4, Color::from_argb(0xff050606));
    canvas.stroke_rounded_rect(panel, 10.0, 1.0, Color::rgba(255, 255, 255, 20));
    canvas.stroke_rounded_rect(panel.reduced(4.0), 8.0, 0.9, Color::from_argb(0x90090c0a));

    draw_screw(canvas, panel.x + 16.0, panel.y + 16.0, 5.5);
    draw_screw(canvas, panel.right() - 16.0, panel.y + 16.0, 5.5);
    draw_screw(canvas, panel.x + 16.0, panel.bottom() - 16.0, 5.5);
    draw_screw(canvas, panel.right() - 16.0, panel.bottom() - 16.0, 5.5);
    if panel.w > 420.0 {
        draw_screw(canvas, panel.center_x(), panel.y + 16.0, 5.0);
        draw_screw(canvas, panel.center_x(), panel.bottom() - 16.0, 5.0);
    }
}

/// Port of `goodlookinui::drawKey` from `GoodLookinUI/GoodLookinUI.h`.
pub fn draw_key(canvas: &mut Canvas, bounds: Rect, label: &str, active: bool, accent: Color) {
    let area = bounds.reduced(3.0);
    let bezel = Rect::new(
        area.x + 4.0,
        area.y + 3.0,
        area.w - 8.0,
        area.h - 20.0,
    );
    let label_bounds = Rect::new(area.x, area.bottom() - 15.0, area.w, 14.0);

    canvas.fill_rounded_rect(bezel.translated(0.0, 2.2), 7.0, Color::rgba(0, 0, 0, 140));
    canvas.fill_vertical_gradient(
        bezel,
        Color::from_argb(0xff101312),
        Color::from_argb(0xff232927),
        7.0,
    );
    canvas.stroke_rounded_rect(bezel, 7.0, 1.0, Color::rgba(255, 255, 255, 20));

    let key = bezel
        .reduced(4.5)
        .translated(0.0, if active { 1.6 } else { 0.0 });
    let top = if active {
        Color::from_argb(0xfff1dfbe)
    } else {
        Color::from_argb(0xfff5ebd8)
    };
    let bottom = if active {
        Color::from_argb(0xffb5966a)
    } else {
        Color::from_argb(0xffc9b492)
    };
    canvas.fill_vertical_gradient(key, top, bottom, 5.5);

    let face = key.reduced_xy(3.5, 3.0);
    canvas.fill_vertical_gradient(
        face,
        top.brighter(0.05),
        bottom.darker(0.08),
        4.2,
    );
    canvas.stroke_rounded_rect(key, 5.5, 1.0, Color::from_argb(0xc0261e16));
    canvas.stroke_rounded_rect(key.reduced(1.2), 4.5, 0.8, Color::rgba(255, 255, 255, 115));

    let lamp = Rect::new(key.center_x() - 10.0, key.y + 4.5, 20.0, 4.0);
    canvas.fill_rounded_rect(
        lamp,
        2.0,
        if active {
            accent.brighter(0.25)
        } else {
            Color::from_argb(0xff4a4135)
        },
    );

    let ridge_y = face.center_y() + 2.0;
    canvas.draw_line(
        face.x + 5.0,
        ridge_y,
        face.right() - 5.0,
        ridge_y,
        1.2,
        Color::from_argb(0x5533281d),
    );
    canvas.draw_line(
        face.x + 5.0,
        ridge_y + 1.1,
        face.right() - 5.0,
        ridge_y + 1.1,
        0.9,
        Color::rgba(255, 255, 255, 115),
    );

    canvas.draw_text_centered(label, label_bounds, 1, Color::from_argb(0xffd8cfbe));
}

/// Port of `goodlookinui::drawConsoleKnob` from `GoodLookinUI/GoodLookinUI.h`.
pub fn draw_console_knob(
    canvas: &mut Canvas,
    bounds: Rect,
    item: &Item,
    normalized_value: f32,
    value_text: &str,
    selected: bool,
) {
    let area = bounds.reduced(3.0);
    let value = normalized_value.clamp(0.0, 1.0);
    let accent = Color::from_hex(&item.colour);

    if selected {
        canvas.stroke_rounded_rect(area.reduced(1.0), 8.0, 1.4, accent.brighter(0.35));
    }

    let label_rect = Rect::new(area.x + 2.0, area.y + 1.0, area.w - 4.0, 15.0);
    canvas.draw_text_centered(&item.label, label_rect, 1, Color::from_argb(0xffdcd3c2));

    let dial_zone = Rect::new(area.x + 4.0, area.y + 15.0, area.w - 8.0, area.h - 31.0);
    let diameter = dial_zone.w.min(dial_zone.h) * 0.74;
    let radius = diameter * 0.5;
    let cx = dial_zone.center_x();
    let cy = dial_zone.center_y() + 2.0;
    let start_angle = -2.38_f32;
    let end_angle = 2.38_f32;
    let angle = start_angle + value * (end_angle - start_angle);

    // 11 scale ticks
    for tick in 0..11 {
        let t = tick as f32 / 10.0;
        let a = start_angle + t * (end_angle - start_angle) - PI * 0.5;
        let major = tick % 2 == 0;
        let r1 = radius + 4.0;
        let r2 = radius + if major { 9.5 } else { 6.8 };
        let tick_col = if t <= value + 0.001 {
            accent.brighter(0.35)
        } else {
            Color::from_argb(0xffbfb5a2).with_alpha(if major { 0.82 } else { 0.48 })
        };
        canvas.draw_line(
            cx + a.cos() * r1,
            cy + a.sin() * r1,
            cx + a.cos() * r2,
            cy + a.sin() * r2,
            if major { 1.4 } else { 1.0 },
            tick_col,
        );
    }

    // Drop shadow + skirt + body + cap
    canvas.fill_circle(cx, cy + 4.0, radius + 2.0, Color::rgba(0, 0, 0, 150));
    canvas.fill_circle(cx, cy, radius + 2.0, Color::from_argb(0xff1a1917));
    canvas.stroke_circle(cx, cy, radius + 2.0, 1.1, Color::from_argb(0xff5b5347));
    canvas.fill_circle(cx, cy, radius * 0.86, Color::from_argb(0xff25221f));

    // Flutes around knob perimeter
    for flute in 0..18 {
        let a = angle - PI * 0.5 + (flute as f32) * (PI * 2.0 / 18.0);
        let r1 = radius * 0.72;
        let r2 = radius * 0.87;
        canvas.draw_line(
            cx + a.cos() * r1,
            cy + a.sin() * r1,
            cx + a.cos() * r2,
            cy + a.sin() * r2,
            1.6,
            if flute % 2 == 0 {
                Color::rgba(255, 255, 255, 28)
            } else {
                Color::rgba(0, 0, 0, 128)
            },
        );
    }

    canvas.fill_circle(cx, cy, radius * 0.66, accent.darker(0.25));
    canvas.stroke_circle(cx, cy, radius * 0.66, 1.0, Color::rgba(255, 255, 255, 45));

    // Pointer indicator
    let pointer_angle = angle - PI * 0.5;
    canvas.draw_line(
        cx + pointer_angle.cos() * (radius * 0.14),
        cy + pointer_angle.sin() * (radius * 0.14),
        cx + pointer_angle.cos() * (radius * 0.63),
        cy + pointer_angle.sin() * (radius * 0.63),
        2.3,
        Color::from_argb(0xfff5ecd9),
    );
    canvas.fill_circle(cx, cy, radius * 0.13, Color::from_argb(0xff2a2520));

    let badge = Rect::new(area.x + 8.0, area.bottom() - 14.0, area.w - 16.0, 13.0);
    canvas.fill_rounded_rect(badge, 3.5, Color::from_argb(0xbb0e1110));
    canvas.stroke_rounded_rect(badge, 3.5, 0.8, Color::rgba(255, 255, 255, 20));
    canvas.draw_text_centered(value_text, badge, 1, Color::from_argb(0xffd7cebe));
}

/// Port of `goodlookinui::drawKnob` from `GoodLookinUI/GoodLookinUI.h`.
pub fn draw_knob(
    canvas: &mut Canvas,
    bounds: Rect,
    item: &Item,
    normalized_value: f32,
    value_text: &str,
    selected: bool,
) {
    if item.style == "console" {
        draw_console_knob(canvas, bounds, item, normalized_value, value_text, selected);
        return;
    }

    let area = bounds.reduced(6.0);
    let accent = Color::from_hex(&item.colour);
    let value = normalized_value.clamp(0.0, 1.0);

    canvas.fill_rounded_rect(area.translated(0.0, 3.0), 12.0, Color::rgba(0, 0, 0, 115));
    canvas.fill_vertical_gradient(
        area,
        Color::from_argb(0xff1e2422),
        Color::from_argb(0xff111514),
        12.0,
    );
    canvas.stroke_rounded_rect(
        area,
        12.0,
        if selected { 1.8 } else { 1.0 },
        if selected {
            accent.brighter(0.4)
        } else {
            Color::rgba(255, 255, 255, 24)
        },
    );

    let dial_zone = Rect::new(area.x + 10.0, area.y + 8.0, area.w - 20.0, area.h - 46.0);
    let diameter = dial_zone.w.min(dial_zone.h) * 0.78;
    let radius = diameter * 0.5;
    let cx = dial_zone.center_x();
    let cy = dial_zone.center_y() + 2.0;
    let start_angle = -2.35_f32;
    let end_angle = 2.35_f32;
    let angle = start_angle + value * (end_angle - start_angle);

    for tick in 0..21 {
        let t = tick as f32 / 20.0;
        let a = start_angle + t * (end_angle - start_angle) - PI * 0.5;
        let major = tick % 5 == 0;
        let r1 = radius + 5.0;
        let r2 = radius + if major { 11.5 } else { 8.0 };
        let col = if t <= value + 0.001 {
            accent.brighter(0.25)
        } else {
            Color::from_argb(0xffcfc6b6).with_alpha(if major { 0.72 } else { 0.34 })
        };
        canvas.draw_line(
            cx + a.cos() * r1,
            cy + a.sin() * r1,
            cx + a.cos() * r2,
            cy + a.sin() * r2,
            if major { 1.6 } else { 1.0 },
            col,
        );
    }

    canvas.fill_circle(cx, cy + 5.0, radius + 2.0, Color::rgba(0, 0, 0, 140));
    canvas.fill_circle(cx, cy, radius + 2.0, Color::from_argb(0xff201d19));
    canvas.stroke_circle(cx, cy, radius + 2.0, 1.2, Color::from_argb(0xff6b6254));

    let cap_col = match item.style.as_str() {
        "bakelite" => Color::from_argb(0xff2c1c16),
        "ivory" => Color::from_argb(0xffdfcfb2),
        _ => Color::from_argb(0xff92999e),
    };
    canvas.fill_circle(cx, cy, radius * 0.72, cap_col);

    let pointer_angle = angle - PI * 0.5;
    let pointer_col = if item.style == "ivory" {
        Color::from_argb(0xff241d17)
    } else {
        Color::from_argb(0xfff5ecd8)
    };
    canvas.draw_line(
        cx + pointer_angle.cos() * (radius * 0.16),
        cy + pointer_angle.sin() * (radius * 0.16),
        cx + pointer_angle.cos() * (radius * 0.68),
        cy + pointer_angle.sin() * (radius * 0.68),
        2.5,
        pointer_col,
    );
    canvas.fill_circle(cx, cy, radius * 0.14, accent);

    let label_bounds = Rect::new(area.x + 8.0, area.bottom() - 36.0, area.w - 16.0, 17.0);
    let value_bounds = Rect::new(area.x + 12.0, area.bottom() - 18.0, area.w - 24.0, 14.0);
    canvas.draw_text_centered(&item.label, label_bounds, 1, Color::from_argb(0xffece3d3));
    canvas.draw_text_centered(value_bounds_str(value_text), value_bounds, 1, accent.brighter(0.45));
}

fn value_bounds_str(s: &str) -> &str {
    s
}
