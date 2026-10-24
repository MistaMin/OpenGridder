# Credits & Attribution

**OpenGridder (Rust Edition)** is a complete, JUCE-free Rust reimplementation of **AudioGridder** (`marcosfork-1.2.1`, commit `559863f`).

## Original Concept & Architecture
- **Andreas Pohl** (`@apohl79`) — Creator of [AudioGridder](https://github.com/apohl79/audiogridder), released under the MIT License. The network protocol (v13), 4-socket command/audio/screen streaming model, multi-mono channel mapping, sandbox architecture, and plugin chain design originate from his work.

## Fork Features, UI Toolkit & Rust Port
- **Marcos Deida** (`@MistaMin`) — Author of the `marcosfork-1.2.1` per-plugin 2x/4x oversampling architecture, settings persistence fixes, and creator of [GoodLookinUI](https://github.com/MistaMin/GoodLookinUI) (MIT License), used here for hardware-inspired console panels, knobs, and controls without any JUCE dependency.
