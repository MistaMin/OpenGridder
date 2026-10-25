#!/usr/bin/env python3
import ctypes
import os
import plistlib
import sys
from pathlib import Path

def main():
    if len(sys.argv) < 2:
        print("Usage: smoke-auv3.py <path/to/OpenGridderAUv3.appex>", file=sys.stderr)
        sys.exit(1)

    bundle = Path(sys.argv[1]).resolve()
    assert bundle.exists(), f"Bundle not found: {bundle}"

    # 1. Verify bundle layout
    plist_path = bundle / "Contents" / "Info.plist"
    binary = bundle / "Contents" / "MacOS" / "OpenGridderAUv3"

    assert plist_path.is_file(), "Info.plist missing"
    assert binary.is_file(), "OpenGridderAUv3 binary missing"

    # 2. Check architecture slices
    lipo_out = os.popen(f"lipo -info '{binary}'").read()
    print(f"Lipo info: {lipo_out.strip()}")
    assert "arm64" in lipo_out, "Missing arm64 slice"
    assert "x86_64" in lipo_out, "Missing x86_64 slice"

    # 3. Verify Info.plist AUv3 attributes
    with open(plist_path, "rb") as f:
        pl = plistlib.load(f)

    ns_ext = pl.get("NSExtension", {})
    assert ns_ext.get("NSExtensionPointIdentifier") == "com.apple.AudioUnit-UI", "Invalid extension point"
    attrs = ns_ext.get("NSExtensionAttributes", {})
    components = attrs.get("AudioComponents", [])
    assert len(components) > 0, "No AudioComponents declared in Info.plist"
    comp = components[0]
    assert comp.get("type") == "aufx", "Component type must be aufx"
    assert comp.get("subtype") == "ogrd", "Component subtype must be ogrd"
    assert comp.get("manufacturer") == "OGRD", "Component manufacturer must be OGRD"

    # 4. Load dynamic library with ctypes on host arch
    dylib = ctypes.CDLL(str(binary))
    assert dylib is not None, "Failed to load OpenGridderAUv3 binary"

    print("SUCCESS: AUv3 smoke test passed!")

if __name__ == "__main__":
    main()
