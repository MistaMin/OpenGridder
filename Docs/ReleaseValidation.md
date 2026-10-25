# Preview candidate validation - 24 October 2026

Candidate: 1.2.1-preview.1. Binary implementation snapshot: 8bb942a.
Local distribution files: dist/1.2.1-preview.1 (excluded from source control).

The Apple Silicon and Intel AAX/server paths pass the real isolated Apple
AULowpass system check after extracting the final signed installer. The
extracted AAX executable matches the PACE signing input, SHA-256
300ae87873ce4f5133a2d4f8a5034eb0624b7a1a10d17a6b48c08f8912883353.
The final workspace run passes 31 tests with two installed-VST3 opt-in tests
excluded. Broader earlier fixture results are recorded in native/VALIDATION.md.

PACE verifies the extracted AAX as OpenGridder, Signing Only. Apple strict
bundle verification passes. The universal server is Developer ID signed with
the plugin-host library-validation exception, verified in both architecture
entitlement dictionaries. The installer is Developer ID Installer signed.

Apple accepted the installer submission d77528a4-2f13-4928-968f-6adc990cccb7
and disk-image submission 234df0d5-6e43-49a2-bfa6-7577b4812c6b. Both artifacts
have validated stapled tickets and pass Gatekeeper assessment. Final SHA256SUMS
are alongside the artifacts and checked after stapling.

The installer exposes selectable AAX and server components and mandatory notices.
Payload inspection finds no SDK headers/sources, RTAS/TDM .dpm or private keys.
The two-page manual was rendered and visually checked against actual controls.
No system installation or GitHub release publication has been performed.

This record does not establish Pro Tools host compatibility. PT10/32-bit AAX,
OS X10.8.5 and macOS26 qualification, full remote session recall/editor controls,
offline bounce and sustained hardware/network performance remain outstanding.

## Separate legacy artifacts — 25 October 2026

These artifacts are separate from the notarized modern installer above and have
not been installed or released publicly. Neither has run inside Pro Tools.

- Experimental i386 AAX: Actions run 40147792377 passed real compilation/linking,
  Apple/PACE signing with PT10 version-1 signature compatibility, and ZIP
  extraction/strict Apple signature verification. The downloaded archive's
  checksum and extracted signature were independently verified with normal
  macOS trust/keychain access. Deployment and SDK load commands are 10.8.
- OpenGridder RTAS BETA: Actions run 40148017808 passed the full external SDK
  build, i386 linking, Apple signing and ZIP extraction/strict verification.
  Private wrapper commit 73d90f0 is pinned in the workflow artifact. The build
  uses official Apple 10.6 headers for legacy QuickDraw and 10.8 link libraries.
  The wrapper includes a programmatic numeric remote-plugin selector editor.
- SDK source, docs, header copies and build toolchains are excluded from both
  artifact uploads. Wrapper sources remain in the separate local private repo.
- TDM is **not built**. The DSP assembler runs, but its actual card firmware and
  host/DMA transport implementation remain unfinished. No assembler probe is
  distributed as a plugin.

Host initialization/UI/audio/latency/stability and OS X 10.8.5 compatibility still
require real Pro Tools qualification. Successful builds/signatures establish
neither low-latency performance nor production readiness.
