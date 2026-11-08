//! LV2 and VST3 hosting. The C++ side loads lilv and a VST3 module factory.
//! The generic editor drawn here is an RGBA image, not a screen capture.

use std::ffi::c_char;
use std::os::raw::c_int;

use serde_json::{json, Value};

use crate::host::{CatalogPlugin, EditorFrame, Hosted};

#[repr(C)]
#[derive(Clone, Copy)]
struct AgCatalogItem {
    name: [u8; 128],
    company: [u8; 128],
    id: [u8; 768],
    format: [u8; 16],
    is_instrument: c_int,
}

enum AgNative {}

unsafe extern "C" {
    fn ag_native_scan(out: *mut AgCatalogItem, cap: c_int) -> c_int;
    fn ag_native_open(id: *const c_char, sample_rate: f32, max_block: c_int, channels: c_int) -> *mut AgNative;
    fn ag_native_close(raw: *mut AgNative);
    fn ag_native_latency(raw: *mut AgNative) -> c_int;
    fn ag_native_param_count(raw: *mut AgNative) -> c_int;
    fn ag_native_param(raw: *mut AgNative, index: c_int, name: *mut c_char, name_cap: c_int, minimum: *mut f32, maximum: *mut f32, current: *mut f32) -> c_int;
    fn ag_native_set_param(raw: *mut AgNative, index: c_int, normalized: f32) -> c_int;
    fn ag_native_get_param(raw: *mut AgNative, index: c_int) -> f32;
    fn ag_native_process(raw: *mut AgNative, audio: *mut *mut f32, channels: c_int, frames: c_int);
}

pub fn scan() -> Vec<CatalogPlugin> {
    let total = unsafe { ag_native_scan(std::ptr::null_mut(), 0) }.max(0) as usize;
    let mut items = vec![AgCatalogItem {
        name: [0; 128],
        company: [0; 128],
        id: [0; 768],
        format: [0; 16],
        is_instrument: 0,
    }; total];
    let count = unsafe { ag_native_scan(items.as_mut_ptr(), items.len() as c_int) };
    let count = count.max(0) as usize;
    items.truncate(count.min(items.len()));
    items.into_iter().map(|item| CatalogPlugin {
        name: text(&item.name),
        company: text(&item.company),
        id: text(&item.id),
        format: text(&item.format),
        is_instrument: item.is_instrument != 0,
        category: if item.is_instrument != 0 { "Instrument".to_string() } else { "Effect".to_string() },
    }).filter(|plugin| !plugin.id.is_empty() && (plugin.format == "LV2" || plugin.format == "VST3" || plugin.format == "CLAP")).collect()
}

pub fn instantiate(id: &str, sample_rate: f32, max_block: usize, channels: usize) -> Option<Box<dyn Hosted>> {
    let mut c_id = id.as_bytes().to_vec();
    c_id.push(0);
    let raw = unsafe { ag_native_open(c_id.as_ptr() as *const c_char, sample_rate, max_block as c_int, channels as c_int) };
    if raw.is_null() {
        return None;
    }
    let count = unsafe { ag_native_param_count(raw) }.max(0) as usize;
    let mut params = Vec::with_capacity(count);
    for index in 0..count {
        let mut name = [0u8; 128];
        let mut minimum = 0.0f32;
        let mut maximum = 1.0f32;
        let mut current = 0.0f32;
        let ok = unsafe {
            ag_native_param(raw, index as c_int, name.as_mut_ptr() as *mut c_char, name.len() as c_int, &mut minimum, &mut maximum, &mut current)
        };
        if ok == 0 {
            continue;
        }
        params.push(NativeParam { name: text(&name), minimum, maximum, default_norm: current });
    }
    Some(Box::new(NativePlugin { raw, params, editor: false, sample_rate, max_block }))
}

struct NativeParam {
    name: String,
    minimum: f32,
    maximum: f32,
    default_norm: f32,
}

struct NativePlugin {
    raw: *mut AgNative,
    params: Vec<NativeParam>,
    editor: bool,
    #[allow(dead_code)]
    sample_rate: f32,
    max_block: usize,
}

unsafe impl Send for NativePlugin {}

impl Drop for NativePlugin {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { ag_native_close(self.raw) };
            self.raw = std::ptr::null_mut();
        }
    }
}

