# OpenGridder v0.1.1-alpha — New Chat Handover Briefing

> [!CAUTION]
> **CRITICAL REALITY CHECK: THIS IS EARLY ALPHA SOFTWARE. HARDLY ANYTHING WORKS RELIABLY YET.**
> Do NOT assume features work just because unit tests pass or protocol packets deserialize. In real-world live DAWs (REAPER, Pro Tools), the software is highly unstable, the GUI behaves erratically, audio streaming is fragile, and the overall user experience is far from production-ready.

---

## 1. Ground Truth & Honest Current Status

### What is actually broken or barely functioning:
1. **Host Stability & Live DAW Behavior**:
   - In **REAPER**: The GUI behaves weirdly (sluggish interaction, erratic window redrawing, menu popup quirks).
   - In **Pro Tools (AAX)**: Historically showed a blank/generic 79px host parameter strip due to `AAX_eProperty_UsesClientGUI`. While custom Cocoa view code was added (`720x420`), Pro Tools has been loading an **old 17:33 root binary from `/Library/...`** instead of the new build, so the live experience in Pro Tools is still unverified and broken.
2. **Audio Streaming & Latency Compensation**:
   - Real-time audio transmission over the socket is brittle under host playback.
   - Dynamic latency changes reported by remote plugins can cause dropouts, ring-buffer desync, or host freezing.
3. **Remote Plugin Editor (Screen Streaming)**:
   - Clicking a plugin to open its native GUI remotely (`og_native_edit_plugin`) does not reliably display the remote window or capture user clicks/keyboard events cleanly.
4. **Binary & Installation Clashes**:
   - macOS hosts look in both `/Library/` (system, root-owned) and `~/Library/` (user).
   - Old `17:33` binaries exist in `/Library/...` and require `sudo` to replace.
   - Pro Tools strictly scans `/Library/Application Support/Avid/Audio/Plug-Ins/` and ignores `~/Library/`, meaning Pro Tools runs obsolete code until manual elevation is performed.

### What has been wired up so far (low-level / protocol):
- **Handshake Wire Alignment**: `HandshakeRequest::WIRE_SIZE` is now 64 bytes (matching AudioGridder v13 C++ struct alignment for `activeChannels`).
- **Catalog Decoding**: Server sends 2,192 plugins via MsgPack; decoding logic is implemented in `msg.get_msgpack()`.
- **Dynamic Rack Structure**: `og-goodlookinui` has unbounded slots, per-plugin oversampling (`1x`, `2x`, `4x`), and bypass/unload buttons.
- **PACE Eden Signing**: AAX signed via `wraptool sign` using Developer ID `Marcos Deida (ZQYU67JUBH)`.
- **Cargo Unit Tests**: 31 unit tests pass in isolation (`cargo test --lib`).

---

## 2. Environment & Repository Snapshot

- **Workspace**: `/Users/marcosdeida/Projects/OpenGridder-Rust`
- **Active Branch**: `perf/tdm-optimization`
- **Latest Commit**: `043c6e9` (*pushed to origin/perf/tdm-optimization*)
- **Avid AAX SDK**: `/Volumes/Developer/Dev Projects/AAX Tools/aax-sdk-2-8-1`
- **Local AudioGridder Server**: Running on `127.0.0.1:55056`
- **Core Principle**: **Zero JUCE**. Pure Rust core with native C/C++/Objective-C Cocoa wrappers only.

---

## 3. The Immediate Blocker: System Binary Overwrite

Before anything will work in Pro Tools, the old root-owned binary from `17:33` in `/Library` must be overwritten:

```bash
sudo rm -rf "/Library/Audio/Plug-Ins/VST3/OpenGridder.vst3" \
            "/Library/Audio/Plug-Ins/Components/OpenGridder.component" \
            "/Library/Application Support/Avid/Audio/Plug-Ins/OpenGridder.aaxplugin" && \
sudo ditto "/Users/marcosdeida/Projects/OpenGridder-Rust/target/vst3-universal/OpenGridder.vst3" "/Library/Audio/Plug-Ins/VST3/OpenGridder.vst3" && \
sudo ditto "/Users/marcosdeida/Projects/OpenGridder-Rust/target/au-universal/OpenGridder.component" "/Library/Audio/Plug-Ins/Components/OpenGridder.component" && \
sudo ditto "/Users/marcosdeida/Projects/OpenGridder-Rust/target/aax-pace-signed/OpenGridder.aaxplugin" "/Library/Application Support/Avid/Audio/Plug-Ins/OpenGridder.aaxplugin"
```

---

## 4. Key Files & Components

- **Native AAX Wrapper**: [`native/aax/OpenGridder.mm`](file:///Users/marcosdeida/Projects/OpenGridder-Rust/native/aax/OpenGridder.mm)
- **Native VST3 Wrapper**: [`native/vst3/OpenGridderVST3.cpp`](file:///Users/marcosdeida/Projects/OpenGridder-Rust/native/vst3/OpenGridderVST3.cpp)
- **Native AU Wrapper**: [`native/au/OpenGridderAU.mm`](file:///Users/marcosdeida/Projects/OpenGridder-Rust/native/au/OpenGridderAU.mm)
- **Network Wire Protocol**: [`crates/og-common/src/message.rs`](file:///Users/marcosdeida/Projects/OpenGridder-Rust/crates/og-common/src/message.rs)
- **Client Networking & State**: [`crates/og-client/src/client.rs`](file:///Users/marcosdeida/Projects/OpenGridder-Rust/crates/og-client/src/client.rs)
- **GoodLookin Cocoa Rack UI**: [`crates/og-goodlookinui/src/lib.rs`](file:///Users/marcosdeida/Projects/OpenGridder-Rust/crates/og-goodlookinui/src/lib.rs)
- **DSP Polyphase Oversampling**: [`crates/og-dsp/src/lib.rs`](file:///Users/marcosdeida/Projects/OpenGridder-Rust/crates/og-dsp/src/lib.rs)
- **Plugin Core Engine**: [`crates/og-plugin/src/lib.rs`](file:///Users/marcosdeida/Projects/OpenGridder-Rust/crates/og-plugin/src/lib.rs)

---

## 5. Next Focus Areas

1. **Verify Live Pro Tools AAX GUI**: Ensure the Cocoa view is actually drawn (720x420) without crashing the Pro Tools thread or reverting to 79px generic host view.
2. **Fix REAPER UI Quirks**: Debug the Cocoa view lifecycle, event loop, mouse tracking, and window resizing so the GUI feels smooth and stable.
3. **End-to-End Audio Buffer Stability**: Stress test audio streaming with real plugin processing in the DAW timeline.
4. **Remote Plugin Window Streaming**: Stabilize screen capture transmission and mouse event forwarding.
