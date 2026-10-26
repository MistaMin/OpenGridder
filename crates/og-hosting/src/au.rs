//! macOS AudioUnit (`AudioToolbox` + `CoreFoundation`) native C FFI scanner and host.
//! Zero-overhead pure-native implementation.

use crate::HostedPlugin;
use og_common::server_plugin::{PluginLayout, ServerPlugin};

#[cfg(target_os = "macos")]
#[link(name = "AudioToolbox", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn AudioComponentFindNext(
        in_component: *mut std::ffi::c_void,
        in_desc: *const AudioComponentDescription,
    ) -> *mut std::ffi::c_void;

    fn AudioComponentGetDescription(
        in_component: *mut std::ffi::c_void,
        out_desc: *mut AudioComponentDescription,
    ) -> i32;

    fn AudioComponentCopyName(
        in_component: *mut std::ffi::c_void,
        out_name: *mut *const std::ffi::c_void,
    ) -> i32;

    fn CFStringGetCString(
        the_string: *const std::ffi::c_void,
        buffer: *mut i8,
        buffer_size: isize,
        encoding: u32,
    ) -> u8;

    fn CFRelease(cf: *const std::ffi::c_void);
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioComponentDescription {
    pub component_type: u32,
    pub component_sub_type: u32,
    pub component_manufacturer: u32,
    pub component_flags: u32,
    pub component_flags_mask: u32,
}

pub const K_AUDIO_UNIT_TYPE_EFFECT: u32 = u32::from_be_bytes(*b"aufx");
pub const K_AUDIO_UNIT_TYPE_MUSIC_EFFECT: u32 = u32::from_be_bytes(*b"aumf");
pub const K_AUDIO_UNIT_TYPE_MUSIC_DEVICE: u32 = u32::from_be_bytes(*b"aumu");
pub const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

fn fourcc_to_string(code: u32) -> String {
    let bytes = code.to_be_bytes();
    bytes
        .iter()
        .map(|&b| {
            if (32..=126).contains(&b) {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

/// Scan installed macOS AudioUnits (`aufx`, `aumf`, `aumu`) using `AudioComponentFindNext`.
pub fn scan_audiounits() -> Vec<ServerPlugin> {
    let mut results = Vec::new();
    #[cfg(target_os = "macos")]
    {
        let types = [
            (K_AUDIO_UNIT_TYPE_EFFECT, false, "Effect"),
            (K_AUDIO_UNIT_TYPE_MUSIC_EFFECT, false, "Music Effect"),
            (K_AUDIO_UNIT_TYPE_MUSIC_DEVICE, true, "Instrument"),
        ];
        for (au_type, is_instrument, category) in types {
            let search_desc = AudioComponentDescription {
                component_type: au_type,
                component_sub_type: 0,
                component_manufacturer: 0,
                component_flags: 0,
                component_flags_mask: 0,
            };
            let mut comp: *mut std::ffi::c_void = std::ptr::null_mut();
            loop {
                comp = unsafe { AudioComponentFindNext(comp, &search_desc) };
                if comp.is_null() {
                    break;
                }
                let mut desc = AudioComponentDescription::default();
                let status = unsafe { AudioComponentGetDescription(comp, &mut desc) };
                if status != 0 {
                    continue;
                }
                let mut cf_name: *const std::ffi::c_void = std::ptr::null();
                let name_status = unsafe { AudioComponentCopyName(comp, &mut cf_name) };
                let full_name = if name_status == 0 && !cf_name.is_null() {
                    let mut buf = [0i8; 512];
                    let ok = unsafe {
                        CFStringGetCString(
                            cf_name,
                            buf.as_mut_ptr(),
                            buf.len() as isize,
                            K_CF_STRING_ENCODING_UTF8,
                        )
                    };
                    unsafe { CFRelease(cf_name) };
                    if ok != 0 {
                        unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        format!(
                            "{}: {}",
                            fourcc_to_string(desc.component_manufacturer),
                            fourcc_to_string(desc.component_sub_type)
                        )
                    }
                } else {
                    format!(
                        "{}: {}",
                        fourcc_to_string(desc.component_manufacturer),
                        fourcc_to_string(desc.component_sub_type)
                    )
                };

                let (company, name) = if let Some((mfg, nm)) = full_name.split_once(": ") {
                    (mfg.trim().to_string(), nm.trim().to_string())
                } else {
                    (fourcc_to_string(desc.component_manufacturer), full_name)
                };

                let id = format!(
                    "AudioUnit:{}:{}:{}",
                    fourcc_to_string(desc.component_type),
                    fourcc_to_string(desc.component_sub_type),
                    fourcc_to_string(desc.component_manufacturer)
                );
                let unique_id =
                    ((desc.component_sub_type as i64) << 32) | (desc.component_manufacturer as i64);

                results.push(ServerPlugin {
                    module_path: String::new(),
                    name: name.clone(),
                    company,
                    id: id.clone(),
                    id_deprecated: name,
                    plugin_type: "AudioUnit".to_string(),
                    category: category.to_string(),
                    version: "1.0".to_string(),
                    is_instrument,
                    unique_id,
                    num_input_ch: if is_instrument { 0 } else { 2 },
                    num_output_ch: 2,
                    presets: vec!["Default".to_string()],
                    parameters: Vec::new(),
                    layouts: vec![PluginLayout {
                        inputs: if is_instrument {
                            vec![]
                        } else {
                            vec!["Stereo".to_string()]
                        },
                        outputs: vec!["Stereo".to_string()],
                    }],
                    has_editor: true,
                    scoped_id: format!("AudioUnit:{}", id),
                });
            }
        }
    }
    results
}

pub fn instantiate_audiounit(desc: &ServerPlugin) -> Result<Box<dyn HostedPlugin>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(crate::au_native::AudioUnitPlugin::new(desc)?))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = desc;
        Err("Audio Units are only supported on macOS".into())
    }
}
