# Native format review and automation repair

2026-11-07. Working tree, no Git commit. Binary/source SHA-256 identities: `formats-review-identity.json`. Root's earlier VST3/AU/editor implementation was reviewed; changes here address VST3 automation timing and missing MIDI/playhead entry paths.

## Changes and evidence

- VST3 now advertises one MIDI input bus. Note-on and note-off events are passed through the audio-owned bridge, with offsets adjusted for each rendered automation segment.
- VST3 reads every parameter-queue point, splits audio at its sample offset, and applies it at that boundary. Previously only the last point was applied to the whole block.
- Per-segment VST3 ProcessContext is converted into `Audio.position`: valid tempo/signature/PPQ and playing/recording/cycle bits, with timeline samples and derived seconds advanced for the segment. It is retained for root's live-transport attachment. Current ordinary local DSP does not consume tempo, and this alone does not prove transport reaches a remote hosted plugin.
- Bridge adds `ag_insert_midi` and `ag_insert_position`; ag-formats depends on the existing ag-protocol crate.
- New native harness assertions exercise the built public COM interfaces, not direct implementation calls. At +6 dB, the queue changes gain at frame 256, then again at 384. With 128-sample delay, output before frame 384 stays unchanged, frame 400 reflects the first point, and the final controller value is the second point's 0.75.
- `vst3-review-host.log`: all twelve combinations passed (mono/stereo, f32/f64, 44.1/48/96 kHz), retaining the original gain, latency and partial-stream state tests.
- `formats-review-test.log`: two Rust format tests passed. The added bridge test confirms note offset 7/channel 2 and full position values including signature 7/8, seconds 1.5, recording and looping reach audio-owned storage.

Commands:

```
cargo build -p ag-formats --offline
c++ -std=c++17 -DINIT_CLASS_IID -I '/Volumes/Developer 2/Dependencies/clap-wrapper/build/cpm/vst3sdk' crates/ag-formats/tests/vst3_host.cpp -o /tmp/ag-open-takeover/vst3-review-host
/tmp/ag-open-takeover/vst3-review-host "$PWD/target/debug/libag_formats.dylib"
cargo test -p ag-formats --offline
```

These are debug harness checks, not DAW qualification, listening acceptance, or release checks.

## Findings sent to root

1. **Fixed by this pass:** VST3 last-point/whole-block automation, absent MIDI bus/events and missing ProcessContext forwarding.
2. **Root fixed during review:** bridge state loading did not increment the editor revision; native unassigned slot writes could report nonzero values although the underlying plugin ignored them.
3. **Still open:** automation is piecewise constant between supplied points. Linear interpolation of VST3 continuous automation ramps is not implemented by this boundary-splitting change.
4. **Still open:** native host latency changes need the declared request/accept policy. Automation queues currently update profile values directly, without requesting host latency recalculation on the UI thread.
5. **Still open:** the AU Cocoa factory hands the view a raw pointer to unit-owned processor storage. Unlike VST3's View, the Cocoa view does not retain a processor owner or receive unit-disposal invalidation. Retaining the view across AudioComponentInstanceDispose risks timer/draw access to freed storage. This is confirmed by the ownership code, not a crash reproduction; add a lifecycle fixture before fixing.
6. **Still open:** AU ignores SetParameter's buffer offset and does not implement ScheduleParameters; native AU transport/MIDI forwarding is absent.
7. **Still open:** remote `activeServer`/`loadedPlugins` state is stored on the bridge's control-side instance. Its application to the live audio session awaits root's runtime integration.
8. **Not validated here:** MIDI emitted by the remote chain, full VST3 MIDI mapping/CC conversion, native AU Cocoa lifecycle, actual DAW host qualification, and release allocation instrumentation.

No source in archived projects or external SDKs was changed. No signing, commits, system installs, or user DAW sessions were used.
