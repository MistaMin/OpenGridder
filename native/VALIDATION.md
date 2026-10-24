# Native server/system validation — 24 October 2026

All results are local development checks on this machine. They do not establish
compatibility with every plugin, host, operating system or network.

| Check | Result |
| --- | --- |
| Rust workspace, arm64 | 33 passed; 0 failed; 0 ignored |
| Rust workspace, x86_64 under Rosetta | 32 passed; 0 failed; 1 filtered (arm64-only installed Water Channel fixture) |
| Real Apple AULowpass over client/server | Audio, parameters, variable blocks and state recall pass on both architectures |
| Real AU through native callback | Real filtered output; zero callback heap operations, both architectures |
| Installed Water Channel Reference VST3, arm64 | 44 native parameters; −12 dB audio change; queued-edit state recall passes |
| Installed OpenPreamp VST3, arm64 and Intel | Network state recall and float64 transport conversion pass |
| Installed Event Horizon VST3, arm64 | Explicit host probe: rendering and state recall pass |
| Native plugin worker crash | Kill one test-owned child; second client continues; parent accepts reconnect/state restore, both architectures |
| Final universal AAX + standalone server | Compiled native runtime → isolated real AU → callback output passes on arm64 and Intel |
| AAX entry points and delayed impulse | Final universal bundle passes native and Rosetta ABI smoke tests |
| Architectures/ad-hoc signatures | Server and AAX both contain arm64+x86_64 and pass strict local codesign verification |

The final arm64 test run measured isolated stereo AU / 48 kHz / 64-frame loopback
round trips at **0.106 ms median, 0.176 ms p95, 0.317 ms p99, 0.366 ms maximum**
over 200 blocks, after 20 warmup blocks. The Intel/Rosetta release run measured
0.090 ms median, 0.134 ms p95, 0.311 ms p99 and 0.445 ms maximum. These are
synchronous loopback transaction measurements, excluding AAX buffering, audio
hardware and external network delay. They are not realtime deadline guarantees.
The compiled-system smoke test deliberately paces callbacks for functional
verification rather than benchmarking a real DAW audio clock.

AIR Kill EQ instantiated after tolerating its unimplemented VST3 processing-state
notification, but supplied no usable component state in this environment. Its
state qualification failed; its stderr also reports a missing preset directory.
It is not included among the qualified effects. No substitute DSP is used.

## Reproduce

Set `OPENGRIDDER_VST3_SDK` to an external SDK root. Set
`OPENGRIDDER_TEST_VST3` to the installed Water Channel Reference bundle and
`OPENGRIDDER_TEST_COMPAT_VST3` to the installed OpenPreamp bundle.

```sh
cargo test --workspace --locked -- --include-ignored --nocapture
```

Use an Intel-hosted Rust toolchain under `arch -x86_64` for the Intel run, passing
`--target x86_64-apple-darwin --target-dir target/rosetta-intel`, and skip
`installed_vst3_network_audio_parameters_and_state` if the Water fixture is
arm64-only. The SDK build and test dependencies remain external.

After running both build scripts:

```sh
python3 scripts/smoke-aax.py target/aax-universal/OpenGridder.aaxplugin
arch -x86_64 /usr/bin/python3 scripts/smoke-aax.py target/aax-universal/OpenGridder.aaxplugin
python3 scripts/smoke-system.py target/server-universal/OpenGridderServer target/aax-universal/OpenGridder.aaxplugin
arch -x86_64 /usr/bin/python3 scripts/smoke-system.py target/server-universal/OpenGridderServer target/aax-universal/OpenGridder.aaxplugin
```

Saved evidence: `target/server-tests-native.log`, `target/server-tests-intel.log`,
`target/system-smoke-arm64.json`, `target/system-smoke-intel.json`,
`target/OpenPreamp-vst3-probe-native.json`, `target/eventhorizon-vst3-probe-native.json`,
`target/build-server-macos.log` and `target/build-aax-macos.log`.
Earlier Avid descriptor/model/parameter results are under `target/aax-validation/`;
those checks were not rerun for this transport/server iteration. The final AAX
binary received ABI/system smoke checks. Production Pro Tools loading remains unverified.

## PACE signing follow-up

Wrap configuration: `807F4D50-CFDB-11F1-A0CD-00505692AD3E` (Signing Only).
PACE verifies product OpenGridder. The signed artifact is
`target/aax-pace-signed/OpenGridder.aaxplugin`. Apple Developer ID hardened-runtime
sealing and strict verification succeed after the PACE pass; PACE verification
also succeeds after that final Apple pass. Both ABI and real AU system smoke
tests pass on arm64 and Intel. Signing evidence is in `target/aax-pace-signed/`.
No notarization, Pro Tools-host validation, system installation or publication
is implied by these signing checks.
