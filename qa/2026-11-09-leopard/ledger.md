# Leopard RTAS/TDM BETA scope

Source: user request, build 38 (31f5f6f), legacy/client-core and legacy/native.

| ID | Item | Code | GUI | Test / evidence | Status |
|---|---|---|---|---|---|
| S1 | GCC compiles and links ppc/i386 against 10.5 SDK | scripts/leopard-toolchain.sh | none | probe, lipo, dependency audit | in progress |
| S2 | C99 client preserves v13 handshake/catalog/ADD/DEL/audio | legacy/client-c | unchanged | Rust tests; C protocol/live equivalent | todo |
| S3 | Timestamped audio, latency acknowledgement, bypass, reconnect, no callback allocation | legacy/client-c | unchanged controls | native unit and differential tests | todo |
| S4 | Screen queue/header conversion and mouse/key forwarding | legacy/client-c | existing remote editor | live editor and bounded protocol tests | todo |
| S5 | Same exported legacy C ABI including TDM profiles and Q1.23 bridge | legacy/client-c | unchanged | symbol comparison, self-test, TDM harness all | todo |
| S6 | RTAS/TDM ppc+i386 bundles with 10.5 dependencies | legacy/native | unchanged | static binary/SDK audit | todo |
| S7 | Pro Tools 8 API/host-port DMA compatibility | legacy/native | unchanged | licensed SDK audit | todo |
| S8 | CPU-independent DSP56321 firmware unchanged | legacy/native/tdm | none | verify-tdm-engine.py; all three harness modes | todo |
| S9 | GitHub dispatch on local licensed runner, signed artifacts only | .github/workflows | none | passing run, artifact inspection | todo |
| S10 | Leopard package, configuration and catalog instructions | package script | none | archive inspection/signature/hash | todo |
| S11 | Copy alongside build 38 on USB, verify checksum | release artifact | none | cp -X, dot_clean, shasum | todo |
| S12 | Execution on PPC/Leopard and Pro Tools/HD hardware | released binaries | real host | unavailable on ARM64 build Mac | unverified: requires target Mac and HD Accel card |
| S13 | Existing AU/VST3/CLAP/LV2/64-bit AAX preserved, no JUCE or 32-bit AAX | existing modern tree | unchanged | diff boundary and workspace checks | todo |

No proprietary SDK text, headers or source may be committed or uploaded. Local SDK extracts and raw build logs stay in ignored build directories. This ledger stores conclusions only.
