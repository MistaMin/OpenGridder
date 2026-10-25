# OpenGridder 0.1.1-alpha - Multi-Format Release

### Highlights:
- **AudioGridder Feature Parity & Unlimited Plugin Chain**: Dynamic slot rack supporting arbitrary length plugin chains, with native Cocoa categorized plugin browser menu, per-plugin bypass, per-plugin oversampling (`1x`, `2x`, `4x`), and remote GUI editor triggering.
- **Pro Tools Custom GUI Integration**: Full `AAX_CEffectGUI` implementation bringing the GoodLookin rack interface directly into Avid Pro Tools with active remote plugin management and rotary controls.
- **Zero JUCE Architecture**: Pure Rust core with clean native C/C++/Objective-C SDK wrappers across all targets.
- **GoodLookinDri Hardware UI**: Custom brushed anodized aluminum console interface across AUv3, AUv2 Cocoa UI, and VST3 with rotary controls, clickable oversampling badges (1x/2x/4x), and animated dual stereo LED ladder VU meters.
- **Server Startup Splash Window**: Centered floating splash screen with real-time scan progress, live plugin name display, and skip scan button; auto-dismisses upon server readiness.
- **Plugin Manager Tool (Cmd-P)**: Native macOS management window with instant search, format filtering (All/AU/VST3/VST2), plugin validation table, and one-click Rescan All (Cmd-R).
- **Sub-Second Plugin Scanning**: Persistent timestamp caching (`plugin_cache.json`) scans 1,700+ plugins in under 500 ms.
- **AudioUnit v3 (AUv3)**: Modern macOS App Extension (`OpenGridderAUv3.appex`) embedded inside `OpenGridder Server.app`, compatible with Logic Pro and GarageBand on macOS 11 through macOS 26/27.
- **VST3**: Universal binary (`OpenGridder.vst3`, arm64 + x86_64) supporting Steinberg Cubase, Nuendo, Studio One, Bitwig, and Reaper with dynamic `kLatencyChanged` delay compensation and `IPlugView` GoodLookin UI.
- **AudioUnit v2 (AUv2)**: Universal binary (i386 + x86_64 + arm64) with Carbon Component Manager 'thng' resource for Logic Pro 8, Logic Pro 9 (32/64-bit), and modern DAWs, supporting `kAudioUnitProperty_CocoaUI`.
- **VST 2.4**: Universal binary (i386 + x86_64 + arm64) for Cubase 4/5/6/7, Nuendo, and legacy Mac DAWs.
- **AAX Native**: Universal binary (arm64 + x86_64) PACE Eden-signed for Pro Tools 11+ and Pro Tools Ultimate.
- **OpenGridder Server.app**: Native macOS Cocoa application bundle with custom high-res icon, system menu bar status controls, dock integration, log viewer, and background daemon management.
- **Windows 7 Server**: x86_64 server package with custom Win32 synchronization (`WaitOnAddress`) and cryptographic (`ProcessPrng`) forwarder shims.
- **Notarization & Codesigning**: Apple Developer ID signed, notarized with `xcrun notarytool`, and stapled for both `.pkg` and `.dmg`.
- **4 Latency Profiles**: Low (128), Standard (256), High (512), and Studio (1024) samples with automatic dynamic PDC.
