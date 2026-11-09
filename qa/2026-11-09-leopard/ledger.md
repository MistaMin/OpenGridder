# Leopard RTAS/TDM BETA scope

Source: user request, build 38 (31f5f6f), legacy/client-core and legacy/native.
Release: build 44 (af85c90), GitHub Actions run 42605473800, artifact
`OpenGridder-Legacy-BETA-0.1.1-alpha-build44-macos-ppc-i386-UNTESTED.zip`
(SHA-256 `1988a6c3001fd63eaffdceb68d2be602b57f3dbc86d4eadb51257c97c47728f0`).
Targets: Pro Tools 8.0.x LE/M-Powered on OS X 10.5.8 (PPC and Intel), and
Pro Tools 8.0cs3 HD (TDM, HD Accel/DSP56321) on Leopard (PPC and Intel).

| ID | Item | Code | GUI | Test / evidence | Status |
|---|---|---|---|---|---|
| S1 | GCC compiles and links ppc/i386 against 10.5 SDK | scripts/leopard-toolchain.sh | none | prove-leopard-toolchain.sh: ppc, i386 and universal probes; lipo; per-slice SDK import audit | done |
| S2 | C99 client preserves v13 handshake/catalog/ADD/DEL/audio | legacy/client-c | unchanged | C unit/protocol tests incl. 20000 fuzz inputs and mock v13; live server catalog 2244 plugins; RESULT: PASS C client | done |
| S3 | Timestamped audio, latency acknowledgement, bypass, reconnect, no callback allocation | legacy/client-c | unchanged controls | runtime tests (empty pool/no allocation, variable blocks, late wet, acknowledged latency, profiles, 4096 selection); live AULowpass audio and latency 256 | done |
| S4 | Screen queue/header conversion and mouse/key forwarding | legacy/client-c | existing remote editor | editor tests (v13/full/partial, bounded queue/restart, mouse coalescing, key endian encoding); live editor 705x366 with drag updates | done |
| S5 | Same exported legacy C ABI including TDM profiles and Q1.23 bridge | legacy/client-c | unchanged | all 62 functions declared in opengridder.h/opengridder_tdm.h exported on ppc, i386 and native; TDM profile/Q1.23 tests; TDM harness all three modes | done |
| S6 | RTAS/TDM ppc+i386 bundles with 10.5 dependencies | legacy/native | unchanged | universal lipo; audit: all strong imports resolved in 85 Leopard SDK libraries on both slices; i386 classic dyld info (no dyld_stub_binder) | done |
| S7 | Pro Tools 8 API/host-port DMA compatibility | legacy/native | unchanged | only long-standing CEffectProcessRTAS/TDM, CPluginControl, GetDelaySamplesLong and DSP host-port calls are used (no 8.1/8.0.3-only feature); USE_HOST_PORT_DMA=1; SDK release notes place host-port DMA before 2004. No Pro Tools/HD card here | build-level done; hardware unverified |
| S8 | CPU-independent DSP56321 firmware unchanged | legacy/native/tdm | none | verify-tdm-engine.py: 654 words match Motorola ASM56300; harness all: emulated DSP, firmware capture/mailbox, actual image (RX=19177 TX=19140); rsrc identical on both slices | done |
| S9 | GitHub dispatch on local licensed runner, signed artifacts only | .github/workflows/leopard-ppc-beta.yml | none | run 42605473800 passed (checkout, tests, ppc+i386 builds, audit, sign, upload); artifact 2.6 MB contains only bundles, self-test, notices | done |
| S10 | Leopard package, configuration and catalog instructions | package-leopard-beta.sh | none | build 44 zip inspected; both bundles and universal self-test signed (Developer ID, sha1+sha256); SHA-256 verified; READ ME names hosts, client.json path and --list | done |
| S11 | Copy alongside build 38 on USB, verify checksum | release artifact | none | `/Volumes/32 GB/OpenGridder Alpha`: build44 zip + `READ ME FIRST - PPC and Intel Leopard.txt`; SHA256SUMS has build38+build44, both OK after cp -X and dot_clean -m | done |
| S12 | Execution on PPC/Leopard and Pro Tools/HD hardware | released binaries | real host | unavailable on this ARM64 Mac: no PowerPC/32-bit execution, no Pro Tools, no HD Accel card | unverified |
| S13 | Existing AU/VST3/CLAP/LV2/64-bit AAX preserved, no JUCE or 32-bit AAX | existing modern tree | unchanged | diff from 31f5f6f touches only legacy/, .github/workflows/ and qa/; no AAX/JUCE/VST3/CLAP/LV2 file changed; RTAS/TDM builds only | done |

Rust client unit tests still pass: 20 passed, 0 failed, 3 ignored
(OG_LEGACY_RUN_TESTS=1, i386 archive rebuilt).

No proprietary SDK text, headers or source is committed or uploaded. Local SDK
extracts and raw build logs stay in ignored build directories. This ledger stores
conclusions only.
