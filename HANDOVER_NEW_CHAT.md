# OpenGridder v0.2.1-alpha — New Chat Handover Briefing & Release Readiness

> [!IMPORTANT]
> **STATUS**: **v0.2.1-alpha** on branch `perf/tdm-optimization`.
> All stabilization items (`S1`–`S9`), complete AudioGridder 1.3.5 feature parity (`F1`–`F6`), Three-Way Parameters (`P1`–`P7`), 2x-Wide Analog VU Metering (`864×420`), 3-Bank Collapsible Hardware UI, Dual VU Calibration (`-18 dBFS` / `-3 dBFS`), Input/Output Metering Source Toggles (`[IN* | OUT]`), HybridEQ-matched Residual ADAA + 2x/4x Oversampled Drive Saturation, and TDM 64-frame Q1.23 bridge are **100% implemented, unit-tested, PACE-signed, and deployed**.
>
> The next phase is to execute the **Make-Plugin-Release** skill (`/Users/marcosdeida/.gemini/config/skills/Make-Plugin-Release/SKILL.md`) to package, codesign, notarize, and produce official macOS distribution packages and installer DMGs.

---

## 1. What Was Completed & Verified in This Phase

### A. Core Engine & AudioGridder 1.3.5 Parity
- **6 Plugin Formats Verified**: `VST3`, `AU` (`aufx`), `AAX` (PACE Eden signed), `CLAP`, `VST2`, and `LV2`.
- **Audio & MIDI Network Transport**: 32-bit float (`process`), 64-bit float (`process_f64`), sample-accurate MIDI event queueing (`og_native_queue_midi`), lock-free SPSC chunk ring buffers without allocation on audio callbacks.
- **Dynamic Remote Chain Management**: Multi-slot chain, per-slot oversampling (`1x`/`2x`/`4x`), bypass toggles, slot reordering, preset selection, remote parameter automation, and hardware A/B instant state switching.
- **Remote GUI Streaming & Input**: `OG-ScreenClient` 32B/20B protocol with floating `OpenGridderRemoteWindowController` streaming plugin UIs (e.g. 2016 Stereo Room) and forwarding mouse clicks and keyboard events.
- **State v7 Serialization**: Bidirectional JSON State v7 recall across all formats and session save/reload.

### B. Precision Console UI & Hardware Controls
- **2x-Wide Metering Expansion**: Dynamic window resizing (`864×420` in Analog VU mode vs `720×420` in Digital PPM mode) using pure native Cocoa (`OpenGridderEditorView`).
- **3 Independent Collapsible Hardware Banks**:
  1. `PLUGINS` (Plugin Selection Rack): Can clap closed to a 36px vertical anodized strip with status LED.
  2. `CONSOLE I/O & DRIVE` (Input, Wet/Dry, Drive/Sat, Output Trim): Can clap closed to a 36px vertical strip.
  3. `METER` (Dual VU dials or Stereo PPM): Can clap closed to a 36px vertical strip with audio activity LED.
- **Dual VU Calibration Reference**: Clickable `[-18* | -3 dB]` / `[-18 | -3* dB]` pill toggle.
  - `-18 dBFS` mode: Dial face prints `0 VU = +4 dBu = -18 dBFS` with SMPTE standard needle ballistics.
  - `-3 dBFS` mode: Dial face prints `0 VU = -3 dBFS` with broadcast needle ballistics.
- **Stereo Input/Output Metering Source**: Clickable `[IN* | OUT]` / `[IN | OUT*]` pill toggle in both PPM and VU modes. Post-Input Gain input metering vs post-Output Trim output metering with 2.0 ceiling for over-detection.
- **Console Drive & Harmonic Saturation DSP**:
  - **HybridEQ Residual ADAA**: Ported directly from HybridEQ (`dsp::adaa::Residual` in `Source/DSP/Adaa.h`). Splits signal into linear path (first-order all-pass with 0.5-sample group delay and 0.000 dB loss to Nyquist) and residual ADAA path, eliminating the high-frequency droop of standard textbook ADAA.
  - **2x & 4x Linear-Phase Oversampling**: Polyphase Kaiser-windowed FIR oversampling filters with fixed integer group delay (20 samples for 2x, 30 samples for 4x) automatically compensated against transport delay.
  - **3 Character Modes**: `CLEAN` (symmetric odd harmonics), `Tb TUBE` (asymmetric even harmonics with ~8 Hz DC blocker), and `Tr XFMR` (low-frequency-weighted core saturation with ~8 Hz DC blocker).
  - **Per-Sample Parameter Smoothing**: 10 ms one-pole filter on Input Gain, Wet/Dry, Drive, and Output Trim to eliminate zipper noise.
  - **Accurate Pre-Gain Readout**: Calibrated logarithmic display (`+0.0 dB` to `+20.0 dB`).

