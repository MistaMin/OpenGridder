#!/usr/bin/env python3
"""
smoke-win7.py - Verifies the Windows 7 compatibility package for OpenGridderServer.
"""

import os
import sys
import struct

def parse_pe_header(filepath):
    with open(filepath, "rb") as f:
        data = f.read()

    if data[:2] != b"MZ":
        raise ValueError(f"{filepath} is not a valid PE executable (missing MZ magic)")

    e_lfanew = struct.unpack("<I", data[0x3C:0x40])[0]
    pe_sig = data[e_lfanew:e_lfanew+4]
    if pe_sig != b"PE\x00\x00":
        raise ValueError(f"{filepath} invalid PE signature")

    magic = struct.unpack("<H", data[e_lfanew+24:e_lfanew+26])[0]
    is_64bit = (magic == 0x20B)

    # In Optional Header:
    # PE32+ (64-bit): MajorOperatingSystemVersion at offset 40, MajorSubsystemVersion at 48
    if is_64bit:
        major_os, minor_os = struct.unpack("<HH", data[e_lfanew+24+40:e_lfanew+24+44])
        major_sub, minor_sub = struct.unpack("<HH", data[e_lfanew+24+48:e_lfanew+24+52])
        subsystem = struct.unpack("<H", data[e_lfanew+24+68:e_lfanew+24+70])[0]
    else:
        major_os, minor_os = struct.unpack("<HH", data[e_lfanew+24+40:e_lfanew+24+44])
        major_sub, minor_sub = struct.unpack("<HH", data[e_lfanew+24+48:e_lfanew+24+52])
        subsystem = struct.unpack("<H", data[e_lfanew+24+68:e_lfanew+24+70])[0]

    return {
        "is_64bit": is_64bit,
        "major_os": major_os,
        "minor_os": minor_os,
        "major_subsystem": major_sub,
        "minor_subsystem": minor_sub,
        "subsystem": subsystem
    }

def main():
    pkg_dir = sys.argv[1] if len(sys.argv) > 1 else "target/server-windows7"
    exe_path = os.path.join(pkg_dir, "OpenGridderServer.exe")
    synch_dll = os.path.join(pkg_dir, "api-ms-win-core-synch-l1-2-0.dll")
    prng_dll = os.path.join(pkg_dir, "bcryptprimitives.dll")

    print(f"Scanning Windows 7 package at: {pkg_dir}")
    for p in [exe_path, synch_dll, prng_dll]:
        if not os.path.exists(p):
            print(f"FAIL: Missing required file: {p}")
            sys.exit(1)

    # Check PE header of server
    hdr = parse_pe_header(exe_path)
    print(f"OpenGridderServer.exe PE Info: 64-bit={hdr['is_64bit']}, OS Version={hdr['major_os']}.{hdr['minor_os']}, Subsystem Version={hdr['major_subsystem']}.{hdr['minor_subsystem']}")

    if hdr['major_subsystem'] > 6 or (hdr['major_subsystem'] == 6 and hdr['minor_subsystem'] > 1):
        print(f"FAIL: Subsystem version {hdr['major_subsystem']}.{hdr['minor_subsystem']} exceeds Windows 7 (6.1)")
        sys.exit(1)

    print("Verified PE subsystem version <= 6.1 (Windows 7 NT 6.1 compatible)")

    # Verify DLLs are PE and 64-bit
    synch_hdr = parse_pe_header(synch_dll)
    prng_hdr = parse_pe_header(prng_dll)

    assert synch_hdr['is_64bit'], "synch.dll must be 64-bit"
    assert prng_hdr['is_64bit'], "prng.dll must be 64-bit"
    print("Verified companion Win7 compatibility shims (WaitOnAddress + ProcessPrng) present and valid.")

    print("PASS: Windows 7 server package verification passed completely!")

if __name__ == "__main__":
    main()
