# Third-party licences and SDK boundaries

OpenGridder's own source in this repository is MIT-licensed unless a file says
otherwise. Externally supplied SDKs keep their own licences and are never
relicensed or copied into this repository.

| Dependency | Use | Licence / source |
| --- | --- | --- |
| Avid AAX SDK | AAX interface adapter and SDK support library | Avid AAX SDK licence; supplied externally. See the installed SDK's licence and https://developer.avid.com/aax/ |
| Avid RTAS/TDM SDK | Private RTAS/TDM adapters only | Avid RTAS/TDM SDK licence; supplied externally by the licensed builder. Not in this repository and not uploaded. |
| OpenPluginSDK | Shared plug-in ABI used by the formats | Its own licence; local dependency path |
| Steinberg VST3 SDK | VST3 interface and host helpers | Steinberg VST3 licence; supplied externally, source not vendored |
| Apple SDKs and frameworks | macOS builds and system frameworks | Apple SDK licence; used as compiler/linker inputs |
| Rust crates | Server, plug-in, protocol and tooling | Licences in `Cargo.lock` and the crates' own metadata |

## RTAS and TDM are freeware, not open source

The RTAS/TDM adapter source, the Avid-SDK integration and the DSP firmware
integration live in the private repository
`MistaMin/OpenGridder-Legacy-Wrappers`. The compiled RTAS/TDM bundles may be
distributed free of charge as freeware with their notices, but their source is
not published and is not covered by this repository's MIT licence. No Avid SDK
code is included there or in the binaries. See that repository's `LICENSE` and
`THIRD_PARTY_LICENSES.md`, and `LEGACY.md` here.

AAX, Pro Tools, Avid, RTAS, TDM, HD Accel, Control|24 and C|24 are trademarks
of Avid Technology, Inc. VST is a trademark of Steinberg Media Technologies
GmbH. Audio Units and Apple are trademarks of Apple Inc. All other names belong
to their owners.

This project is not affiliated with, sponsored by or endorsed by Avid.
