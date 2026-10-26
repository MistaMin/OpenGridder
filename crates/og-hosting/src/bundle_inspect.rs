//! macOS CFBundle and cross-platform metadata inspection for VST3 and VST2 plugin bundles.
use og_common::server_plugin::ServerPlugin;
use std::ffi::c_void;
use std::path::Path;

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: *const c_void,
        buffer: *const u8,
        buf_len: isize,
        is_directory: bool,
    ) -> *const c_void;

    fn CFBundleCreate(allocator: *const c_void, bundle_url: *const c_void) -> *const c_void;

    fn CFBundleGetInfoDictionary(bundle: *const c_void) -> *const c_void;

    fn CFBundleGetIdentifier(bundle: *const c_void) -> *const c_void;

    fn CFDictionaryGetValue(the_dict: *const c_void, key: *const c_void) -> *const c_void;

    fn CFStringCreateWithCString(
        alloc: *const c_void,
        c_str: *const u8,
        encoding: u32,
    ) -> *const c_void;

    fn CFStringGetCString(
        the_string: *const c_void,
        buffer: *mut i8,
        buffer_size: isize,
        encoding: u32,
    ) -> u8;

    fn CFRelease(cf: *const c_void);
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[cfg(target_os = "macos")]
unsafe fn cf_to_string(cf_str: *const c_void) -> Option<String> {
    if cf_str.is_null() {
        return None;
    }
    let mut buf = [0i8; 1024];
    if CFStringGetCString(cf_str, buf.as_mut_ptr(), buf.len() as isize, K_CF_STRING_ENCODING_UTF8) != 0 {
        let c_str = std::ffi::CStr::from_ptr(buf.as_ptr());
        return c_str.to_str().ok().map(|s| s.to_string());
    }
    None
}

#[cfg(target_os = "macos")]
unsafe fn cf_dict_string(dict: *const c_void, key: &str) -> Option<String> {
    if dict.is_null() {
        return None;
    }
    let mut key_bytes = key.as_bytes().to_vec();
    key_bytes.push(0);
    let cf_key = CFStringCreateWithCString(std::ptr::null(), key_bytes.as_ptr(), K_CF_STRING_ENCODING_UTF8);
    if cf_key.is_null() {
        return None;
    }
    let val = CFDictionaryGetValue(dict, cf_key);
    CFRelease(cf_key);
    cf_to_string(val)
}

/// Inspect a VST3 or VST2 bundle and extract `ServerPlugin` descriptors.
pub fn inspect_bundle(bundle_path: &Path, plugin_type: &str) -> Vec<ServerPlugin> {
    let file_stem = bundle_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    // 1. Try parsing moduleinfo.json (standardized VST 3.7+ manifest)
    let module_info_candidates = [
        bundle_path.join("Contents/Resources/moduleinfo.json"),
        bundle_path.join("Contents/moduleinfo.json"),
        bundle_path.join("moduleinfo.json"),
    ];

    for candidate in &module_info_candidates {
        if candidate.is_file() {
            if let Ok(content) = std::fs::read_to_string(candidate) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(classes) = val.get("Classes").and_then(|c| c.as_array()) {
                        let mut results = Vec::new();
                        let vendor = val
                            .get("Factory Info")
                            .and_then(|f| f.get("Vendor"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        for cls in classes {
                            let category = cls.get("Category").and_then(|c| c.as_str()).unwrap_or("");
                            // We host Audio Module Classes (effects & processors)
                            if category == "Audio Module Class" {
                                let name = cls
                                    .get("Name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or(&file_stem)
                                    .to_string();
                                let cid = cls
                                    .get("CID")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or(&name)
                                    .to_string();
                                let cls_vendor = cls
                                    .get("Vendor")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(&vendor)
                                    .to_string();
                                let sub_cats = cls
                                    .get("Sub Categories")
                                    .and_then(|s| s.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join("|")
                                    })
                                    .unwrap_or_else(|| "Effect".to_string());
                                let is_instrument = sub_cats.to_lowercase().contains("instrument");

                                let id = if plugin_type == "VST3" && !cid.starts_with("VST3:") {
                                    format!("VST3:{cid}")
                                } else {
                                    cid
                                };
                                let mut plugin = ServerPlugin::new(
                                    name,
                                    cls_vendor,
                                    id.clone(),
                                    id,
                                    plugin_type,
                                    sub_cats,
                                    is_instrument,
                                    vec!["Stereo".to_string()],
                                );
                                plugin.module_path = bundle_path.to_string_lossy().to_string();
                                results.push(plugin);
                            }
                        }
                        if !results.is_empty() {
                            return results;
                        }
                    }
                }
            }
        }
    }

    // 2. On macOS, use CFBundle to inspect Info.plist with native CoreFoundation
    #[cfg(target_os = "macos")]
    {
        let path_str = bundle_path.to_string_lossy();
        let bytes = path_str.as_bytes();
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                true,
            );
            if !url.is_null() {
                let bundle = CFBundleCreate(std::ptr::null(), url);
                CFRelease(url);
                if !bundle.is_null() {
                    let dict = CFBundleGetInfoDictionary(bundle);
                    let cf_id = CFBundleGetIdentifier(bundle);
                    let bundle_id = cf_to_string(cf_id).unwrap_or_else(|| {
                        format!("studio.opengrid.{}.{}", plugin_type.to_lowercase(), file_stem)
                    });

                    let name = cf_dict_string(dict, "CFBundleDisplayName")
                        .or_else(|| cf_dict_string(dict, "CFBundleName"))
                        .or_else(|| cf_dict_string(dict, "CFBundleExecutable"))
                        .unwrap_or_else(|| file_stem.clone());

                    let company = cf_dict_string(dict, "NSHumanReadableCopyright")
                        .or_else(|| cf_dict_string(dict, "CFBundleGetInfoString"))
                        .map(|c| clean_company_name(&c))
                        .unwrap_or_else(|| infer_company_from_id(&bundle_id));

                    let version = cf_dict_string(dict, "CFBundleShortVersionString")
                        .or_else(|| cf_dict_string(dict, "CFBundleVersion"))
                        .unwrap_or_else(|| "1.0".to_string());

                    let category = infer_category_from_name(&name);

                    CFRelease(bundle);

                    let id = if plugin_type == "VST3" && !bundle_id.starts_with("VST3:") {
                        format!("VST3:{bundle_id}")
                    } else if (plugin_type == "VST" || plugin_type == "VST2") && !bundle_id.starts_with("VST:") {
                        format!("VST:{bundle_id}")
                    } else {
                        bundle_id
                    };
                    let mut plugin = ServerPlugin::new(
                        name,
                        company,
                        id.clone(),
                        id,
                        plugin_type,
                        category,
                        false,
                        vec!["Stereo".to_string()],
                    );
                    plugin.version = version;
                    plugin.module_path = path_str.to_string();
                    return vec![plugin];
                }
            }
        }
    }

    // 3. Fallback generic plugin descriptor
    let id = if plugin_type == "VST3" && !file_stem.starts_with("VST3:") {
        format!("VST3:{file_stem}")
    } else if (plugin_type == "VST" || plugin_type == "VST2") && !file_stem.starts_with("VST:") {
        format!("VST:{file_stem}")
    } else {
        file_stem.clone()
    };
    let mut plugin = ServerPlugin::new(
        file_stem.clone(),
        infer_company_from_id(&file_stem),
        id.clone(),
        id,
        plugin_type,
        infer_category_from_name(&file_stem),
        false,
        vec!["Stereo".to_string()],
    );
    plugin.module_path = bundle_path.to_string_lossy().to_string();
    vec![plugin]
}

