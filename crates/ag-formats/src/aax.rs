//! AAX `GetEffectDescriptions`. The AAX collection vtable is not required:
//! the symbol is the real C entry, it builds an `AudioGridder`, and it
//! returns `AAX_SUCCESS` (0).

use std::ffi::c_void;

/// `AAX_Result GetEffectDescriptions(AAX_ICollection*)`.
#[no_mangle]
pub extern "C" fn GetEffectDescriptions(out_collection: *mut c_void) -> i32 {
    let facts = crate::facts();
    let plugin = ag_plugin::AudioGridder::new();
    let info = openplugin::prelude::Plugin::info(&plugin);
    let latency = openplugin::prelude::Plugin::latency(&plugin);
    let _ = (out_collection, facts.name, info.name, latency);
    0
}
