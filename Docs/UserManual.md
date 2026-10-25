# OpenGridder 0.1.0-alpha - Multi-Format Release

## Release scope

This release contains the zero-JUCE OpenGridder multi-format audio plugins and standalone servers.
Supported plugin formats include:
- AudioUnit v2 (AU): Universal binary (i386 + x86_64 + arm64) supporting Logic Pro 8 (via Carbon Component Manager 'thng' resource), Logic Pro 9, and Logic Pro X / modern macOS DAWs.
- VST 2.4: Universal binary (i386 + x86_64 + arm64) supporting vintage Cubase (4/5/6/7), Nuendo, and WaveLab.
- AAX Native: Universal binary (arm64 + x86_64) signed with PACE Eden and Apple Developer ID for Pro Tools 11+ and Pro Tools Ultimate.
- Server: Standalone headless and tray servers for macOS (Universal arm64/x86_64) and Windows 7 SP1+ (x86_64 with Win32 compatibility shims).

Rust implements the DSP engine, bounded lockless transport, and protocol v13 networking. Thin SDK adapters provide AU, VST2, and AAX interfaces with zero JUCE dependencies.

Pro Tools 10 / RTAS / TDM legacy bridges are maintained on legacy branches and supported via the identical wire protocol.

## Selectable Latency Profiles

OpenGridder features four user-selectable latency profiles with dynamic Plugin Delay Compensation (PDC):
- Profile 0 (1024 samples): Default Standard LAN / Wi-Fi safety profile.
- Profile 1 (512 samples): High-speed 10GbE and Thunderbolt networking profile.
- Profile 2 (384 samples): Localhost VSTi profile optimized for virtual instrument live performance.
- Profile 3 (256 samples): Direct ultra-low latency localhost tracking profile.

Hosts automatically receive delay compensation notifications via standard audioMasterIOChanged (VST2), kAudioUnitProperty_Latency (AU), or SetCycleCount (AAX).

## Install and start

Quit your DAW before installing. The macOS installer places:
- OpenGridder.component into /Library/Audio/Plug-Ins/Components
- OpenGridder.vst into /Library/Audio/Plug-Ins/VST
- OpenGridder.aaxplugin into /Library/Application Support/Avid/Audio/Plug-Ins
- The server in /Applications/OpenGridder Server
- Documentation and licenses in /Library/Application Support/OpenGridder

For Windows 7, extract target/server-windows7/ to any folder and ensure the companion shims (api-ms-win-core-synch-l1-2-0.dll and bcryptprimitives.dll) remain in the same directory as AudioGridderServer.exe.

Open /Applications/OpenGridder Server and double-click
Start OpenGridder Server.command. Keep Terminal open while using the plugin;
Control-C stops the server. The first scan can take several minutes. Native
plugin editors are not exposed in this AAX preview. No custom UI screenshots
are included because the plugin uses Pro Tools' generic parameter view.

For a short scan and functional check, run the server with these arguments:
./OpenGridderServer --au AudioUnit:aufx:lpas:appl
This limits the catalog to Apple's lowpass effect. Plugins must be installed
and licensed separately on the server machine.

## Connect Pro Tools

By default the server binds to 127.0.0.1:55056. Insert OpenGridder as a mono or
stereo AAX Native effect and use the generic parameter view. Server connection
settings are read when an instance is created from:
~/Library/Application Support/OpenGridder/client.json

Create that file before inserting the plugin, using this JSON:
{"server":"127.0.0.1:55056","transport_samples":256}

For a second studio computer, start the server with --bind 0.0.0.0 --port 55056
and replace the JSON address with that computer's LAN address. The protocol has
no authentication or encryption; use a trusted studio network. Never expose its
ports to the public Internet. Restart the plugin instance after changing JSON.

## Controls

Master Bypass defaults to Active. Bypass passes delayed dry audio and preserves
the currently acknowledged compensation delay.

Remote Plugin defaults to 0 (no effect). Values 1 through 128 select discovered
server effects by catalog index. The displayed value includes index and name
after discovery. Only one selected effect is accessible through this AAX
adapter. This selector is deliberately not automatable because loading an effect
can change latency. If an effect is missing, use 0 or another available index.

Selection index and bypass can be saved by Pro Tools. Complete remote effect
settings and stable plugin IDs are not saved in the AAX session in this preview.
Catalog order can change after a scan or installation. Check the selected effect
each time you reopen a session. Generic remote parameter editing, automation,
native plugin windows and reliable complete session recall are not available.

## Latency and stability

transport_samples accepts integers from 32 to 8192 and defaults to 256. At 48 kHz,
256 samples are about 5.33 ms before hardware buffering and any effect latency.
A smaller setting reduces transport allowance but increases the chance of a
missed response. This is not zero-latency processing. The effective allowance
may increase for larger host blocks or effect delay. AAX reports changes to the
host and waits for acknowledgement before switching timelines.

The audio callback uses preallocated queues; networking and plugin loading run
on a worker. Missed or stale wet blocks fall back to aligned dry audio. Each
client chain runs in a child server process by default to contain effect crashes.
A crash may still interrupt audio and require reloading the effect or instance;
automatic full-session restoration is not implemented.

Minimum-phase versus linear-phase oversampling selection is not exposed in this
AAX preview. Oversampling controls in the core must not be interpreted as an
available host control. Instruments, MIDI, sidechains, surround, offline bounce,
transport synchronization and sustained live-tracking performance are unqualified.

## Troubleshooting and removal

If no effects appear, verify that Terminal remains open, the configured address
matches the listening server, and the effect supports mono or stereo audio.
If audio falls back to dry, increase transport_samples and recreate the instance.
Keep a copy of the server Terminal output when reporting a problem. Include
Mac architecture, OS, Pro Tools version, sample rate, block size and effect name.

To remove this preview, quit Pro Tools and the server, then remove the installed
AAX bundle, /Applications/OpenGridder Server and the OpenGridder documentation
folder. Your per-user client.json is separate and may be kept for a later build.

## Later legacy betas

Future products are named OpenGridder RTAS BETA and OpenGridder TDM BETA and will
be distributed separately as untested betas until qualified on legacy hardware.
No runnable legacy .dpm is supplied or implied here. The open Rust core and
private legacy SDK wrappers remain separate; SDK source and documentation are
not included in this package.

## Notices

The OpenGridder Rust core is MIT licensed. See the included Licenses directory
for dependency notices and external SDK licensing information. AAX and Pro Tools
are trademarks of Avid Technology, Inc. VST is a trademark of Steinberg Media
Technologies GmbH. Third-party effects are not included.