fn clean_company_name(s: &str) -> String {
    let trimmed = s.trim();
    // Strip common copyright noise like "Copyright © 2026", "All rights reserved."
    let cleaned = trimmed
        .replace("Copyright", "")
        .replace("©", "")
        .replace("(c)", "")
        .replace("(C)", "");
    let parts: Vec<&str> = cleaned.split(|c: char| c == '.' || c == ',' || c == '\n').collect();
    let first = parts.first().unwrap_or(&"").trim();
    // Strip leading year digits if any
    let result = first.trim_start_matches(|c: char| c.is_ascii_digit() || c.is_whitespace());
    if result.is_empty() {
        trimmed.to_string()
    } else {
        result.to_string()
    }
}

fn infer_company_from_id(id: &str) -> String {
    // E.g. "com.soundtoys.EchoBoy" -> "Soundtoys", "com.fabfilter.Pro-Q.3" -> "FabFilter"
    let parts: Vec<&str> = id.split('.').collect();
    if parts.len() >= 2 && (parts[0] == "com" || parts[0] == "org" || parts[0] == "net") {
        let comp = parts[1];
        if !comp.is_empty() {
            let mut chars = comp.chars();
            let first = chars.next().unwrap().to_uppercase().to_string();
            return format!("{}{}", first, chars.as_str());
        }
    }
    "Generic".to_string()
}

fn infer_category_from_name(name: &str) -> String {
    let n = name.to_lowercase();
    if n.contains("eq") || n.contains("equaliz") {
        "EQ".to_string()
    } else if n.contains("comp") || n.contains("limit") || n.contains("gate") || n.contains("dynamics") {
        "Dynamics".to_string()
    } else if n.contains("reverb") || n.contains("verb") || n.contains("room") || n.contains("space") {
        "Reverb".to_string()
    } else if n.contains("delay") || n.contains("echo") || n.contains("tape") {
        "Delay".to_string()
    } else if n.contains("chorus") || n.contains("flang") || n.contains("phas") || n.contains("mod") {
        "Modulation".to_string()
    } else if n.contains("filter") {
        "Filter".to_string()
    } else if n.contains("dist") || n.contains("sat") || n.contains("drive") || n.contains("overdrive") || n.contains("amp") {
        "Distortion".to_string()
    } else if n.contains("meter") || n.contains("scope") || n.contains("anal") || n.contains("util") {
        "Utility".to_string()
    } else {
        "Effect".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inspect_real_vst3_or_vst2() {
        let p_vst3 = Path::new("/Library/Audio/Plug-Ins/VST3/ADPTR MetricAB.vst3");
        if p_vst3.exists() {
            let plugins = inspect_bundle(p_vst3, "VST3");
            assert!(!plugins.is_empty(), "Should discover ADPTR MetricAB VST3");
            assert_eq!(plugins[0].name, "ADPTR MetricAB");
            assert_eq!(plugins[0].plugin_type, "VST3");
            println!("Discovered VST3: {:?}", plugins[0]);
        }

        let p_vst2 = Path::new("/Library/Audio/Plug-Ins/VST/SoundID Reference VST Plugin.vst");
        if p_vst2.exists() {
            let plugins = inspect_bundle(p_vst2, "VST");
            assert!(!plugins.is_empty(), "Should discover SoundID Reference VST2");
            assert_eq!(plugins[0].plugin_type, "VST");
            println!("Discovered VST2: {:?}", plugins[0]);
        }
    }
}

