//! Print the legacy contract version. This binary is not a Pro Tools `.dpm`.

use og_legacy::{
    og_legacy_pcm_api_version, FRAME_COUNT, SCHEDULE_BLOCKS, SCHEDULE_SAMPLES, SLOT_COUNT,
};

fn main() {
    let version = og_legacy_pcm_api_version();
    println!(
        "og-legacy contract api {version}: {FRAME_COUNT}-frame stereo, {SLOT_COUNT} slots, {SCHEDULE_BLOCKS}-block / {SCHEDULE_SAMPLES}-sample schedule. Not a .dpm. Not an OS X 10.6 or i386 build. Not qualified in Pro Tools."
    );
}
