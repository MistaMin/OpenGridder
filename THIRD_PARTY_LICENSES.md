# Native adapter dependencies

The OpenGridder Rust core remains under this repository's MIT license.
Externally supplied SDKs retain their own licenses; they are not relicensed
by the OpenGridder source license or copied into this repository.

| Dependency | Use | License / source |
| --- | --- | --- |
| Avid AAX SDK | AAX interface adapter and SDK support library | Avid AAX SDK license; see the installed SDK's LICENSE.txt and https://developer.avid.com/aax/ |
| Legacy RTAS/TDM SDKs | Future private or external wrappers | Supplied independently by the wrapper builder; not included |

AAX and Pro Tools are trademarks of Avid Technology, Inc.

The preview packaging script runs `tools/collect_licenses.py` to include notices
for the resolved Rust client/server dependency graph, Rust runtime copyright
records, and external SDK license notices. SDK source and documentation are
not packaged. The generated `rust-dependencies.json` lists exact versions.


## Optional native VST3 host

The VST3 host compiles selected SDK helper sources directly from the externally
supplied Steinberg SDK (`OPENGRIDDER_VST3_SDK`). SDK sources are not vendored into
this repository. The SDK's own license applies to those linked helpers; consult
the LICENSE supplied with the SDK version before distributing binaries. The MIT
license on the OpenGridder core does not relicense the SDK. VST is a trademark of
Steinberg Media Technologies GmbH. Hosting an independently installed plugin does
not incorporate that plugin's source into OpenGridder.
