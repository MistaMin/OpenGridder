"""Load a development AAX Mach-O and exercise its actual Rust callback ABI.

This is not a substitute for AAX host validation or PACE signature verification.
Run with native Python and with an Intel Python under Rosetta to test both slices.
"""
import ctypes as c
import os
from pathlib import Path
import platform
import sys

bundle = Path(sys.argv[1]).resolve()
os.environ["OPENGRIDDER_SERVER"] = "127.0.0.1:0"  # no connection to a user's server
lib = c.CDLL(str(bundle / "Contents/MacOS/OpenGridder"))
for symbol in ("ACFRegisterPlugin", "ACFRegisterComponent", "ACFGetClassFactory",
               "ACFCanUnloadNow", "ACFStartup", "ACFShutdown"):
    getattr(lib, symbol)
lib.og_native_create.argtypes = (c.c_double, c.c_uint32, c.c_uint32, c.c_uint32)
lib.og_native_create.restype = c.c_void_p
lib.og_native_destroy.argtypes = (c.c_void_p,)
lib.og_native_process.argtypes = (c.c_void_p, c.POINTER(c.POINTER(c.c_float)),
                                c.POINTER(c.POINTER(c.c_float)), c.c_uint32)
lib.og_native_process.restype = c.c_int32
lib.og_native_latency.argtypes = (c.c_void_p,)
lib.og_native_latency.restype = c.c_uint32
handle = lib.og_native_create(48000.0, 64, 2, 256)
assert handle, "Rust runtime creation failed"
try:
    assert lib.og_native_latency(handle) == 256
    observed = []
    for block in range(12):
        source = (c.c_float * 64)()
        if block == 0:
            source[0] = 1.0
        left, right = (c.c_float * 64)(), (c.c_float * 64)()
        inputs = (c.POINTER(c.c_float) * 2)(source, source)
        outputs = (c.POINTER(c.c_float) * 2)(left, right)
        assert lib.og_native_process(handle, inputs, outputs, 64) == 0
        assert list(left) == list(right)
        observed.extend(left)
    assert observed[256] == 1.0
    assert sum(value != 0.0 for value in observed) == 1
finally:
    lib.og_native_destroy(handle)
print(f"{platform.machine()}: AAX entry points load; Rust callback impulse delay verified")
