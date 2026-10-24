# OpenGridder compatibility and reliability plan

Assessment date: 2026-10-24. Audited source commit:
`27195703326563d0db81dc3af76ff55af83abf28`.

## Project and scope

The production project is **MistaMin/OpenGridder-Rust**, in this directory.
The user explicitly requires Rust. The AudioGridder C++ fork is reference
material only; its local changes are not OpenGridder implementation work.
Do not replace this project with that fork or move the engine to C++.

Prioritize AAX Native support from Pro Tools 10 through current Pro Tools,
OS X 10.8.5 through macOS 26 on qualified host/OS pairs, stability, and low
measured latency. RTAS and TDM are deferred and must not gate AAX development.
Legacy SDKs are available per the user; a working TDM system is unconfirmed.
AU, VST3 and other client formats are secondary.

Selectable minimum-phase FIR and linear-phase FIR oversampling remains a
requested design direction. JUCE's minimum-phase IIR reference is not a
minimum-phase FIR implementation, and its measurements do not characterize
OpenGridder's existing Rust filters.

This document is a proposal and baseline audit, not compatibility certification.

## Current implementation update

The baseline audit below is historical. The working tree now contains the native
AAX development adapter, callback queues, real macOS AU/VST3 effect hosting,
block-batched transport, state acknowledgements and per-client process isolation.
See [native/SERVER.md](native/SERVER.md) and [native/VALIDATION.md](native/VALIDATION.md)
for current capabilities and validation. Full AAX session recall, editor/automation,
PACE signing and the legacy compatibility matrix remain outstanding.

## Verified baseline (before these changes)

Downloaded `MistaMin/OpenGridder-Rust` into the local Projects workspace.
`cargo test --workspace --locked` passed all 17 existing tests on this machine.
These tests do not validate loading in Pro Tools or external plugin hosting.

Observed implementation gaps:

- At baseline, `og-plugin/src/plugin_formats.rs` had no AAX, RTAS, or TDM wrapper.
  It has module entry/exit symbols, but no `GetPluginFactory` implementation;
  `lv2_descriptor` returns null. A cdylib alone is not a working DAW plugin.
- `og-hosting/src/vst3.rs::instantiate_vst3` constructs the internal
  `HostedPluginInstance`. That instance implements its own gain/mix processing;
  it does not instantiate the described third-party VST3 processor.
- `og-plugin/src/processor.rs::process_block_f32/f64` calls
  `Client::process_audio_f32/f64` directly. Those methods lock the audio socket
  and synchronously send and receive audio. The transport allocates response
  buffers per block. Network stalls and allocation are incompatible with the
  requested bounded callback behavior.
- Sandbox modes and handshake flags exist, but those declarations alone do
  not establish operating-system process isolation or crash recovery.

## Proposed delivery families

| Family | Architecture and format | Prerequisites / status |
| --- | --- | --- |
| Legacy Pro Tools 10 / OS X 10.8.5 | i686 Rust client with 32-bit AAX Native integration | Compatible legacy Rust/dependencies and OS SDK, 32-bit AAX SDK, actual host validation |
| Pro Tools 11 onward | x86_64 AAX Native; arm64 for supported modern hosts | Appropriate AAX SDK versions, signing, host/OS qualification matrix |
| RTAS / TDM | Deferred | Outside the primary AAX delivery path |
| Remote server | Modern independent process hosting actual external plugins | Real hosting adapters, process isolation, transport and recovery implementation |
| Secondary client formats | AU, VST3, others as scoped later | Actual format interfaces and host validation after Pro Tools path |

Use separate legacy and modern artifacts/toolchains with a versioned shared
protocol and stable state/parameter identities. Keep client/server logic, audio
transport and DSP in Rust. An SDK-facing AAX adapter may require minimal C++
FFI glue because AAX exposes C++ interfaces; this must not become a replacement
C++ client/engine. Establish the adapter boundaries against the actual SDK.

Pro Tools 10 requires a 32-bit AAX build. Modern Pro Tools needs 64-bit AAX,
including arm64 for supported Apple Silicon hosts. A modern Rust deployment
target setting alone does not establish OS X 10.8.5 compatibility. Current Rust
macOS baselines and lack of distributed i686 artifacts make legacy toolchain
and dependency feasibility an early required milestone. Investigate a pinned
historical Rust toolchain or a narrowly scoped custom runtime/build before
claiming support; neither path has yet been verified on OS X 10.8.5.

