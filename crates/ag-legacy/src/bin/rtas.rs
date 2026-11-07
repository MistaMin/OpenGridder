//! RTAS entry point binary.
//!
//! Separated from TDM per L10 requirement.
//! In legacy Pro Tools systems, RTAS runs on host CPU (dynamic buffer sizes,
//! direct offload transport), while TDM runs on DSP hardware (DSP56321 / 1024-sample schedule).

use ag_legacy::{ag_legacy_pcm_api_version, API_VERSION};

fn main() {
    let version = ag_legacy_pcm_api_version();
    println!(
        "ag-legacy RTAS entrypoint (api {version}): host-based real-time audio suite target. Separate from TDM binary."
    );
    assert_eq!(version, API_VERSION);
}