impl Hosted for NativePlugin {
    fn name(&self) -> String {
        self.params.first().map(|param| param.name.clone()).unwrap_or_else(|| "Native".to_string())
    }

    fn has_editor(&self) -> bool {
        true
    }

    fn supports_double(&self) -> bool {
        false
    }

    fn latency(&self) -> i32 {
        unsafe { ag_native_latency(self.raw) }
    }

    fn parameters(&self) -> Value {
        let list: Vec<Value> = self.params.iter().enumerate().map(|(index, param)| {
            json!({
                "idx": index,
                "name": param.name,
                "defaultValue": param.default_norm,
                "currentValue": self.param(index as i32),
                "category": 0,
                "label": "",
                "numSteps": 0x7fff_ffff_u32,
                "isBoolean": false,
                "isDiscrete": false,
                "isMeta": false,
                "isOrientInv": false,
                "minValue": param.minimum,
                "maxValue": param.maximum,
                "allValues": []
            })
        }).collect();
        Value::Array(list)
    }

    fn set_param(&mut self, index: i32, normalized: f32) -> bool {
        if index < 0 || index as usize >= self.params.len() {
            return false;
        }
        unsafe { ag_native_set_param(self.raw, index, normalized) != 0 }
    }

    fn param(&self, index: i32) -> f32 {
        if index < 0 || index as usize >= self.params.len() {
            return 0.0;
        }
        unsafe { ag_native_get_param(self.raw, index) }
    }

    fn set_oversampling(&mut self, _factor: i32) {}

    fn oversampling_latency(&self) -> i32 {
        0
    }

    fn process_f32(&mut self, audio: &mut [Vec<f32>], _midi: &[[u8; 3]], frames: usize) {
        if self.raw.is_null() || audio.is_empty() || frames == 0 {
            return;
        }
        let frames = frames.min(self.max_block).min(audio.iter().map(|channel| channel.len()).min().unwrap_or(0));
        if frames == 0 {
            return;
        }
        let mut ptrs: Vec<*mut f32> = audio.iter_mut().map(|channel| channel.as_mut_ptr()).collect();
        unsafe { ag_native_process(self.raw, ptrs.as_mut_ptr(), ptrs.len() as c_int, frames as c_int) };
    }

    fn process_f64(&mut self, audio: &mut [Vec<f64>], midi: &[[u8; 3]], frames: usize) {
        let mut temporary: Vec<Vec<f32>> = audio.iter().map(|channel| channel.iter().take(frames).map(|sample| *sample as f32).collect()).collect();
        self.process_f32(&mut temporary, midi, frames);
        for (dst, src) in audio.iter_mut().zip(&temporary) {
            for (out, value) in dst.iter_mut().zip(src) {
                *out = f64::from(*value);
            }
        }
    }

    fn show_editor(&mut self) {
        self.editor = true;
    }

    fn hide_editor(&mut self) {
        self.editor = false;
    }

    fn editor_frame(&mut self) -> Option<EditorFrame> {
        if !self.editor {
            return None;
        }
        let width = 64;
        let height = 16;
        let mut pixels = vec![0u8; width * height * 4];
        for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let row = index / width;
            let level = if self.params.is_empty() {
                180
            } else {
                let row_param = row.min(self.params.len() - 1) as i32;
                crate::screen::gray_level(self.param(row_param))
            };
            pixel[0] = level;
            pixel[1] = level;
            pixel[2] = level;
            pixel[3] = 255;
        }
        Some(EditorFrame { width: width as i32, height: height as i32, pixels })
    }

    fn mouse(&mut self, kind: u8, _x: f32, _y: f32, _shift: bool, _ctrl: bool, _alt: bool, delta_x: f32, delta_y: f32) {
        if self.params.is_empty() {
            return;
        }
        if kind == 10 {
            if let Some(next) = crate::host::apply_editor_wheel(self.param(0), delta_x, delta_y) {
                self.set_param(0, next);
            }
            return;
        }
        if kind != 1 {
            return;
        }
        let current = self.param(0);
        let next = if current < 0.5 { 1.0 } else { 0.0 };
        self.set_param(0, next);
    }

    fn key(&mut self, codes: &[u16]) {
        if self.params.is_empty() {
            return;
        }
        if let Some(next) = crate::host::apply_editor_key(self.param(0), codes) {
            self.set_param(0, next);
        }
    }
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
