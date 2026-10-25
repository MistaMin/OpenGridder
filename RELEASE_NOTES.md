# OpenGridder 0.1.0-alpha - Multi-Format Release

### Highlights:
- **Zero JUCE Architecture**: Pure Rust core with clean C/C++ SDK wrappers across all targets.
- **AudioUnit v3 (AUv3)**: Modern macOS App Extension (`OpenGridderAUv3.appex`) embedded inside `OpenGridder Server.app`, compatible with Logic Pro and GarageBand on macOS 11 through macOS 26/27.
- **VST3**: Universal binary (`OpenGridder.vst3`, arm64 + x86_64) supporting Steinberg Cubase, Nuendo, Studio One, Bitwig, and Reaper with dynamic `kLatencyChanged` delay compensation.
- **AudioUnit v2 (AUv2)**: Universal binary with Carbon Component Manager 'thng' resource for Logic Pro 8, Logic Pro 9 (32/64-bit), and modern DAWs.
- **VST 2.4**: Universal binary (i386 + x86_64 + arm64) for Cubase 4/5/6/7, Nuendo, and legacy Mac DAWs.
- **AAX Native**: Universal binary (arm64 + x86_64) PACE Eden-signed for Pro Tools 11+ and Pro Tools Ultimate.
- **OpenGridder Server.app**: Native macOS Cocoa application bundle with custom high-res icon, system menu bar status controls, dock integration, log viewer, and background daemon management.
- **Windows 7 Server**: x86_64 server package with custom Win32 synchronization (`WaitOnAddress`) and cryptographic (`ProcessPrng`) forwarder shims.
- **Notarization & Codesigning**: Apple Developer ID signed, notarized with `xcrun notarytool`, and stapled for both `.pkg` and `.dmg`.
- **4 Latency Profiles**: Low (128), Standard (256), High (512), and Studio (1024) samples.
