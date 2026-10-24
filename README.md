# OpenGridder-Rust (`v1.2.1-marcosfork`)

A complete, from-scratch **Rust** rewrite of **AudioGridder `v1.2.1-marcosfork`** (commit `559863f`) with **zero JUCE** and **zero GTK** dependencies, licensed under the **MIT License**.

## Credits & Attribution

- **Andreas Pohl** (`apohl79@gmail.com`) — Original creator and architect of [AudioGridder](https://github.com/apohl79/audiogridder) (`AG_PROTOCOL_VERSION = 13`, 4-socket real-time audio/screen/command architecture, sandbox isolation, and DAW plugin workflow).
- **Marcos Deida** (`MistaMin`) — Creator of the `v1.2.1-marcosfork` enhancements (per-plugin `1x`/`2x`/`4x` polyphase FIR oversampling with automatic PDC latency reporting, state serialization v7, server settings persistence fix), creator of the [GoodLookinUI](https://github.com/MistaMin/GoodLookinUI) hardware UI framework, and lead architect of the Rust rewrite.

---

## Workspace Architecture (9 Crates)

| Crate | Replaces (`audiogridder-fork`) | Description |
| :--- | :--- | :--- |
| `crates/og-common` | `Common/Source/*` | Binary wire protocol v13 (`HandshakeRequest`, `HandshakeResponse`, `AudioMessage`, `MessageType` including `SetPluginOversampling = 75`), `AudioRingBuffer`, `ChannelSet`, `ChannelMapper`, `ImageDiff`, `ServerPlugin`, `Metrics`, `Defaults`, mDNS (`_audiogridder._tcp.local.`). |
| `crates/og-dsp` | `Server/Source/Processor.*` (DSP) | Polyphase equiripple half-band FIR `Oversampler<f32>` & `Oversampler<f64>` (`1x`, `2x`, `4x`), multi-mono slot processing, and exact PDC latency calculation. |
| `crates/og-goodlookinui` | `GoodLookinUI` + JUCE UI | Pure-Rust port of `GoodLookinUI` (`Design.h` CSV schema, validation, critically damped spring `Motion`, and procedural hardware vector rendering: `draw_panel`, `draw_screw`, `draw_key`, `draw_console_knob`, `draw_knob`) plus AudioGridder's `PluginButton` (`1x`/`2x`/`4x` badge) and `GenericEditor` console rack. |
| `crates/og-hosting` | JUCE `AudioPluginFormatManager` | Native zero-JUCE plugin discovery and hosting for **AudioUnit (AU)** (`AudioToolbox` FFI on macOS), **VST3**, and **LV2** (`.lv2` / `manifest.ttl` / `lv2_descriptor`). |
| `crates/og-screen` | `Server/Source/Screen*`, `Plugin/Source/ImageReader*` | RGBA delta diffing (`og_common::image_diff`) and PNG/raw frame encoding & decoding over the `screen` socket. |
| `crates/og-client` | `Plugin/Source/Client.*`, `AudioStreamer.hpp` | 4-socket client (`cmdOut`, `cmdIn`, `audio`, `screen`), lock-free `AudioStreamer<T>`, and all 26 remote RPC commands including `set_plugin_oversampling`. |
| `crates/og-server` | `Server/Source/*` | `AudioGridderServer` binary, `Processor`, `ProcessorChain`, `AudioWorker`, `ScreenWorker`, `Worker`, `SandboxMode`, and `ServerConfig` (with the PR #1354 `save_config()` persistence fix). |
| `crates/og-plugin` | `Plugin/Source/*` | DAW `PluginProcessor` (`Fx`, `Instrument`, `Midi`), `LoadedPlugin` state v7 (`oversampling_factor` at index 11), 16 automation slots, A/B comparison, `GoodLookinUI` `PluginEditor`, and VST3/LV2 C-ABI exports. |
| `crates/og-tray` | `PluginTray/Source/*` | `AudioGridderPluginTray` binary, TCP instance status monitor (`55055`), recents synchronization, and `GoodLookinUI` monitor view. |

---

## Building & Running Tests

```bash
cargo test --workspace
cargo build --release --workspace
```
