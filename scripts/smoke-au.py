#!/usr/bin/env python3
import ctypes
import os
import sys
from pathlib import Path

def main():
    if len(sys.argv) < 2:
        print("Usage: smoke-au.py <path/to/OpenGridder.component>", file=sys.stderr)
        sys.exit(1)

    bundle = Path(sys.argv[1]).resolve()
    assert bundle.exists(), f"Bundle not found: {bundle}"

    # 1. Verify bundle layout
    plist = bundle / "Contents" / "Info.plist"
    rsrc = bundle / "Contents" / "Resources" / "OpenGridder.rsrc"
    binary = bundle / "Contents" / "MacOS" / "OpenGridder"

    assert plist.is_file(), "Info.plist missing"
    assert rsrc.is_file(), "OpenGridder.rsrc missing (required for Logic 8)"
    assert binary.is_file(), "OpenGridder binary missing"

    plist_text = plist.read_text()
    assert "AudioComponents" in plist_text, "AudioComponents missing from Info.plist"
    assert "OpenGridderAUFactory" in plist_text, "OpenGridderAUFactory missing from Info.plist"
    assert rsrc.stat().st_size > 0, "Resource file is empty"

    # 2. Check architecture slices
    lipo_out = os.popen(f"lipo -info '{binary}'").read()
    print(f"Lipo info: {lipo_out.strip()}")
    assert "i386" in lipo_out, "Missing i386 slice (required for Logic 8 / 32-bit Logic 9)"
    assert "x86_64" in lipo_out, "Missing x86_64 slice (required for 64-bit Logic 9)"

    # 3. Load dynamic library on current host architecture
    dylib = ctypes.CDLL(str(binary))
    assert hasattr(dylib, "OpenGridderAUFactory"), "OpenGridderAUFactory export missing"

    class AudioComponentDescription(ctypes.Structure):
        _fields_ = [
            ("componentType", ctypes.c_uint32),
            ("componentSubType", ctypes.c_uint32),
            ("componentManufacturer", ctypes.c_uint32),
            ("componentFlags", ctypes.c_uint32),
            ("componentFlagsMask", ctypes.c_uint32),
        ]

    class AudioComponentPlugInInterface(ctypes.Structure):
        _fields_ = [
            ("Open", ctypes.CFUNCTYPE(ctypes.c_int32, ctypes.c_void_p, ctypes.c_void_p)),
            ("Close", ctypes.CFUNCTYPE(ctypes.c_int32, ctypes.c_void_p)),
            ("Lookup", ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_int16)),
            ("reserved", ctypes.c_void_p),
        ]

    dylib.OpenGridderAUFactory.restype = ctypes.POINTER(AudioComponentPlugInInterface)
    dylib.OpenGridderAUFactory.argtypes = [ctypes.POINTER(AudioComponentDescription)]

    desc = AudioComponentDescription(
        int.from_bytes(b"aufx", "big"),
        int.from_bytes(b"OGFx", "big"),
        int.from_bytes(b"OpGr", "big"),
        0, 0
    )

    plugin_iface = dylib.OpenGridderAUFactory(ctypes.byref(desc))
    assert plugin_iface, "OpenGridderAUFactory returned NULL"

    # Test Open
    open_fn = plugin_iface.contents.Open
    lookup_fn = plugin_iface.contents.Lookup
    close_fn = plugin_iface.contents.Close

    res = open_fn(plugin_iface, None)
    assert res == 0, f"Open failed with error {res}"

    # Verify Lookups
    kAudioUnitInitializeSelect = 1
    kAudioUnitGetPropertySelect = 4
    kAudioUnitRenderSelect = 14

    init_ptr = lookup_fn(kAudioUnitInitializeSelect)
    getprop_ptr = lookup_fn(kAudioUnitGetPropertySelect)
    render_ptr = lookup_fn(kAudioUnitRenderSelect)

    assert init_ptr, "Failed to lookup AudioUnitInitialize"
    assert getprop_ptr, "Failed to lookup AudioUnitGetProperty"
    assert render_ptr, "Failed to lookup AudioUnitRender"

    # Test Initialize
    init_fn = ctypes.CFUNCTYPE(ctypes.c_int32, ctypes.c_void_p)(init_ptr)
    res = init_fn(plugin_iface)
    assert res == 0, f"Initialize failed with {res}"

    # Test GetProperty: Latency (property ID 12)
    kAudioUnitProperty_Latency = 12
    getprop_fn = ctypes.CFUNCTYPE(
        ctypes.c_int32, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32)
    )(getprop_ptr)

    latency_sec = ctypes.c_double(0.0)
    size = ctypes.c_uint32(ctypes.sizeof(latency_sec))
    res = getprop_fn(plugin_iface, kAudioUnitProperty_Latency, 0, 0, ctypes.byref(latency_sec), ctypes.byref(size))
    assert res == 0, f"GetProperty Latency failed with {res}"
    print(f"Verified AudioUnit initialized cleanly! Default profile latency: {latency_sec.value:.6f} seconds")

    # Test Close
    res = close_fn(plugin_iface)
    assert res == 0, f"Close failed with error {res}"

    print("PASS: Universal Logic 8/9 AudioUnit component smoke test passed completely!")

if __name__ == "__main__":
    main()
