# OpenGridder 1.2.1-preview.1 - macOS AAX evaluation candidate

Included: universal AAX Native mono/stereo effect, universal AU/VST3 hosting
server, launcher, manual and dependency notices. No other client formats or
legacy .dpm binaries are included.

The AAX uses the host's generic view with Master Bypass and Remote Plugin
(0-128, one catalog selection). The Rust callback uses preallocated queues and
aligned dry fallback; server chains are isolated in child processes. PACE
Signing Only is used; no OpenGridder activation codes are required.

This package is a preview, not a qualified production release. Pro Tools host
tests, complete remote session recall, native plugin editors, automation and
long-duration/network qualification remain open. Pro Tools 10 and OS X 10.8.5
are not supported by this 64-bit build; macOS 26 compatibility is unqualified.
Do not assume zero latency or guaranteed glitch-free tracking.

RTAS and TDM will be separate, explicitly untested BETA releases after runnable
legacy binaries can be built. Neither a 32-bit queue archive nor adapter source
is a .dpm plugin.
