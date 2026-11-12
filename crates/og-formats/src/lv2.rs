//! LV2 `lv2_descriptor`. Index 0 is `https://opengrid.studio/plugins/audiogridder`.

use std::ffi::{c_char, c_void};

use openplugin::prelude::Plugin;

const URI: &[u8] = b"https://opengrid.studio/plugins/audiogridder\0";

#[repr(C)]
struct Lv2Feature {
    uri: *const c_char,
    data: *mut c_void,
}

#[repr(C)]
pub(crate) struct Lv2Descriptor {
    uri: *const c_char,
    instantiate: extern "C" fn(
        *const Lv2Descriptor,
        f64,
        *const c_char,
        *const *const Lv2Feature,
    ) -> *mut c_void,
    connect_port: extern "C" fn(*mut c_void, u32, *mut c_void),
    activate: extern "C" fn(*mut c_void),
    run: extern "C" fn(*mut c_void, u32),
    deactivate: extern "C" fn(*mut c_void),
    cleanup: extern "C" fn(*mut c_void),
    extension_data: extern "C" fn(*const c_char) -> *const c_void,
}

unsafe impl Sync for Lv2Descriptor {}

struct Lv2Instance {
    plugin: og_plugin::AudioGridder,
    ports: [*mut f32; 4],
    scratch: [Vec<f32>; 2],
    prepared: usize,
}

static DESCRIPTOR: Lv2Descriptor = Lv2Descriptor {
    uri: URI.as_ptr() as *const c_char,
    instantiate: lv2_instantiate,
    connect_port: lv2_connect,
    activate: lv2_activate,
    run: lv2_run,
    deactivate: lv2_deactivate,
    cleanup: lv2_cleanup,
    extension_data: lv2_extension,
};

extern "C" fn lv2_instantiate(
    _descriptor: *const Lv2Descriptor,
    sample_rate: f64,
    _bundle_path: *const c_char,
    _features: *const *const Lv2Feature,
) -> *mut c_void {
    let facts = crate::facts();
    let mut plugin = og_plugin::AudioGridder::new();
    let info = Plugin::info(&plugin);
    let _ = (info.name, facts.latency);
    let rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
        sample_rate as f32
    } else {
        48_000.0
    };
    Plugin::prepare(&mut plugin, rate, 8192, 2);
    let _ = Plugin::latency(&plugin);
    let instance = Box::new(Lv2Instance {
        plugin,
        ports: [std::ptr::null_mut(); 4],
        scratch: [vec![0.0; 8192], vec![0.0; 8192]],
        prepared: 8192,
    });
    Box::into_raw(instance) as *mut c_void
}

extern "C" fn lv2_connect(instance: *mut c_void, port: u32, data: *mut c_void) {
    if instance.is_null() || port > 3 {
        return;
    }
    unsafe {
        (*(instance as *mut Lv2Instance)).ports[port as usize] = data as *mut f32;
    }
}

extern "C" fn lv2_activate(instance: *mut c_void) {
    if instance.is_null() {
        return;
    }
    unsafe {
        let inst = &mut *(instance as *mut Lv2Instance);
        Plugin::reset(&mut inst.plugin);
    }
}

extern "C" fn lv2_run(instance: *mut c_void, sample_count: u32) {
    if instance.is_null() || sample_count == 0 {
        return;
    }
    unsafe {
        let inst = &mut *(instance as *mut Lv2Instance);
        let frames = (sample_count as usize).min(inst.prepared);
        if frames == 0 {
            return;
        }
        for ch in 0..2 {
            let src = inst.ports[ch];
            if src.is_null() {
                inst.scratch[ch][..frames].fill(0.0);
            } else {
                inst.scratch[ch][..frames].copy_from_slice(std::slice::from_raw_parts(src, frames));
            }
        }
        Plugin::process(&mut inst.plugin, &mut inst.scratch, frames);
        for ch in 0..2 {
            let dst = inst.ports[ch + 2];
            if !dst.is_null() {
                std::slice::from_raw_parts_mut(dst, frames)
                    .copy_from_slice(&inst.scratch[ch][..frames]);
            }
        }
    }
}

extern "C" fn lv2_deactivate(instance: *mut c_void) {
    if instance.is_null() {
        return;
    }
    unsafe {
        Plugin::reset(&mut (*(instance as *mut Lv2Instance)).plugin);
    }
}

extern "C" fn lv2_cleanup(instance: *mut c_void) {
    if instance.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(instance as *mut Lv2Instance));
    }
}

extern "C" fn lv2_extension(_uri: *const c_char) -> *const c_void {
    std::ptr::null()
}

#[no_mangle]
pub extern "C" fn lv2_descriptor(index: u32) -> *const Lv2Descriptor {
    let facts = crate::facts();
    let _ = facts.latency;
    if index == 0 {
        &DESCRIPTOR
    } else {
        std::ptr::null()
    }
}
