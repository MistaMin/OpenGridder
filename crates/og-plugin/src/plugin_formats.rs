//! Raw C-ABI plugin format entry points for VST3 (`GetPluginFactory` / `bundleEntry`) and
//! LV2 (`lv2_descriptor`) so `og-plugin` compiles as a native zero-JUCE `cdylib`.

use std::ffi::c_void;
use std::os::raw::c_char;

#[unsafe(no_mangle)]
pub extern "C" fn bundleEntry(_bundle_ref: *mut c_void) -> bool {
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn bundleExit() -> bool {
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn ModuleEntry(_shared_library_handle: *mut c_void) -> bool {
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn ModuleExit() -> bool {
    true
}

pub const OPENGRIDDER_VST3_URI: &CStr = c"http://opengridder.org/plugins/OpenGridderFx";
use std::ffi::CStr;
const _USE_CCHAR: Option<*const c_char> = None;
