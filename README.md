# OpenGridder — Zero-Overhead Precision Audio Network

OpenGridder is an ultra-low-latency, zero-overhead audio streaming network and DSP offload engine written in **Rust** with pure-native C/C++/Objective-C platform adapters.

OpenGridder is **closed-source, proprietary software** (see `LICENSE`). Third-party components keep their own licences (see `THIRD_PARTY_LICENSES.md`).

---

## Highlights & Features (v1.0 BETA)

- **Zero-Overhead Pure-Native Architecture**: Zero framework bloat, lock-free real-time audio transport, and sub-millisecond network streaming.
- **Pro Tools Hardware Console Integration**: Native Page Tables for Focusrite Control|24 and Avid C|24 (`FrTL` 24-channel layout), plus ICON D-Control/D-Command (`HgTL`), ProControl (`PcTL`), and Command|8/HUI (`MkTL`) for physical motorized faders, rotary encoders, and scribble strip OLED displays.
- **2x-Wide Analog VU Metering**: Dynamic window resizing (`864×420` in Analog VU mode vs `720×420` in Digital PPM mode) using pure native Cocoa (`OpenGridderEditorView`).
- **Dual VU Calibration & I/O Source**: Switchable `-18 dBFS` and `-3 dBFS` reference calibrations, plus `[IN* | OUT]` source metering with over-detection ceiling.
- **HybridEQ Residual ADAA & Harmonic Saturation**: Zero high-frequency loss ADAA with 2x/4x Kaiser-windowed polyphase oversampling filters, Clean/Tube/Transformer character modes, and zipper-free parameter smoothing.
- **6 Verified Plugin Formats**: VST3, AUv2 (`aufx`), AUv3, AAX Native (PACE Eden signed), CLAP, VST 2.4, and LV2.
- **Native macOS Server Application**: Menu bar status icon, dock integration, high-speed timestamp scan caching (`< 500 ms` for 1,700+ plugins), and floating startup splash window.
- **Apple Notarized & Signed**: Signed with Apple Developer ID (Hardened Runtime), notarized via Apple notarytool, and stapled for both `.pkg` and `.dmg`.

---

## Workspace Architecture

| Crate / Module | Description |
| :--- | :--- |
| `crates/og-common` | Binary wire protocol v13 (`HandshakeRequest`, `HandshakeResponse`, `AudioMessage`), `AudioRingBuffer`, `ChannelSet`, `ChannelMapper`, `ImageDiff`, `ServerPlugin`, `Metrics`, `Defaults`, mDNS. |
| `crates/og-dsp` | Polyphase equiripple half-band FIR `Oversampler<f32>` & `Oversampler<f64>` (`1x`, `2x`, `4x`), multi-mono slot processing, and latency reporting. |
| `crates/og-goodlookinui` | Pure-Rust port of `GoodLookinUI` (`Design.h` CSV schema, validation, critically damped spring `Motion`, and procedural hardware vector rendering: `draw_panel`, `draw_screw`, `draw_key`, `draw_console_knob`, `draw_knob`) plus `PluginButton` (`1x`/`2x`/`4x` badge) and `GenericEditor` console rack. |
| `crates/og-hosting` | Native macOS **AU v2** effect hosting (Rust/AudioToolbox), **VST3** effect hosting (Steinberg SDK ABI adapter), and LV2 discovery. |
| `crates/og-screen` | RGBA delta diffing (`og_common::image_diff`) and PNG/raw frame encoding & decoding over the `screen` socket. |
| `crates/og-client` | 4-socket client (`cmdOut`, `cmdIn`, `audio`, `screen`), lock-free `AudioStreamer<T>`, and remote RPC commands including `set_plugin_oversampling`. |
| `crates/og-server` | `OpenGridderServer` binary, `Processor`, `ProcessorChain`, `AudioWorker`, `ScreenWorker`, `Worker`, `SandboxMode`, and `ServerConfig`. |
| `crates/og-plugin` | DAW `PluginProcessor` (`Fx`, `Instrument`, `Midi`), `LoadedPlugin` state v7 (`oversampling_factor` at index 11), 16 automation slots, A/B comparison, `GoodLookinUI` `PluginEditor`, and VST3/LV2 C-ABI exports. |
| `crates/og-tray` | `OpenGridderPluginTray` binary, TCP instance status monitor (`55055`), recents synchronization, and `GoodLookinUI` monitor view. |
| `native/` | Thin platform SDK wrappers: AAX (`OpenGridder.mm`), AUv2 (`OpenGridderAU.cpp`), AUv3 (`OpenGridderAUv3.mm`), VST3 (`OpenGridderVST3.mm`), VST2 (`OpenGridderVST2.cpp`), CLAP (`OpenGridderCLAP.mm`), and Cocoa Server Launcher (`server_app/main.m`). |

---

## Building & Testing

```bash
# Run unit tests across workspace
cargo test --workspace

# Build release binaries
cargo build --release --workspace
```

For release packaging, codesigning, and Apple notarization, see [`scripts/package-release-macos.sh`](scripts/package-release-macos.sh) and the [User Manual](Docs/UserManual.md).

---

## Notices & License

OpenGridder is proprietary software; see `LICENSE`. AAX, Control|24, C|24, and Pro Tools are trademarks of Avid Technology, Inc. VST is a trademark of Steinberg Media Technologies GmbH. Audio Units is a trademark of Apple Inc.
