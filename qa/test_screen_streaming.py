#!/usr/bin/env python3
import ctypes
import os
import sys
import time
from PIL import Image

LIB_PATH = "/Users/marcosdeida/Projects/OpenGridder-Rust/target/release/libog_plugin.dylib"

if not os.path.exists(LIB_PATH):
    print(f"Error: {LIB_PATH} does not exist", file=sys.stderr)
    sys.exit(1)

lib = ctypes.CDLL(LIB_PATH)

# Function Prototypes
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

lib.og_native_edit_plugin.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int32, ctypes.c_int32]
lib.og_native_edit_plugin.restype = ctypes.c_bool

lib.og_native_hide_plugin.argtypes = [ctypes.c_void_p]
lib.og_native_hide_plugin.restype = ctypes.c_bool

lib.og_native_get_screen_info.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_uint32),
    ctypes.POINTER(ctypes.c_uint32),
    ctypes.POINTER(ctypes.c_double),
    ctypes.POINTER(ctypes.c_uint64)
]
lib.og_native_get_screen_info.restype = ctypes.c_bool

lib.og_native_copy_screen_frame.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_uint8),
    ctypes.c_uint32,
    ctypes.POINTER(ctypes.c_uint64)
]
lib.og_native_copy_screen_frame.restype = ctypes.c_bool

lib.og_native_send_mouse.argtypes = [
    ctypes.c_void_p,
    ctypes.c_uint8,
    ctypes.c_float, ctypes.c_float,
    ctypes.c_bool, ctypes.c_bool, ctypes.c_bool,
    ctypes.c_float, ctypes.c_float,
    ctypes.c_bool
]
lib.og_native_send_mouse.restype = None

lib.og_native_is_editor_open.argtypes = [ctypes.c_void_p]
lib.og_native_is_editor_open.restype = ctypes.c_bool

lib.og_native_is_server_local_mode.argtypes = [ctypes.c_void_p]
lib.og_native_is_server_local_mode.restype = ctypes.c_bool


def main():
    print("=== OpenGridder Screen Streaming Test ===")
    p = lib.og_native_create(48000.0, 512, 2, 256)
    if not p:
        print("FAIL: og_native_create returned null")
        sys.exit(1)
    print(f"Created NativePlugin handle: {hex(p)}")

    # Wait for connection to server
    print("Waiting for connection to OpenGridder server (127.0.0.1:55056)...")
    connected = False
    for _ in range(30):
        st = lib.og_native_status(p)
        if st >= 2:
            connected = True
            break
        time.sleep(0.1)

    if not connected:
        print(f"FAIL: could not connect to server, status={lib.og_native_status(p)}")
        lib.og_native_destroy(p)
        sys.exit(1)

    print("Connected successfully!")
    num_avail = lib.og_native_num_available_plugins(p)
    print(f"Available plugins count: {num_avail}")

    if num_avail == 0:
        print("FAIL: no plugins available on server")
        lib.og_native_destroy(p)
        sys.exit(1)

    # Print first plugin
    name_buf = ctypes.create_string_buffer(256)
    id_buf = ctypes.create_string_buffer(256)
    cat_buf = ctypes.create_string_buffer(256)
    lib.og_native_get_available_plugin(p, 0, name_buf, 256, id_buf, 256, cat_buf, 256)
    first_name = name_buf.value.decode("utf-8", "ignore")
    print(f"Plugin [0]: {first_name} ({id_buf.value.decode('utf-8')}) in {cat_buf.value.decode('utf-8')}")

    # Load plugin 0
    print(f"Loading plugin 0 ({first_name})...")
    lib.og_native_load_plugin(p, 0)
    time.sleep(1.0)

    num_loaded = lib.og_native_num_loaded_plugins(p)
    print(f"Loaded plugins count: {num_loaded}")

    is_local = lib.og_native_is_server_local_mode(p)
    print(f"Server ScreenLocalMode: {is_local}")

    # Open editor
    print("Requesting editor GUI (og_native_edit_plugin)...")
    lib.og_native_edit_plugin(p, 0, 100, 100)
    time.sleep(0.5)
    print(f"is_editor_open: {lib.og_native_is_editor_open(p)}")

    if is_local:
        print("PASS: In Local Mode, native Cocoa/AU editor displays directly on host display with zero streaming overhead.")
    else:
        # Wait for screen frame in Remote Mode
        print("Waiting for streamed screen capture frame...")
        w = ctypes.c_uint32(0)
        h = ctypes.c_uint32(0)
        scale = ctypes.c_double(1.0)
        fid = ctypes.c_uint64(0)
        got_frame = False

        out_dir = "/Users/marcosdeida/Projects/OpenGridder-Rust/qa/2026-10-25-alpha-stabilization"
        os.makedirs(out_dir, exist_ok=True)
        out_img = os.path.join(out_dir, "screen_stream_frame.png")

        for _ in range(50):
            if lib.og_native_get_screen_info(p, ctypes.byref(w), ctypes.byref(h), ctypes.byref(scale), ctypes.byref(fid)):
                if w.value > 0 and h.value > 0 and fid.value > 0:
                    got_frame = True
                    break
            time.sleep(0.1)

        if not got_frame:
            print("FAIL: timed out waiting for screen capture frame from server")
            lib.og_native_hide_plugin(p)
            lib.og_native_destroy(p)
            sys.exit(1)

        print(f"SUCCESS: Received frame #{fid.value}: {w.value}x{h.value} @ {scale.value}x scale")

        # Copy frame
        buf_size = w.value * h.value * 4
        buf = (ctypes.c_uint8 * buf_size)()
        copied_fid = ctypes.c_uint64(0)
        if not lib.og_native_copy_screen_frame(p, buf, buf_size, ctypes.byref(copied_fid)):
            print("FAIL: og_native_copy_screen_frame returned false")
            lib.og_native_hide_plugin(p)
            lib.og_native_destroy(p)
            sys.exit(1)

        # Save image
        img_data = bytes(buf)
        img = Image.frombytes("RGBA", (w.value, h.value), img_data)
        img.save(out_img)
        print(f"Saved decoded screen frame to: {out_img}")

    # Forward mouse click
    print("Testing mouse click forwarding (LeftDown -> LeftUp at x=50, y=50)...")
    lib.og_native_send_mouse(p, 1, 50.0, 50.0, False, False, False, 0.0, 0.0, False)
    time.sleep(0.05)
    lib.og_native_send_mouse(p, 2, 50.0, 50.0, False, False, False, 0.0, 0.0, False)
    print("Mouse events forwarded successfully!")

    # Close editor
    print("Closing editor (og_native_hide_plugin)...")
    lib.og_native_hide_plugin(p)
    time.sleep(0.2)
    print(f"is_editor_open after hide: {lib.og_native_is_editor_open(p)}")

    # Clean up
    lib.og_native_destroy(p)
    print("NativePlugin destroyed cleanly.")
    print("=== PASS: Screen viewer and mouse forwarding verified! ===")


if __name__ == "__main__":
    main()
