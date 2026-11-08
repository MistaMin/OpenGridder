//! Format entry points for the AudioGridder insert.
//!
//! Each exported factory constructs `ag_plugin::AudioGridder` and reads
//! `Plugin::info` / `Plugin::latency`, so the plugin object file stays linked.
//! The audio callbacks below only call `Plugin::process`, which does not touch
//! the network.

mod bridge;
mod au;
mod clap;
mod lv2;
mod vst3;

use std::sync::OnceLock;

use openplugin::prelude::Plugin;

pub(crate) struct Facts {
    pub name: &'static str,
    pub vendor: &'static str,
    pub url: &'static str,
    pub email: &'static str,
    pub version: &'static str,
    pub id: &'static str,
    pub latency: usize,
}

/// Build one plugin and keep its description. Called from every factory.
pub(crate) fn facts() -> &'static Facts {
    static FACTS: OnceLock<Facts> = OnceLock::new();
    FACTS.get_or_init(|| {
        let plugin = ag_plugin::AudioGridder::new();
        let info = Plugin::info(&plugin);
        let latency = Plugin::latency(&plugin);
        Facts {
            name: info.name,
            vendor: info.vendor,
            url: info.url,
            email: info.email,
            version: info.version,
            id: info.id,
            latency,
        }
    })
}

pub(crate) fn cstr_eq(ptr: *const std::ffi::c_char, expected: &[u8]) -> bool {
    if ptr.is_null() || expected.is_empty() {
        return false;
    }
    unsafe {
        for (i, byte) in expected.iter().enumerate() {
            if *ptr.add(i) as u8 != *byte {
                return false;
            }
        }
        *ptr.add(expected.len()) == 0
    }
}

pub(crate) fn write_cstr(dst: &mut [u8], src: &str) {
    let bytes = src.as_bytes();
    let n = bytes.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&bytes[..n]);
    if n < dst.len() {
        dst[n] = 0;
    }
}

/// macOS VST3 TUID layout (`COM_COMPATIBLE` is 0 in the Steinberg SDK).
pub(crate) const fn tuid(a: u32, b: u32, c: u32, d: u32) -> [u8; 16] {
    let parts = [a, b, c, d];
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < 4 {
        let v = parts[i];
        out[i * 4] = (v >> 24) as u8;
        out[i * 4 + 1] = (v >> 16) as u8;
        out[i * 4 + 2] = (v >> 8) as u8;
        out[i * 4 + 3] = v as u8;
        i += 1;
    }
    out
}

pub(crate) fn iid_eq(iid: *const u8, expected: &[u8; 16]) -> bool {
    if iid.is_null() {
        return false;
    }
    unsafe { std::slice::from_raw_parts(iid, 16) == expected.as_slice() }
}