Do not pin the whole modern server to an obsolete toolchain merely to support
a legacy client. Isolate the legacy-compatible Rust client core and validate
its protocol interoperability with the modern server. The server need not run
on the oldest client OS unless the user requires it.

## Implementation milestones and acceptance evidence

1. Confirm legacy hardware, SDK availability, server OS, required hosted plugin
   formats, channel counts, sample rates, and tracking versus mixing needs.
   Use OpenGridder-Rust as the production baseline. Locate the AAX SDK and
   prove the 32-bit Rust/AAX build path early.
2. Prove a real external plugin can load, process, expose parameters, and restore
   state in a server child process. Crash that child deliberately and verify
   containment, restart, and deterministic client output.
3. Implement a preallocated bounded audio exchange with a dedicated network
   worker. The DAW callback must not wait for sockets, mutexes, filesystem,
   logging, UI, or heap allocation/deallocation. Specify maximum channels,
   frames, MIDI events, queue depth, and overload behavior. Sequence audio blocks
   so late results cannot silently play at the wrong time.
4. Prove a modern AAX Native vertical slice: audio, automation, state recall,
   sample-accurate latency reporting, bypass, and reconnect. In parallel in the
   engineering schedule, prove legacy client toolchain/SDK feasibility before
   committing to a shared client implementation. No agent delegation implied.
5. Build the Rust client with 32-bit AAX integration on the legacy toolchain.
   Test on Pro Tools 10 / OS X 10.8.5, including native/DSP routing transitions
   on HD where applicable. RTAS/TDM remain deferred.
6. Qualify OS/host pairs and release packages. Measure impulse round-trip delay,
   callback execution distribution and maxima, underruns, network jitter, CPU,
   and sustained load. Record hardware, network, sample rate, buffer size,
   channel/instance count and hosted plugin latency with every result.

Reliability tests must include malformed/truncated packets, disconnect and
reconnect, server and plugin crashes, queue over/underrun, variable and zero-size
blocks, sample-rate changes, offline bounce, rapid transport changes, session
save/reopen, MIDI note cleanup, and dynamic hosted-plugin latency changes.
Select dry bypass versus silence and recovery transitions explicitly for effects
and instruments; delayed dry paths must respect reported latency.

Latency is the combination of host buffering, transport scheduling/network,
server processing, hosted-plugin latency and any oversampling. No numerical
latency or stability guarantee has yet been established. Agree measurable
acceptance thresholds after the user's hardware/workload details are available.

## External compatibility references

- Avid Pro Tools 10.3.7 qualifications include OS X 10.8.5 and HD Accel:
  https://kb.avid.com/pkb/articles/en_US/Knowledge/Pro-Tools-10-3-7-Downloads
- Avid Pro Tools reference guide describes 32-bit AAX/RTAS/TDM versus 64-bit AAX:
  https://resources.avid.com/SupportFiles/attach/Pro_Tools/12.0/Pro_Tools_12_0_Reference_Guide.pdf
- Avid AAX SDK and development/signing prerequisites:
  https://developer.avid.com/aax/
- Rust macOS platform requirements currently start at Intel macOS 10.12:
  https://github.com/rust-lang/rust/blob/main/src/doc/rustc/src/platform-support/apple-darwin.md
- Avid's host/OS qualification matrix must be checked for each release:
  https://kb.avid.com/pkb/articles/en_US/Knowledge/Pro-Tools-Operating-System-Compatibility-Chart

## Open decisions

- Exact legacy Mac, Pro Tools version, HD card family, and local AAX SDK paths.
  SDK availability is confirmed; exact versions/build tools need inspection.
- Whether only the client or also the server must run on OS X 10.8.5.
- Server hardware/OS, network topology, hosted plugin formats and representative
  third-party plugins available for validation.
- Tracking latency budget, sample rates, buffer sizes, channel and instance load.
- Whether existing AudioGridder protocol/session interoperability is mandatory.

## Native development implementation

The native AAX adapter is now in `native/aax`, with an SDK-neutral C interface
to `og-plugin::native`. The Rust callback uses a preallocated ownership-transfer
queue and timestamped audio timeline; network IO and plugin selection run on a
worker. Late results are rejected and underruns use delayed dry audio. Reported
latency changes wait for host acknowledgement before becoming active.

Arm64 and Rosetta-built x86_64 Rust release artifacts have been compiled. See
`native/README.md` for build commands, current limitations and SDK boundaries.
This does not establish Pro Tools 10 or OS X 10.8.5 support. RTAS/TDM wrappers
remain separate private or externally supplied build layers. The earlier C++
PDC fix is reference work only and has not been applied to the Rust DSP.
