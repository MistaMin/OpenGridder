# OpenGridder 0.3.2 Release

### Highlights:
- **macOS Accessibility Authorization**: Integrated system prompt and tray menu controls for macOS Accessibility (`AXIsProcessTrustedWithOptions`) ensuring seamless synthetic mouse and keyboard event forwarding to hosted plugin windows without system event drops.
- **Robust AU Window Teardown**: Explicit teardown and detachment of plugin Cocoa editor windows prior to AudioUnit instance disposal, completely preventing `SIGSEGV` faults during editor close or plugin unload.
- **Pro Tools AAX Page Tables**: Bundled and PACE Eden-signed `OpenGridderPages.xml` ensuring complete control surface integration across Avid S6, S3, ICON D-Control/D-Command, and Focusrite/Avid C|24.
- **Multi-Stream DSP Core Sharing (TDM Engine)**: Fully optimized Motorola DSP56321 assembly firmware allocating 4 concurrent stereo streams per core (8 channels per DSP, up to 72 stereo instances across an 18-core Pro Tools HD Accel card) utilizing shared on-chip 24-bit X/Y RAM ring buffering.
- **Hardware Console Integration (Focusrite Control|24 & Avid C|24)**: Complete 24-channel rotary encoder and switch mapping via custom `FrTL` Pro Tools XML Page Tables, alongside full surface support for `HgTL` (ICON D-Control / D-Command), `PcTL` (ProControl), and `MkTL` (Command|8 / HUI).
- **Zero-Overhead Pure-Native Architecture**: Zero-overhead pure-Rust audio streaming core paired with native C/C++/Objective-C bindings directly to Steinberg, Apple, and Avid SDKs with zero runtime bloat.
- **GoodLookin Hardware Console UI**: 3-bank collapsible brushed anodized aluminum rack, 2x-wide precision analog ballistic VU meters (`864×420`), dual VU calibration (`-18 dBFS` broadcast / `-3 dBFS` headroom), and real-time `[IN | OUT]` metering source toggles.
- **Residual ADAA & Polyphase Oversampling**: HybridEQ-matched non-linear drive saturation with anti-derivative anti-aliasing (ADAA) and selectable `1x`, `2x`, and `4x` linear-phase oversampling per plugin slot.
- **Dynamic Plugin Chain & Slot Routing**: Modular insert rack supporting unlimited chain length, categorized native Cocoa browser menu, individual plugin bypass, and low-latency remote GUI streaming.
- **Server Startup Splash Window**: Centered floating hardware splash screen with real-time scan progress, live plugin name display, and skip scan button; auto-dismisses upon server readiness.
- **Sub-Second Plugin Scanning**: Persistent cache (`plugin_cache.json`) scans 1,700+ host plugins in under 500 ms with live background verification.
- **AudioUnit v3 (AUv3)**: Modern macOS App Extension (`OpenGridderAUv3.appex`) embedded inside `OpenGridder Server.app`, compatible with Logic Pro and GarageBand on macOS 11 through macOS 26/27.
- **VST3 Universal**: Universal binary (`OpenGridder.vst3`, arm64 + x86_64) supporting Steinberg Cubase, Nuendo, Studio One, Bitwig, and Reaper with dynamic `kLatencyChanged` delay compensation and `IPlugView` GoodLookin UI.
- **AudioUnit v2 (AUv2)**: Universal binary (i386 + x86_64 + arm64) with Carbon Component Manager 'thng' resource for Logic Pro 8, Logic Pro 9 (32/64-bit), and modern DAWs, supporting `kAudioUnitProperty_CocoaUI`.
- **VST 2.4**: Universal binary (i386 + x86_64 + arm64) for Cubase 4/5/6/7, Nuendo, and legacy Mac DAWs.
- **AAX Native**: Universal binary (arm64 + x86_64) PACE Eden-signed for Pro Tools 11+ and Pro Tools Ultimate.
- **OpenGridder Server.app**: Native macOS Cocoa application bundle with custom high-res icon, system menu bar status controls, dock integration, log viewer, and background daemon management.
- **Windows 7 Server**: x86_64 server package with custom Win32 synchronization (`WaitOnAddress`) and cryptographic (`ProcessPrng`) forwarder shims.
- **Apple Notarization & Codesigning**: Apple Developer ID signed, notarized with `xcrun notarytool`, and stapled for both `.pkg` and `.dmg`.
- **4 Latency Profiles**: Low (128), Standard (256), High (512), and Studio (1024) samples with automatic dynamic PDC.
