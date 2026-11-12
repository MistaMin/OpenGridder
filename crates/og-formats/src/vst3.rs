//! VST3 COM interfaces are compiled against the local Steinberg headers.
//! Rust owns the OpenPluginSDK processor and the C++ layer owns host interfaces.
use std::ffi::c_void;
unsafe extern "C" { fn ag_vst3_factory() -> *mut c_void; }
#[no_mangle]
pub extern "C" fn GetPluginFactory() -> *mut c_void { unsafe { ag_vst3_factory() } }
#[no_mangle]
pub extern "C" fn bundleEntry(_: *mut c_void) -> bool { true }
#[no_mangle]
pub extern "C" fn bundleExit() -> bool { true }
