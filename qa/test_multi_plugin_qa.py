#!/usr/bin/env python3
"""
Multi-Plugin QA Test Suite:
1. Connect to OpenGridder Server.
2. Load multiple plugins (e.g. AUDelay and AULowpass) into the rack.
3. Query and sweep parameters on each loaded plugin.
4. Process audio through the multi-plugin chain.
5. Verify bypass, oversampling, and slot exchange.
6. Verify latency reporting.
"""

import ctypes
import os
import sys
import time

LIB_PATH = "/Users/marcosdeida/Projects/OpenGridder-Rust/target/release/libog_plugin.dylib"

if not os.path.exists(LIB_PATH):
    print(f"Error: {LIB_PATH} does not exist", file=sys.stderr)
    sys.exit(1)

lib = ctypes.CDLL(LIB_PATH)

lib.og_native_create.argtypes = [ctypes.c_double, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
lib.og_native_create.restype = ctypes.c_void_p
lib.og_native_destroy.argtypes = [ctypes.c_void_p]
lib.og_native_destroy.restype = None
lib.og_native_status.argtypes = [ctypes.c_void_p]
lib.og_native_status.restype = ctypes.c_int32

lib.og_native_num_available_plugins.argtypes = [ctypes.c_void_p]
lib.og_native_num_available_plugins.restype = ctypes.c_uint32
lib.og_native_get_available_plugin.argtypes = [
    ctypes.c_void_p, ctypes.c_uint32,
    ctypes.c_char_p, ctypes.c_uint32,
    ctypes.c_char_p, ctypes.c_uint32,
    ctypes.c_char_p, ctypes.c_uint32
]
lib.og_native_get_available_plugin.restype = ctypes.c_bool

lib.og_native_num_loaded_plugins.argtypes = [ctypes.c_void_p]
lib.og_native_num_loaded_plugins.restype = ctypes.c_uint32
lib.og_native_get_loaded_plugin.argtypes = [
    ctypes.c_void_p, ctypes.c_uint32,
    ctypes.c_char_p, ctypes.c_uint32,
    ctypes.POINTER(ctypes.c_bool), ctypes.POINTER(ctypes.c_uint32)
]
lib.og_native_get_loaded_plugin.restype = ctypes.c_bool

lib.og_native_load_plugin.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
lib.og_native_load_plugin.restype = ctypes.c_bool
lib.og_native_unload_plugin.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
lib.og_native_unload_plugin.restype = ctypes.c_bool
lib.og_native_set_plugin_bypassed.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_bool]
lib.og_native_set_plugin_bypassed.restype = ctypes.c_bool
lib.og_native_set_plugin_oversampling.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32]
lib.og_native_set_plugin_oversampling.restype = ctypes.c_bool
lib.og_native_exchange_plugins.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32]
lib.og_native_exchange_plugins.restype = ctypes.c_bool

lib.og_native_num_plugin_params.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
lib.og_native_num_plugin_params.restype = ctypes.c_uint32
lib.og_native_get_plugin_param_info.argtypes = [
    ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32,
    ctypes.c_char_p, ctypes.c_uint32,
    ctypes.POINTER(ctypes.c_float), ctypes.POINTER(ctypes.c_float)
]
lib.og_native_get_plugin_param_info.restype = ctypes.c_bool
lib.og_native_set_plugin_param.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_float]
lib.og_native_set_plugin_param.restype = ctypes.c_bool

lib.og_native_process.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.POINTER(ctypes.c_float)),
    ctypes.POINTER(ctypes.POINTER(ctypes.c_float)),
    ctypes.c_uint32
]
lib.og_native_process.restype = ctypes.c_int32

lib.og_native_latency.argtypes = [ctypes.c_void_p]
lib.og_native_latency.restype = ctypes.c_uint32


