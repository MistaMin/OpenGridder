# Roadmap

What is planned beyond 1.0 BETA. Order is not a promise; items marked "blocked" say what is missing.

## Older Mac hardware: PowerPC (RTAS/TDM)

Client only (the server never runs on PowerPC). The editor-streaming view and picture decoder are
already endian neutral; see the PowerPC section of the legacy wrappers README.

- Blocked: no PowerPC compiler on the build Mac (Apple clang and LLVM cannot write PowerPC Mach-O;
  Xcode 3.2.6's gcc is not installed) and no Rust target for it.
- Work: C port of `native/legacy/client-core` (wire protocol, then the audio timeline), build with gcc 4.2
  `-arch ppc` against the 10.5 SDK, test on a real PowerPC Mac (nothing PowerPC can run here).

## Older Windows: XP and Vista

For RTAS/TDM on 32-bit Windows Pro Tools systems (the licensed SDK ships a Windows build folder).

- Blocked: current Rust cannot target XP; Vista needs the same check. Needs an older toolchain or a C client
  core (the same C port as PowerPC would serve here), 32-bit builds, and a test machine for each.
- Work: 32-bit Windows client core, RTAS/TDM `.dpm` for Windows from the SDK's Windows build, a Windows
  version of the editor view (the Mac one uses QuickDraw/Quartz), installer.

## Windows 7 and later: today

Only the server is built for Windows (x86_64, Windows 7 SP1 and later, with two compatibility DLLs). It is
checked by reading the executable's headers; it has not been run on a Windows machine yet. Hosting plugin
windows and the remote editor stream are macOS-only so far. Plugins for DAWs on Windows are not built.

- Work: run and test the server on Windows 7 and 10/11; load VST3 plugins on Windows; capture and stream
  their editors; build the VST3/CLAP/AAX plugins for Windows.
