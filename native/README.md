# Native format adapters

The OpenGridder engine, DSP, transport, callback buffers and GUI logic remain
Rust under the repository's MIT license. No JUCE is used by these targets.
`include/opengridder.h` is the SDK-neutral C boundary. The AAX adapter contains
only C++ SDK-interface glue; it links the Rust static library.

SDKs are supplied through external paths. They must not be copied or committed
into this repository. The build compiles the installed AAX library into the
build directory rather than modifying the SDK installation.

RTAS/TDM are deferred. Their adapters belong in a separate private repository
or optional externally supplied wrapper directory, with users supplying their
own licensed legacy SDKs. The open Rust core must not depend on those SDKs.
No RTAS/TDM target or support claim exists yet.

## AAX development build

On Apple Silicon with Rosetta installed:

```sh
export AAX_SDK_ROOT="/path/to/aax-sdk"
bash scripts/build-aax-macos.sh
```

This uses native arm64 Rust and Intel-hosted Rust/Clang under Rosetta, then
combines the slices into `target/aax-universal/OpenGridder.aaxplugin`.
The script defaults to Rust 1.98.0 toolchain directories; override
`OG_ARM_TOOLCHAIN` and `OG_INTEL_TOOLCHAIN` when needed. An Intel toolchain can
be installed with `rustup toolchain install 1.98.0-x86_64-apple-darwin
--profile minimal --force-non-host`. No SDK or signing credential is embedded.

Build `og-plugin` in release mode, then configure `native/aax` with:

```
-DAAX_SDK_ROOT=/path/to/aax-sdk
-DOPENGRIDDER_RUST_LIBRARY=/path/to/target/release/libog_plugin.a
```

The library and AAX build must have matching architectures. Intel builds can
use an Intel Rust toolchain under Rosetta; this produces 64-bit Intel code,
not the 32-bit code required by Pro Tools 10.

The initial adapter exposes mono/stereo FX, master bypass, and a non-automatable
remote-plugin selector through the host's generic parameter interface. Index
zero means None. Server defaults to `127.0.0.1:55056`; `OPENGRIDDER_SERVER` in
the host process environment overrides it. GUI-launched hosts can use
`~/Library/Application Support/OpenGridder/client.json` (see [SERVER.md](SERVER.md)). The selector loads one discovered
server plugin; it does not yet expose arbitrary chains or the custom editor.
Only selector/bypass parameter state is currently handled by AAX; complete
remote plugin-state/session recall and stable plugin-ID persistence remain
required before a release. Discovery-index selection is development-only.

The native callback defaults to a configurable 256-frame transport budget, preallocated queues
and timestamped return audio. Larger host blocks request a larger budget; the
transport/plugin latency pair becomes active together only after host
acknowledgement. The budget grows during an instance's lifetime; recreating the
instance resets it. Late results are discarded; underruns use delayed dry audio.
Network IO, allocations and remote chain changes run off the host callback.
This is a development implementation, not a latency guarantee. Freewheeling
offline bounce, MIDI and transport synchronization are not yet qualified.

Production AAX requires PACE signing, host qualification, a custom editor,
automation, MIDI/transport, full state recall and host qualification of the
latency-change notification handling. The server now performs actual AU/VST3
effect processing, with native parameters/state and per-client process isolation;
see [server instructions and limits](SERVER.md). Pro Tools 10 / OS X 10.8.5
requires a separate legacy Rust/dependency/toolchain and 32-bit AAX milestone.