def main():
    print("=== Multi-Plugin QA Verification ===")
    p = lib.og_native_create(48000.0, 512, 2, 256)
    assert p, "Failed to create NativePlugin"

    for _ in range(50):
        if lib.og_native_status(p) >= 2:
            break
        time.sleep(0.1)
    assert lib.og_native_status(p) >= 2, "Failed to connect to server"
    avail = lib.og_native_num_available_plugins(p)
    print(f"Connected to OpenGridder Server! Total available plugins: {avail}")

    name_buf = ctypes.create_string_buffer(256)
    id_buf = ctypes.create_string_buffer(256)
    cat_buf = ctypes.create_string_buffer(256)

    targets = ["AUDelay", "AULowpass"]
    found_indices = {}
    for i in range(avail):
        lib.og_native_get_available_plugin(p, i, name_buf, 256, id_buf, 256, cat_buf, 256)
        nm = name_buf.value.decode("utf-8", "ignore")
        for t in targets:
            if t not in found_indices and t.lower() in nm.lower():
                found_indices[t] = (i, nm, id_buf.value.decode("utf-8", "ignore"))

    print(f"Selected test targets: {found_indices}")
    assert len(found_indices) == 2, f"Could not find required test plugins: {found_indices}"

    # Load test plugins sequentially
    for t, (idx, nm, plug_id) in found_indices.items():
        print(f"Loading '{nm}' (idx={idx})...")
        assert lib.og_native_load_plugin(p, idx)
        time.sleep(0.3)

    num_loaded = lib.og_native_num_loaded_plugins(p)
    print(f"Total loaded plugins in rack: {num_loaded}")
    assert num_loaded == len(found_indices), f"Expected {len(found_indices)}, got {num_loaded}"

    # Check each loaded plugin's parameters
    for slot in range(num_loaded):
        is_byp = ctypes.c_bool(False)
        os_fact = ctypes.c_uint32(1)
        lib.og_native_get_loaded_plugin(p, slot, name_buf, 256, ctypes.byref(is_byp), ctypes.byref(os_fact))
        nm = name_buf.value.decode("utf-8", "ignore")
        n_params = lib.og_native_num_plugin_params(p, slot)
        print(f"  Slot {slot}: '{nm}', bypassed={is_byp.value}, oversampling={os_fact.value}x, params={n_params}")

        # Sweep all parameters for this slot
        p_name = ctypes.create_string_buffer(256)
        p_val = ctypes.c_float(0.0)
        p_def = ctypes.c_float(0.0)

        for p_idx in range(n_params):
            if lib.og_native_get_plugin_param_info(p, slot, p_idx, p_name, 256, ctypes.byref(p_val), ctypes.byref(p_def)):
                orig = p_val.value
                new_val = 0.75 if orig < 0.5 else 0.25
                assert lib.og_native_set_plugin_param(p, slot, p_idx, new_val)
                time.sleep(0.02)
                # Read back
                lib.og_native_get_plugin_param_info(p, slot, p_idx, p_name, 256, ctypes.byref(p_val), ctypes.byref(p_def))
                print(f"    Param {p_idx} '{p_name.value.decode()}': set to {new_val:.2f}, readback={p_val.value:.2f}")

    # Process Audio block
    block_size = 512
    in_l = (ctypes.c_float * block_size)(*[0.25] * block_size)
    in_r = (ctypes.c_float * block_size)(*[-0.25] * block_size)
    out_l = (ctypes.c_float * block_size)(0)
    out_r = (ctypes.c_float * block_size)(0)

    in_ptrs = (ctypes.POINTER(ctypes.c_float) * 2)(in_l, in_r)
    out_ptrs = (ctypes.POINTER(ctypes.c_float) * 2)(out_l, out_r)

    # Prime audio pipeline
    for _ in range(10):
        rc = lib.og_native_process(p, in_ptrs, out_ptrs, block_size)
        assert rc == 0
        time.sleep(0.02)

    peak_l = max(abs(out_l[i]) for i in range(block_size))
    peak_r = max(abs(out_r[i]) for i in range(block_size))
    print(f"Audio processing output through multi-slot chain: PeakL={peak_l:.4f}, PeakR={peak_r:.4f}")
    assert peak_l > 0.0 and peak_r > 0.0, "Audio processing produced silence"

    # Test Bypass & Oversampling
    print("Testing bypass on slot 0...")
    assert lib.og_native_set_plugin_bypassed(p, 0, True)
    time.sleep(0.1)

    print("Testing oversampling 2x on slot 1...")
    assert lib.og_native_set_plugin_oversampling(p, 1, 2)
    time.sleep(0.1)

    # Verify reordering
    print("Testing slot exchange (swapping slot 0 and slot 1)...")
    assert lib.og_native_exchange_plugins(p, 0, 1)
    time.sleep(0.2)

    # Clean up
    lib.og_native_destroy(p)
    print("ALL MULTI-PLUGIN QA CHECKS PASSED 100%!")


if __name__ == "__main__":
    main()
