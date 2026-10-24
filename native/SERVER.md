# OpenGridder native server development build

The Rust server now loads and processes real macOS AU v2 and VST3 audio effects.
There is no JUCE dependency in the server or client. The AU host uses Apple's C
API directly. A small C++ adapter bridges the VST3 SDK's C++ interfaces; Rust owns
plugin descriptors, buffers, transport, configuration and worker processes.

## Build

Supply your external Steinberg SDK, including `pluginterfaces/` and `public.sdk/`:

```sh
export OPENGRIDDER_VST3_SDK="/path/to/vst3sdk"
bash scripts/build-server-macos.sh
```

The result is `target/server-universal/OpenGridderServer`, containing arm64 and
x86_64. The Intel Rust build runs under Rosetta. SDK source files remain external;
none are copied into this repository. Building without the SDK produces an
AU-only server; `--version` reports which hosts were compiled.

This is an ad-hoc-signed command-line development executable, not a notarized
application/installer. The current build targets macOS 11+ on arm64 and 10.12+
on Intel. Those are build targets, not a completed OS qualification matrix.
OS X 10.8.5 and Pro Tools 10's 32-bit client still require a separate legacy build.

## Start a local system

For a small, reproducible setup using Apple's installed lowpass effect:

```sh
./target/server-universal/OpenGridderServer --au AudioUnit:aufx:lpas:appl
```

For one VST3 bundle:

```sh
./target/server-universal/OpenGridderServer --vst3-bundle "/path/to/Effect.vst3"
```

With no selection option the server scans enabled AU/VST3 locations. Each VST3
module is scanned in a separate process with a ten-second timeout; a failed scan
is omitted and logged. Only supported effect formats are advertised. All audio
classes in a bundle are enumerated using factory IDs, rather than path hashes.
Use `--list` to inspect the catalog without opening a listening socket.

The default listener is `127.0.0.1:55056` (plus any configured server ID). For a
remote studio server, use `--bind 0.0.0.0 --port 55056`. The transport has no
authentication/encryption and is intended for a trusted studio network.

Point the native AAX client to the listener using this file:

`~/Library/Application Support/OpenGridder/client.json`

```json
{
  "server": "127.0.0.1:55056",
  "transport_samples": 256
}
```

Use the remote machine's hostname/IP for a network server. `native/client.example.json`
is a template; this build does not overwrite user configuration. Recreate the
plugin instance after changing the file. `OPENGRIDDER_CLIENT_CONFIG` can override
the configuration path, and `OPENGRIDDER_SERVER` / `OPENGRIDDER_TRANSPORT_SAMPLES`
override those fields when launching a host from a shell.

The AAX generic parameter view provides **Remote Plugin**: 0 is none, 1 is the
first discovered effect, and so on, up to 128. The Rust client then loads that
actual remote effect and streams audio through it. A separate PACE/Apple signed AAX build is now available at
`target/aax-pace-signed/OpenGridder.aaxplugin`; the `aax-universal` build remains
ad-hoc signed. Actual Pro Tools host qualification is still pending.

## Latency and failure handling

- Transport budget defaults to 256 samples (5.33 ms at 48 kHz), plus the hosted
  effect's latency. A smaller configured budget, such as 64 samples, reduces the
  added delay if the host block size, network and server scheduling can sustain it.
- Larger host blocks automatically increase the requested budget. AAX latency
  changes activate only after host acknowledgement. A small budget is not a
  guarantee that every server result will arrive in time.
- The AAX callback uses preallocated queues and timestamped output. It never
  performs network IO or waits for the server. Late results are discarded;
  underruns use delayed dry audio.
- Audio packets are assembled per block rather than sending each sample in a
  separate TCP write. Incoming channel/frame/MIDI sizes are bounded.
- The standalone server defaults to **one child process per client chain**.
  Crashing one chain leaves the parent and other clients running. Clients can
  reconnect and restore saved state; automatic full AAX state recovery is still
  pending. `--no-isolation` is available for diagnostics only.
- Plugin-reported delay is converted from the oversampled processing rate to
  session-rate samples before adding the oversampler's delay.

## Verification and current limits

The automated tests exercise Apple's real AULowpass over the protocol, filter
parameter changes, variable block sizes, state recall into another session,
invalid-state acknowledgements, real AU output through the native callback with
zero callback heap operations, and killing a test-owned isolated worker while a
second client remains active. The installed Water Channel Reference VST3 test
checks all 44 real parameters, audible output changes, and state recall including
an edit saved before the next audio block consumes it.

`og-hosting/examples/host_probe.rs` is an explicit per-plugin diagnostic for
rendering and state recall. It does not certify every installed effect. Some
plugins need additional host interfaces, native UI/event-loop integration or
licensing services; failures remain visible rather than falling back to a fake
processor. Native AU/VST3 effects currently use float32 processing; float64 network
buffers are converted. Instruments/MIDI routing, sidechains, surround output,
factory preset browsing, full native editor capture/control, and dynamic bus
changes are not implemented.

The Rust client supports chain/state RPCs, but the **AAX parameter adapter still
saves its selector index and bypass only**. Complete DAW session recall by stable
plugin ID, native editor integration and hosted parameter automation remain
required before this is a production system. PACE signing, old Pro Tools/OS X
compatibility and sustained hardware/network qualification also remain open.