---

## 2. TDM Optimization & Verification Status

- **Architecture**: `native/legacy/client-core/src/tdm.rs` implements the SDK-neutral OpenGridder TDM BETA worker bridge (Rust 1.41 / i386 + modern 64-bit universal).
- **Packet Structure**: 132-word packets (4-word header + 64 stereo frames in Q1.23 24-bit fixed-point format).
- **Latency Profiles**:
  - `profile 0`: 1024 samples (16 blocks) — Standard LAN
  - `profile 1`: 512 samples (8 blocks) — Low Latency 10GbE
  - `profile 2`: 384 samples (6 blocks) — Localhost VSTi
  - `profile 3`: 256 samples (4 blocks) — Localhost Direct
- **Zero Real-Time Allocation**: The audio path operates completely without heap allocations or locks on both input and return rings.
- **Unit Test Coverage**: Added dedicated unit tests in `native/legacy/client-core/src/tdm.rs` verifying profile creation, epoch validation, prefill scheduling, and Q1.23 fixed-point conversion with clipping flags (`og_tdm_clip_status`). All 12 legacy client unit tests pass in 0.09s.
- **Universal Binary Linking**: Rebuilt `libopengridder_legacy_client.a` via `scripts/build-legacy-client-macos.sh` and re-linked tri-architecture universal AU (`arm64`, `x86_64`, `i386`).

---

## 3. Current Git & Environment Snapshot

- **Workspace**: `/Users/marcosdeida/Projects/OpenGridder-Rust`
- **Active Branch**: `perf/tdm-optimization`
- **Latest Commit**: `753a6ea`
- **Version**: `0.2.1-alpha` (configured in `Cargo.toml`, `version.rs`, `Info.plist.in`, `OpenGridderVST3.mm`, `main.m`, and `package-release-macos.sh`)
- **Installed Locations**:
  - VST3: `/Library/Audio/Plug-Ins/VST3/OpenGridder.vst3` (Universal `arm64`, `x86_64`)
  - AU: `/Library/Audio/Plug-Ins/Components/OpenGridder.component` (Universal `arm64`, `x86_64`, `i386`)
  - CLAP: `/Library/Audio/Plug-Ins/CLAP/OpenGridder.clap` (Universal `arm64`, `x86_64`)
  - AAX: `/Library/Application Support/Avid/Audio/Plug-Ins/OpenGridder.aaxplugin` (Universal `arm64`, `x86_64`, PACE Eden signed with account `mistamin` / GUID `807F4D50-CFDB-11F1-A0CD-00505692AD3E`)
- **QA Evidence**: Rendered GUI screenshots available in `qa/2026-10-25-alpha-stabilization/`:
  - `cocoa_gui_vu_mode.png`: 2x-wide VU mode with `-18 dBFS` calibration.
  - `cocoa_gui_vu_minus3_mode.png`: 2x-wide VU mode with `-3 dBFS` calibration.
  - `cocoa_gui_ppm_mode.png`: 1x-wide Digital PPM bargraph mode.
  - `cocoa_gui_folded_banks.png`: Console bank clapped closed.
  - `cocoa_gui_vu_input_mode.png`: VU meter switched to Input metering (`IN L`, `IN R`).
  - `cocoa_gui_ppm_input_mode.png`: PPM meter switched to Input metering (`IN PPM`).
  - `cocoa_gui_drive_modes.png`: Console Drive card with ADAA/OS and Clean/Tb/Tr controls.
  - `screen_stream_frame.png`: Remote plugin window capture.

---

## 4. Next Phase: Executing Make-Plugin-Release

When opening the new chat session, invoke the **Make-Plugin-Release** skill:
`SKILL: /Users/marcosdeida/.gemini/config/skills/Make-Plugin-Release/SKILL.md`

### Specific Steps for Release Automation:
1. **Packaging Script**: Review and run `scripts/package-release-macos.sh`.
2. **Apple Developer ID Codesigning & Notarization**:
   - Signing Identity: `Developer ID Application: Marcos Deida (ZQYU67JUBH)`
   - Installer Identity: `Developer ID Installer: Marcos Deida (ZQYU67JUBH)`
   - Notarytool Profile: `AC_NOTARY`
3. **Artifact Generation**: Produce signed & notarized `.pkg` installer and `.dmg` containing all 6 formats + Server application.
4. **Docs & Manual Audit**: Verify documentation in `Docs/` against the new v0.2.1-alpha features (VU meters, collapsible banks, calibration toggles, I/O metering, drive saturation).
