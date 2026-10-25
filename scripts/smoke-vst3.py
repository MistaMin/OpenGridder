#!/usr/bin/env python3
import ctypes
import os
import sys
from pathlib import Path

def main():
    if len(sys.argv) < 2:
        print("Usage: smoke-vst3.py <path/to/OpenGridder.vst3>", file=sys.stderr)
        sys.exit(1)

    bundle = Path(sys.argv[1]).resolve()
    assert bundle.exists(), f"Bundle not found: {bundle}"

    # 1. Verify bundle layout
    plist = bundle / "Contents" / "Info.plist"
    binary = bundle / "Contents" / "MacOS" / "OpenGridder"

    assert plist.is_file(), "Info.plist missing"
    assert binary.is_file(), "OpenGridder binary missing"

    # 2. Check architecture slices
    lipo_out = os.popen(f"lipo -info '{binary}'").read()
    print(f"Lipo info: {lipo_out.strip()}")
    assert "arm64" in lipo_out, "Missing arm64 slice"
    assert "x86_64" in lipo_out, "Missing x86_64 slice"

    # 3. Load dynamic library on current host architecture
    dylib = ctypes.CDLL(str(binary))
    assert hasattr(dylib, "GetPluginFactory"), "GetPluginFactory export missing"
    assert hasattr(dylib, "bundleEntry"), "bundleEntry export missing"
    assert hasattr(dylib, "bundleExit"), "bundleExit export missing"

    # Call bundleEntry
    bundle_entry = dylib.bundleEntry
    bundle_entry.restype = ctypes.c_bool
    bundle_entry.argtypes = [ctypes.c_void_p]
    entry_res = bundle_entry(None)
    assert entry_res is True, "bundleEntry failed"

    # Call GetPluginFactory
    get_factory = dylib.GetPluginFactory
    get_factory.restype = ctypes.c_void_p
    get_factory.argtypes = []
    factory_ptr = get_factory()
    assert factory_ptr is not None, "GetPluginFactory returned NULL"

    # Call bundleExit
    bundle_exit = dylib.bundleExit
    bundle_exit.restype = ctypes.c_bool
    bundle_exit.argtypes = []
    assert bundle_exit() is True, "bundleExit failed"

    print("SUCCESS: VST3 smoke test passed!")

if __name__ == "__main__":
    main()
