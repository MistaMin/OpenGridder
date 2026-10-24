# OpenGridder-Rust (`v1.2.1-marcosfork`)

An in-development **Rust** implementation of OpenGridder, using **AudioGridder
`v1.2.1-marcosfork`** (commit `559863f`) as a reference. The OpenGridder core is
**MIT licensed**, with **no JUCE** or GTK dependency. Optional SDK adapters retain
their external SDK licensing requirements. See [current build status](native/BUILD_STATUS.md).

## Credits & Attribution

- **Andreas Pohl** (`apohl79@gmail.com`) — Original creator and architect of [AudioGridder](https://github.com/apohl79/audiogridder) (`AG_PROTOCOL_VERSION = 13`, 4-socket real-time audio/screen/command architecture, sandbox isolation, and DAW plugin workflow).
- **Marcos Deida** (`MistaMin`) — Creator of the `v1.2.1-marcosfork` enhancements (per-plugin `1x`/`2x`/`4x` polyphase FIR oversampling with automatic PDC latency reporting, state serialization v7, server settings persistence fix), creator of the [GoodLookinUI](https://github.com/MistaMin/GoodLookinUI) hardware UI framework, and lead architect of the Rust rewrite.

---

## Workspace Architecture (9 Crates)

| Crate | Replaces (`audiogridder-fork`) | Description |
| :--- | :--- | :--- |
| `crates/og-common` | `Common/Source/*` | Binary wire protocol v13 (`HandshakeRequest`, `HandshakeResponse`, `AudioMessage`, `MessageType` including `SetPluginOversampling = 75`), `AudioRingBuffer`, `ChannelSet`, `ChannelMapper`, `ImageDiff`, `ServerPlugin`, `Metrics`, `Defaults`, mDNS (`_audiogridder._tcp.local.`). |
| `crates/og-dsp` | `Server/Source/Processor.*` (DSP) | Polyphase equiripple half-band FIR `Oversampler<f32>` & `Oversampler<f64>` (`1x`, `2x`, `4x`), multi-mono slot processing, and latency reporting. |
| `crates/og-goodlookinui` | `GoodLookinUI` + JUCE UI | Pure-Rust port of `GoodLookinUI` (`Design.h` CSV schema, validation, critically damped spring `Motion`, and procedural hardware vector rendering: `draw_panel`, `draw_screw`, `draw_key`, `draw_console_knob`, `draw_knob`) plus AudioGridder's `PluginButton` (`1x`/`2x`/`4x` badge) and `GenericEditor` console rack. |
| `crates/og-hosting` | JUCE `AudioPluginFormatManager` | Native macOS **AU v2** effect hosting (Rust/AudioToolbox), optional **VST3** effect hosting (external SDK ABI adapter), and LV2 discovery only. |
| `crates/og-screen` | `Server/Source/Screen*`, `Plugin/Source/ImageReader*` | RGBA delta diffing (`og_common::image_diff`) and PNG/raw frame encoding & decoding over the `screen` socket. |
| `crates/og-client` | `Plugin/Source/Client.*`, `AudioStreamer.hpp` | 4-socket client (`cmdOut`, `cmdIn`, `audio`, `screen`), lock-free `AudioStreamer<T>`, and remote RPC commands including `set_plugin_oversampling`. |
| `crates/og-server` | `Server/Source/*` | `AudioGridderServer` binary, `Processor`, `ProcessorChain`, `AudioWorker`, `ScreenWorker`, `Worker`, `SandboxMode`, and `ServerConfig` (with the PR #1354 `save_config()` persistence fix). |
| `crates/og-plugin` | `Plugin/Source/*` | DAW `PluginProcessor` (`Fx`, `Instrument`, `Midi`), `LoadedPlugin` state v7 (`oversampling_factor` at index 11), 16 automation slots, A/B comparison, `GoodLookinUI` `PluginEditor`, and VST3/LV2 C-ABI exports. |
| `crates/og-tray` | `PluginTray/Source/*` | `AudioGridderPluginTray` binary, TCP instance status monitor (`55055`), recents synchronization, and `GoodLookinUI` monitor view. |

---

## Building & Running Tests

**Native AAX development target:** a no-JUCE AAX adapter and a Rust callback/IO
runtime are now in `native/` and `og-plugin`. See [native build instructions](native/README.md).
PACE and Apple signing are verified. Pro Tools 10 / OS X 10.8.5, full AAX state
recall and broad third-party compatibility are not yet supported or qualified.
See [AAX preview release preparation](Docs/Release.md) and the
[current user manual](Docs/UserManual.md).

```bash
cargo test --workspace
cargo build --release --workspace
```


### Native server development build

The server now loads real macOS AU v2 and VST3 effects, with per-client child
processes, bounded audio transport, native parameters and state RPCs. See
[native/SERVER.md](native/SERVER.md) for universal Intel/Apple Silicon build and
startup instructions. This remains a development system: full AAX remote-state
recall/editor controls and the legacy compatibility matrix remain open.
