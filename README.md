# AudioGridder Open

Active rewrite of AudioGridder on OpenPluginSDK (`/Volumes/Developer 2/Dependencies/OpenPluginSDK`). Version `2.0.0-alpha.1`. The shipping JUCE fork stays at `../audiogridder` (1.3.5) until this tree is qualified in a DAW.

This alpha is a Rust workspace:

| Crate | Role |
|---|---|
| `ag-plugin` | Insert. Input gain, output gain, drive, wet/dry, bypass, latency profiles, VU. Built as an `openplugin::Plugin`. |
| `ag-protocol` | AudioGridder wire protocol v13 as explicit little-endian bytes. |
| `ag-server` | One-socket host. It runs one OpenPluginSDK gain. It does not load AU or VST3 plugins. |
| `ag-legacy` | Separate binary. Q1.23 PCM and a 64-frame / 32-slot / 16-block TDM schedule. Not a `.dpm`. |

```sh
cargo test --workspace
cargo tree -p ag-server
```

`ag-server` does not depend on `ag-legacy`. The legacy symbol `ag_legacy_pcm_api_version` is in `target/debug/ag-legacy` only.

## What this alpha does not do

- OpenPluginSDK's CLAP, VST3, AU, and AAX adapters do not export a plugin entry point yet, so nothing here is a bundle a DAW can load. The format SDKs are on `/Volumes/Developer 2/SDKs` (AAX 2.8.1, AudioUnitSDK, clap-sdk, PT9 RTAS/TDM) and `/Volumes/Developer 2/Dependencies/clap-wrapper`.
- The server does not scan or host third-party plugins yet.
- The editor is a draw list. GoodLookinDri, the window-side toolkit, is at `/Volumes/Developer 2/Dependencies/GoodLookinDri`.
- The legacy crate does not link the PT9 SDK yet. That SDK is `/Volumes/Developer 2/SDKs/PT_90_PlugInSDK-AAX-Tools`. Prebuilt 10.7/10.8 deps are in `/Volumes/Developer 2/Dependencies/audiogridder-deps`. No Pro Tools qualification has been run.
- Screen streaming, the four-socket session, and sandboxing are not in this alpha. The handshake answer uses the listen port, and that same socket carries one audio block.

Protocol v13 has no authentication. It is for a trusted LAN.
