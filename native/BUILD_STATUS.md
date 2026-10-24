# OpenGridder native development build — 24 October 2026

Project: `MistaMin/OpenGridder-Rust`. Rust engine and audio transport, with
SDK-only C++ AAX/VST3 adapters. No JUCE is compiled or linked by these targets.

The server now performs real AU/VST3 hosting; see [SERVER.md](SERVER.md) for
build/start instructions, architecture and remaining limits.

## Artifact

`../target/aax-universal/OpenGridder.aaxplugin`

The bundle contains arm64 and x86_64 slices. Intel Rust and the final Intel
SDK adapter build run under Rosetta. Build with:

```sh
export AAX_SDK_ROOT="/Volumes/Developer/Dev Projects/AAX Tools/aax-sdk-2-8-1"
bash scripts/build-aax-macos.sh
```

The original development bundle remains ad-hoc signed. A separate **PACE and
Apple Developer ID signed** bundle is now at
`../target/aax-pace-signed/OpenGridder.aaxplugin`, using the user-supplied
Signing Only wrap configuration `807F4D50-CFDB-11F1-A0CD-00505692AD3E`.
PACE identifies the product as OpenGridder; final Apple strict verification,
ABI smoke checks and real AU system smoke checks pass on arm64 and Intel.
The final Apple sealing pass is followed by PACE verification to verify both
signature systems. See `scripts/sign-aax-macos.sh` to reproduce signing.
The 1.2.1-preview.1 installer and disk image are now Apple notarized (Accepted),
stapled and Gatekeeper verified. The installer contains this PACE-signed AAX,
a Developer ID signed universal server, launcher, audited manual and dependency
notices. Extracted native/Intel system smoke checks pass and the AAX executable
hash matches the signing input. Actual Pro Tools loading remains unverified.
Neither bundle has been installed into the system plugin folder or published.
See [release preparation](../Docs/Release.md).

## Implemented

- Mono/stereo AAX Native effect registration and SDK parameter model.
- SDK-neutral Rust C interface and a Rust IO worker using the existing
  OpenGridder client/protocol.
- Preallocated single-producer/single-consumer ownership-transfer queues.
- Timestamped return audio with stale-result rejection and delayed dry fallback.
- Coherent atomic transport/plugin latency requests. Larger host blocks expand
  the budget; changes become active after host acknowledgement.
- Generic master bypass and a non-automatable remote-plugin selector.
- External SDK build inputs. Legacy RTAS/TDM wrappers are a separate private
  or externally supplied layer, rather than a dependency of the open core.

## Validation

- Current workspace validation: **33 tests pass on arm64; 32 pass under Rosetta**.
  The installed Water Channel fixture is arm64-only, so that one test is excluded
  from the Intel run. Installed OpenPreamp VST3 state/float64-transport tests run
  on both architectures. Real AU processing, callback heap checks and isolated
  crash recovery also run on both. See [VALIDATION.md](VALIDATION.md).
- The final universal AAX binary and standalone server pass an end-to-end test
  through a real isolated Apple AU on arm64 and x86_64, using a configured
  64-sample initial budget. This functional test does not certify realtime deadlines.
- Both Mach-O slices load independently. Native arm64 and Intel Python under
  Rosetta resolve the ACF entry points and verify the actual Rust callback's
  delayed impulse through the universal AAX binary.
- Earlier Avid descriptor validation passes, with a warning about no custom EffectGUI.
- Earlier Avid data-model instantiation passes.
- Earlier Avid parameter validation passes for mono and stereo. The validator warns
  about the selector's 129 discrete steps.
- Bundle plist, architecture checks, ad-hoc signature verification and source
  whitespace checks pass.

The 2024 Avid validator required a temporary copy at a path without spaces and
two nil guards in its disk-space metadata collection on this machine. These
changes do not modify plugin tests or suppress assertions; the original tools
were not modified. Descriptor validation also passed using the original tool.
Logs/results are under `../target/aax-validation/` and `../target/`.

## Still required for the requested release

- **Pro Tools 10 / OS X 10.8.5**: the Rust 1.41 native-client i386 archive and
  AAX adapter/required SDK i386 objects now compile. Final linkage is blocked by
  missing i386 Apple system/framework libraries; a real archived Apple SDK and
  matching C++ headers are required. The universal 64-bit bundle does not provide
  PT10 support. See [legacy progress](legacy/README.md).
- Actual Pro Tools host qualification, including appropriate OS/host pairs through
  macOS 26. Distribution notarization is complete for the preview candidate.
- Broader real-plugin hosting qualification, native editors, sidechains, instruments
  and preset/automation support. Actual AU/VST3 effect processing is now implemented;
  unknown formats/IDs return errors rather than substitute processors.
- Full custom Rust editor integration, remote state recall by stable plugin ID,
  arbitrary chains and parameter automation. The current selector saves an
  index through the AAX parameter model, not complete remote plugin state.
- Offline bounce, MIDI/transport synchronization, recovery transitions and
  sustained-load/latency qualification on real hardware and networks.
- Selectable minimum-phase FIR versus linear-phase FIR oversampling, with
  measured responses and delay compensation. No new filter mode is claimed.
- Native VST3/AU client adapters. RTAS/TDM stay deferred and separate.

This is an early development plugin, not the completed compatibility/stability
release requested by the user.
