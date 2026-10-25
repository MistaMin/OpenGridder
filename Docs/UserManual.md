# OpenGridder 0.1.0-alpha - Multi-Format Release

## Release scope

This release contains the zero-JUCE OpenGridder multi-format audio plugins and native applications.
Supported formats include:
- **AudioUnit v3 (AUv3)**: Modern macOS App Extension (`OpenGridderAUv3.appex`) embedded inside `OpenGridder Server.app`, compatible with Logic Pro, GarageBand, and all modern AUv3 hosts.
- **AudioUnit v2 (AUv2)**: Universal binary (i386 + x86_64 + arm64) supporting Logic Pro 8 (via Carbon Component Manager 'thng' resource), Logic Pro 9 (32-bit and 64-bit), and modern macOS DAWs.
- **VST3**: Universal binary (`OpenGridder.vst3`, arm64 + x86_64) supporting Steinberg Cubase, Nuendo, PreSonus Studio One, Bitwig Studio, and Cockos Reaper.
- **VST 2.4**: Universal binary (i386 + x86_64 + arm64) supporting vintage Cubase (4/5/6/7), Nuendo, and legacy Mac DAWs.
- **AAX Native**: Universal binary (arm64 + x86_64) signed with PACE Eden and Apple Developer ID for Pro Tools 11+ and Pro Tools Ultimate.
- **OpenGridder Server App**: Native macOS Application (`OpenGridder Server.app`) featuring system menu bar status controls, dock integration, log viewer, and background daemon lifecycle.
- **Windows 7 Server**: x86_64 server package with custom Win32 synchronization and cryptographic forwarder shims (`WaitOnAddress`, `ProcessPrng`).

Rust implements the DSP engine, bounded lockless transport, and protocol v13 networking. Thin SDK adapters provide AUv3, AUv2, VST3, VST2, and AAX interfaces with **zero JUCE dependencies**.

## Selectable Latency Profiles

OpenGridder features four user-selectable latency profiles with dynamic Plugin Delay Compensation (PDC):
- **Profile 0 (Low - 128 samples)**: Direct ultra-low latency localhost tracking and performance.
- **Profile 1 (Standard - 256 samples)**: Default balanced latency profile for local and high-speed LAN streaming.
- **Profile 2 (High - 512 samples)**: Standard LAN safety profile for complex multi-channel chains.
- **Profile 3 (Studio - 1024 samples)**: Relaxed high-buffer studio safety profile for remote mixing sessions.

Hosts automatically receive real-time delay compensation notifications via:
- VST3: `kLatencyChanged` component restart
- VST2: `audioMasterIOChanged`
- AUv2 & AUv3: `kAudioUnitProperty_Latency` and `AUAudioUnit.latency`
- AAX: `SetCycleCount`

## Installation and Quick Start

Quit your DAW before installing. The notarized macOS installer (`OpenGridder-0.1.0-alpha-macos.pkg`) installs:
- `OpenGridder.vst3` into `/Library/Audio/Plug-Ins/VST3`
- `OpenGridderAUv3.appex` registered via `OpenGridder Server.app/Contents/PlugIns`
- `OpenGridder.component` into `/Library/Audio/Plug-Ins/Components`
- `OpenGridder.vst` into `/Library/Audio/Plug-Ins/VST`
- `OpenGridder.aaxplugin` into `/Library/Application Support/Avid/Audio/Plug-Ins`
- `OpenGridder Server.app` into `/Applications`
- Documentation and licenses into `/Library/Application Support/OpenGridder`

For Windows 7 SP1+, extract `OpenGridder-0.1.0-alpha-windows7-x86_64.zip` to any folder. Ensure the companion shims (`api-ms-win-core-synch-l1-2-0.dll` and `bcryptprimitives.dll`) remain in the same directory as `AudioGridderServer.exe`.

### Launching OpenGridder Server

1. Open `/Applications` in Finder and double-click **OpenGridder Server.app** (or launch from Launchpad).
2. The OpenGridder icon appears in the macOS top system menu bar. Click it to view status, view logs, or restart.
3. Open your DAW (Logic Pro, Pro Tools, Cubase, Studio One, Reaper) and insert **OpenGridder** on any audio track.
4. Select your desired plugin slot and latency profile.

For command-line server operation or custom plugin scoping, run directly:
```bash
"/Applications/OpenGridder Server.app/Contents/MacOS/OpenGridderServer" --au AudioUnit:aufx:lpas:appl
```

## Connecting Remote Studio Machines

By default the server binds to `127.0.0.1:55056`. Client plugins read connection settings from:
`~/Library/Application Support/OpenGridder/client.json`

Example configuration:
```json
{"server":"127.0.0.1:55056","transport_samples":256}
```

For a separate studio computer on the local network:
1. Start the server with:
   ```bash
   "/Applications/OpenGridder Server.app/Contents/MacOS/OpenGridderServer" --bind 0.0.0.0 --port 55056
   ```
2. Update `client.json` on the DAW machine with the server machine's local IP address.
3. The protocol is intended for trusted studio networks; never expose server ports to the public Internet.

## Controls

- **Master Bypass**: Defaults to Active. When bypassed, passes dry audio while preserving plugin delay compensation alignment.
- **Plugin Slot**: Values 0 (no effect) through 63 select discovered server effects by catalog index.
- **Latency Profile**: Selects between Low (128), Standard (256), High (512), and Studio (1024) samples.

## Latency and Stability

Each client chain runs in an isolated child server worker process by default to contain third-party plugin crashes. In the event of a network hiccup or lost packet, the DSP transport smoothly crossfades to dry aligned audio to prevent clicks or dropouts.

## Legacy Compatibility and Vintage Mac Support

A separate legacy archive (`OpenGridder-0.1.0-alpha-legacy-vintage-mac.zip`) is available for vintage systems running OS X 10.6 Snow Leopard through 10.14 Mojave, containing 32-bit `i386` slices for Logic Pro 8/9 and vintage Cubase.

## Notices

The OpenGridder Rust core is MIT licensed. AAX and Pro Tools are trademarks of Avid Technology, Inc. VST is a trademark of Steinberg Media Technologies GmbH. Audio Units is a trademark of Apple Inc.
