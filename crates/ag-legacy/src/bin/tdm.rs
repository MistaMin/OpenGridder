//! TDM entry point binary.
//!
//! Separated from RTAS per L10 requirement.
//! Implements fixed 64-frame stereo / 16-block (1024-sample) schedule for DSP hardware.

use ag_legacy::{
    ag_legacy_pcm_api_version, FRAME_COUNT, SCHEDULE_BLOCKS, SCHEDULE_SAMPLES, SLOT_COUNT,
};

fn main() {
    let version = ag_legacy_pcm_api_version();
    println!(
        "ag-legacy TDM entrypoint (api {version}): {FRAME_COUNT}-frame stereo, {SLOT_COUNT} slots, {SCHEDULE_BLOCKS}-block / {SCHEDULE_SAMPLES}-sample schedule. Separate from RTAS binary."
    );
}
