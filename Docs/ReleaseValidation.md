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
