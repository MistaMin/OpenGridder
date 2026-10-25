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

## Controls and Hardware Interface

OpenGridder features a custom zero-JUCE hardware console interface designed with **GoodLookinDri**, rendering native high-performance Cocoa views across AudioUnit v3, AudioUnit v2, and VST3.

### Client Plugin Interface (720x420 Hardware Chassis)
The plugin interface mimics a dark brushed anodized analog console rack with corner hex fasteners, split into two functional sections:

#### 1. Left Control Panel (Routing & Profile)
- **Status LED Dot**: Real-time connection indicator (Green = connected, Red = offline / searching).
- **Server Address Readout**: Displays the bound server IP and port (`127.0.0.1:55056`).
- **Latency Profile Buttons (PDC)**: Four tactile buttons (`Low 128`, `Std 256`, `High 512`, `Studio 1024`). Clicking instantly switches the network streaming buffer size and notifies the host DAW to update its plugin delay compensation without audio drops.
- **Plugin Slot Selectors**: 4 dedicated slot buttons (`Slot 0` through `Slot 3`).
- **Oversampling Badges**: Clickable `1x`, `2x`, `4x` oversampling badges on each slot to toggle high-precision DSP oversampling on remote processors.
- **Master Bypass Switch**: Heavy-duty master bypass toggle button (`MASTER: ACTIVE` / `MASTER BYPASS: ENGAGED`). Passes sample-accurate dry audio while preserving PDC alignment in the host timeline.

#### 2. Right Hardware Console Strip (Processing & Telemetry)
- **Input Gain Knob**: Continuous rotary controller adjusting incoming audio gain from -12.0 dB to +12.0 dB (default: 0.0 dB). Drag vertically to adjust.
- **Wet/Dry Mix Knob**: Continuous rotary controller blending dry and remote DSP wet return from 0% to 100% (default: 100%).
- **Drive / Saturation Knob**: Rotary drive controller scaling harmonic warming from 1.0x to 10.0x (default: 2.0x).
- **Output Trim Knob**: Continuous rotary controller trimming master output levels from -12.0 dB to +12.0 dB (default: 0.0 dB).
- **Dual 20-Segment LED VU Meters**: Calibrated stereo LED ladder meters (-48 dBFS to +3 dBFS) with green/yellow/red gradient thresholds and dynamic peak hold indicators.
- **Telemetry Readout**: Real-time display of roundtrip packet delay and active buffer compensation in samples.

---

## Server Tools and Splash Screen

### Startup Splash Window
Upon launching **OpenGridder Server.app**, a hardware-styled floating splash screen appears:
- **Real-Time Progress Bar**: Shows plugin indexing progress across system audio units and VST3 directories.
- **Dynamic Status Label**: Displays the current plugin name and file path being scanned.
- **High-Speed Timestamp Caching**: Re-scanning skips unchanged `.vst3` and `.component` bundles by comparing file modification timestamps against `plugin_cache.json`. A library of over 1,700 plugins loads in less than 0.5 seconds on subsequent launches.
- **Skip Scan Button**: Allows instant bypass of scanning to launch directly into the server state. Automatically dismisses when the server is ready.

### Plugin Manager Tool (`Cmd-P`)
Access the Plugin Manager from the macOS menu bar status icon or by pressing `Cmd-P`:
- **Format Filter**: Segmented tabs to view `All`, `AudioUnit`, `VST3`, or `VST2` plugins.
- **Instant Search Field**: Filter thousands of plugins in real time by plugin name or manufacturer.
- **Plugin Details Table**: Displays Name, Format, Manufacturer, Category, and Validation Status.
- **Rescan All (`Cmd-R`)**: One-click command to flush the timestamp cache and trigger a full re-scan.

---

## Latency and Stability

Each client chain runs in an isolated child server worker process by default to contain third-party plugin crashes. In the event of a network hiccup or lost packet, the DSP transport smoothly crossfades to dry aligned audio to prevent clicks or dropouts.

## Legacy Compatibility and Vintage Mac Support

A separate legacy archive (`OpenGridder-0.1.0-alpha-legacy-vintage-mac.zip`) is available for vintage systems running OS X 10.6 Snow Leopard through 10.14 Mojave, containing 32-bit `i386` slices for Logic Pro 8/9 and vintage Cubase.

## Notices

The OpenGridder Rust core is MIT licensed. AAX and Pro Tools are trademarks of Avid Technology, Inc. VST is a trademark of Steinberg Media Technologies GmbH. Audio Units is a trademark of Apple Inc.
