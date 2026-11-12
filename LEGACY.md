# Legacy RTAS/TDM is separate and not open source

The OpenGridder RTAS beta and TDM alpha adapters are not built from this
repository. They are in the private, closed-source repository:

    https://github.com/MistaMin/OpenGridder-Legacy-Wrappers

That repository contains the Avid RTAS/TDM SDK integration, the DSP56321
firmware, the portable C/Rust legacy clients, the ppc/i386 Leopard build and
the freeware binary packaging. It is separate because it is built against the
licensed Avid SDK and because **RTAS/TDM binaries are freeware, not open
source**.

## What this means

- No Avid RTAS or TDM SDK source, headers, documentation, tools, licence keys or
  standalone libraries are committed in this repository, and none are uploaded as
  build artifacts. The compiled RTAS/TDM bundles statically link plug-in-library
  object code from the separately licensed SDK; that does not redistribute the SDK.
- No RTAS/TDM adapter source is published here. The compiled `.dpm` bundles may
  be distributed free of charge with their notices; the source stays private.
- The MIT `crates/og-legacy` crate in this repository is only the TDM transport
  contract (Q1.23 PCM, timeslots, schedules). It does not link the Avid SDK and
  is not a `.dpm` plug-in.
- The licensed SDKs and the DSP assembler remain external build inputs on the
  private self-hosted build machine.
- The RTAS and TDM adapter source, the Leopard workflow, and the DSP firmware
  live in the private wrappers repository.

## Release identity

OpenGridder server, plug-in formats and the RTAS/TDM bundles are stamped with
the same product version and build number (`0.1.1-alpha`, build N) so a release
is identifiable across all formats. The main build supplies
`OG_VERSION`, `OG_BUILD_NUMBER`, `OG_GIT_COMMIT` and `OG_BUILD_DATE`; the
wrappers build consumes the same values.

## Licensing

The main OpenGridder project keeps its MIT licence (`LICENSE`). The RTAS/TDM
freeware binary licence, the Avid SDK boundary and the MIT components included
in the private tree are documented in the wrappers repository's `LICENSE` and
`THIRD_PARTY_LICENSES.md`. Avid, Pro Tools, RTAS, TDM and HD Accel are
trademarks of Avid Technology, Inc.; this project is not affiliated with or
endorsed by Avid.


