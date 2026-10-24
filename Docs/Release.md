# AAX preview release preparation

The current release scope is macOS AAX Native plus the required CLI server.
The server hosts AU/VST3; no AU/VST3 client, CLAP, LV2, standalone plugin app,
RTAS or TDM target is distributed. The CLI server is not a standalone plugin
application bundle. Legacy wrappers stay private and separate. Future legacy
products must say RTAS BETA / TDM BETA and untested until hardware qualification.
Pro Tools 10 still requires a separate 32-bit AAX client, independently of RTAS.

## Local procedure

Set AAX_SDK_ROOT and OPENGRIDDER_VST3_SDK to independently installed SDK folders.
Use the pinned native and Intel Rust toolchains described in scripts. Do not
commit SDKs, keys, passwords, per-machine credentials or compiled SDK archives.

1. Run workspace and installed-plugin tests described in native/VALIDATION.md.
2. Run scripts/build-server-macos.sh and scripts/build-aax-macos.sh.
3. Run scripts/sign-aax-macos.sh with a connected, authorized signing iLok.
4. Run scripts/smoke-aax.py and scripts/smoke-system.py for both architectures.
5. Compare Docs/UserManual.md with the actual AAX controls. No custom plugin
   editor or fabricated UI screenshot is supplied. Build the PDF with
   tools/build_user_manual.py using Python with ReportLab, render and inspect it.
6. Run scripts/package-aax-macos.sh. OG_RELEASE_PYTHON may select that Python.
   Cargo/rustc must be on PATH for license collection. This stages only compiled
   binaries, the launcher, documentation and notices. It installs nothing.
7. Run scripts/notarize-aax-release-macos.sh after authorizing submission to Apple.
   Both installer and DMG must return Accepted, staple successfully, and pass
   Gatekeeper checks. Rejected submissions are not release assets.
8. Inspect installer payload and signatures after extraction; compare AAX hashes
   and run the signed server system smoke tests. Verify final SHA256SUMS.

Outputs live under dist/1.2.1-preview.1. The scripts never publish a GitHub
release or install the package. Generated stage directories are not overwritten;
archive an existing stage before preparing a revised candidate. Any binary
change requires a new signing, verification and notarization pass.

## GitHub Actions

ci.yml runs SDK-free Rust tests on Linux, Apple Silicon and Intel, plus an i386
bridge archive check. Its AU-only server artifacts are development artifacts;
they are not substitutes for this signed AU/VST3 server or a legacy .dpm.

aax-preview.yml is manual and uses a trusted private-repository macOS Apple
Silicon self-hosted runner labeled opengridder-aax-release. External SDKs,
native/Intel Rust toolchains, Rosetta, CMake/Ninja, ReportLab, signing identities,
the signing iLok and the AC_NOTARY keychain profile must already be available.
Set repository variables OPENGRIDDER_AAX_SDK_ROOT and OPENGRIDDER_VST3_SDK to
their paths, and OPENGRIDDER_AAX_RELEASE_READY=true only after configuring the
runner. Without that flag the workflow reports missing configuration and skips
the release job. No SDK is fetched by a public runner or uploaded as an artifact.
The workflow builds and signs only; submission to Apple is a separate explicit
manual input, default false. It creates private Actions artifacts, not a public
release. Do not enable untrusted pull-request execution on this signing runner.

## Qualification pending

PACE signatures, notarization, binary smoke tests and Rust tests do not establish
actual Pro Tools loading, complete session recall, latency under DAW load,
offline bounce or OS compatibility. Track those separately in native/BUILD_STATUS.md.
The build floors of macOS 11 arm64 and 10.12 Intel are not certified support.
Do not advertise PT10, OS X10.8.5 or macOS26 compatibility for this candidate.
