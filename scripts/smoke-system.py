"""Exercise the built AAX Rust runtime against a real, isolated AU server.

Usage: python3 scripts/smoke-system.py SERVER AAX_BUNDLE
Repeat with Intel Python under Rosetta. This exercises the compiled system, not
Pro Tools itself, and does not replace PACE signing or DAW host qualification.
"""
import ctypes as c
import json
import math
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tempfile
import time

server_path, bundle = [Path(p).resolve() for p in sys.argv[1:3]]
server = subprocess.Popen([str(server_path), '--au', 'AudioUnit:aufx:lpas:appl', '--port', '0'],
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
handle = None
try:
    # Startup is bounded independently of any socket/plugin timeouts.
    import select
    ready, _, _ = select.select([server.stdout], [], [], 10)
    assert ready, 'Server startup timed out'
    line = server.stdout.readline()
    match = re.search(r'listening on 127\.0\.0\.1:(\d+)', line)
    assert match, line
    assert 'chain isolation=true' in line, line
    port = int(match[1])
    os.environ['OPENGRIDDER_SERVER'] = f'127.0.0.1:{port}'
    with tempfile.TemporaryDirectory(prefix='opengridder-smoke-') as folder:
        config = Path(folder) / 'client.json'
        config.write_text(json.dumps({'server': f'127.0.0.1:{port}', 'transport_samples': 64}))
        os.environ['OPENGRIDDER_CLIENT_CONFIG'] = str(config)
        os.environ.pop('OPENGRIDDER_TRANSPORT_SAMPLES', None)
        lib = c.CDLL(str(bundle / 'Contents/MacOS/OpenGridder'))
        lib.og_native_default_latency.restype = c.c_uint32
        assert lib.og_native_default_latency() == 64
        lib.og_native_create.argtypes = [c.c_double, c.c_uint32, c.c_uint32, c.c_uint32]
        lib.og_native_create.restype = c.c_void_p
        lib.og_native_destroy.argtypes = [c.c_void_p]
        lib.og_native_select.argtypes = [c.c_void_p, c.c_uint32]
        lib.og_native_status.argtypes = [c.c_void_p]
        lib.og_native_status.restype = c.c_int32
        lib.og_native_process.argtypes = [c.c_void_p, c.POINTER(c.POINTER(c.c_float)), c.POINTER(c.POINTER(c.c_float)), c.c_uint32]
        lib.og_native_process.restype = c.c_int32
        handle = lib.og_native_create(48000., 256, 2, lib.og_native_default_latency())
        assert handle, 'Could not create the compiled native client'
        lib.og_native_select(handle, 1)
        deadline = time.monotonic() + 10
        while lib.og_native_status(handle) != 3 and time.monotonic() < deadline:
            time.sleep(.002)
        assert lib.og_native_status(handle) == 3, 'Real AU failed to load through native client'
        source, left, right = [(c.c_float * 64)() for _ in range(3)]
        inputs = (c.POINTER(c.c_float) * 2)(source, source)
        outputs = (c.POINTER(c.c_float) * 2)(left, right)
        energy = 0.
        for block in range(200):
            for sample in range(64):
                source[sample] = .25 * math.sin(2 * math.pi * 16000 * (block * 64 + sample) / 48000)
            assert lib.og_native_process(handle, inputs, outputs, 64) == 0
            assert list(left) == list(right)
            assert all(math.isfinite(v) for v in left)
            if block >= 100:
                energy += sum(float(v) ** 2 for v in left)
            # This is a functional test, not a real-time scheduling benchmark.
            time.sleep(.003)
        rms = math.sqrt(energy / 6400)
        assert .00001 < rms < .1, f'Expected real lowpass output, not delayed dry: RMS={rms}'
        lib.og_native_destroy(handle)
        handle = None
        print(json.dumps({'architecture': platform.machine(), 'server': str(server_path),
                          'aax_bundle': str(bundle), 'effect': 'Apple AULowpass',
                          'transport_samples': 64, 'output_rms': rms,
                          'isolated_chain': True, 'result': 'passed'}))
finally:
    if handle is not None:
        lib.og_native_destroy(handle)
    server.terminate()
    try:
        server.wait(timeout=5)
    except subprocess.TimeoutExpired:
        server.kill()
        server.wait()
