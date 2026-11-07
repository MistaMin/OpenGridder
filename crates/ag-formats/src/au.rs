//! AudioUnit v2 interface compiled against Apple's headers.
use std::ffi::c_void;
unsafe extern "C" { fn ag_au_factory(desc: *const c_void) -> *mut c_void; }
#[no_mangle]
pub extern "C" fn AudioComponentFactoryFunction(desc: *const c_void) -> *mut c_void {
    unsafe { ag_au_factory(desc) }
}
