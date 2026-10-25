#!/usr/bin/env python3
import ctypes
import os
import sys
from pathlib import Path

def main():
    if len(sys.argv) < 2:
        print("Usage: smoke-vst2.py <path/to/OpenGridder.vst>", file=sys.stderr)
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
    assert "i386" in lipo_out, "Missing i386 slice (required for vintage 32-bit Cubase)"
    assert "x86_64" in lipo_out, "Missing x86_64 slice (required for 64-bit Cubase)"

    # 3. Load dynamic library on current host architecture
    dylib = ctypes.CDLL(str(binary))
    assert hasattr(dylib, "VSTPluginMain"), "VSTPluginMain export missing"

    class AEffect(ctypes.Structure):
        pass

    audioMasterCallback = ctypes.CFUNCTYPE(
        ctypes.c_int64, ctypes.POINTER(AEffect), ctypes.c_int32, ctypes.c_int32, ctypes.c_int64, ctypes.c_void_p, ctypes.c_float
    )
    AEffectDispatcherProc = ctypes.CFUNCTYPE(
        ctypes.c_int64, ctypes.POINTER(AEffect), ctypes.c_int32, ctypes.c_int32, ctypes.c_int64, ctypes.c_void_p, ctypes.c_float
    )
    AEffectProcessProc = ctypes.CFUNCTYPE(
        None, ctypes.POINTER(AEffect), ctypes.POINTER(ctypes.POINTER(ctypes.c_float)), ctypes.POINTER(ctypes.POINTER(ctypes.c_float)), ctypes.c_int32
    )
    AEffectSetParameterProc = ctypes.CFUNCTYPE(
        None, ctypes.POINTER(AEffect), ctypes.c_int32, ctypes.c_float
    )
    AEffectGetParameterProc = ctypes.CFUNCTYPE(
        ctypes.c_float, ctypes.POINTER(AEffect), ctypes.c_int32
    )

    AEffect._fields_ = [
        ("magic", ctypes.c_int32),
        ("dispatcher", AEffectDispatcherProc),
        ("process", AEffectProcessProc),
        ("setParameter", AEffectSetParameterProc),
        ("getParameter", AEffectGetParameterProc),
        ("numPrograms", ctypes.c_int32),
        ("numParams", ctypes.c_int32),
        ("numInputs", ctypes.c_int32),
        ("numOutputs", ctypes.c_int32),
        ("flags", ctypes.c_int32),
        ("resvd1", ctypes.c_void_p),
        ("resvd2", ctypes.c_void_p),
        ("initialDelay", ctypes.c_int32),
        ("realQualities", ctypes.c_int32),
        ("offQualities", ctypes.c_int32),
        ("ioRatio", ctypes.c_float),
        ("object", ctypes.c_void_p),
        ("user", ctypes.c_void_p),
        ("uniqueID", ctypes.c_int32),
        ("version", ctypes.c_int32),
        ("processReplacing", AEffectProcessProc),
    ]

    io_changed_called = []
    @audioMasterCallback
    def mock_audio_master(effect, opcode, index, value, ptr, opt):
        if opcode == 13: # audioMasterIOChanged
            io_changed_called.append(True)
        return 0

    dylib.VSTPluginMain.restype = ctypes.POINTER(AEffect)
    dylib.VSTPluginMain.argtypes = [audioMasterCallback]

    effect_ptr = dylib.VSTPluginMain(mock_audio_master)
    assert effect_ptr, "VSTPluginMain returned NULL"
    e = effect_ptr.contents

    # Verify AEffect fields
    kEffectMagic = 0x56737450 # 'VstP'
    assert e.magic == kEffectMagic, f"Invalid magic: {hex(e.magic)}"
    assert e.numInputs == 2, f"Expected 2 inputs, got {e.numInputs}"
    assert e.numOutputs == 2, f"Expected 2 outputs, got {e.numOutputs}"
    assert e.numParams == 3, f"Expected 3 parameters, got {e.numParams}"
    assert e.initialDelay == 1024, f"Expected initialDelay 1024, got {e.initialDelay}"
    print(f"Verified VST2 structure: magic=VstP, inputs={e.numInputs}, outputs={e.numOutputs}, initialDelay={e.initialDelay}")

    # Query name via dispatcher
    effGetEffectName = 45
    name_buf = ctypes.create_string_buffer(64)
    e.dispatcher(effect_ptr, effGetEffectName, 0, 0, name_buf, 0.0)
    print(f"Plugin Name: {name_buf.value.decode('utf-8')}")
    assert name_buf.value == b"OpenGridder"

    # Query parameter 2 name (Latency)
    effGetParamName = 8
    param_buf = ctypes.create_string_buffer(64)
    e.dispatcher(effect_ptr, effGetParamName, 2, 0, param_buf, 0.0)
    print(f"Param 2 Name: {param_buf.value.decode('utf-8')}")
    assert param_buf.value == b"Latency"

    # Test changing latency profile to 3 (256s Direct = 1.0f normalized)
    e.setParameter(effect_ptr, 2, 1.0)
    assert e.initialDelay == 256, f"Expected initialDelay updated to 256, got {e.initialDelay}"
    assert len(io_changed_called) > 0, "audioMasterIOChanged was not notified to host"
    print("Verified dynamic PDC update: initialDelay changed to 256 samples and audioMasterIOChanged notified!")

    # Test processReplacing with silence
    frames = 128
    in0 = (ctypes.c_float * frames)()
    in1 = (ctypes.c_float * frames)()
    out0 = (ctypes.c_float * frames)()
    out1 = (ctypes.c_float * frames)()
    in_ptrs = (ctypes.POINTER(ctypes.c_float) * 2)(in0, in1)
    out_ptrs = (ctypes.POINTER(ctypes.c_float) * 2)(out0, out1)

    # Resume audio
    effMainsChanged = 12
    e.dispatcher(effect_ptr, effMainsChanged, 0, 1, None, 0.0)
    e.processReplacing(effect_ptr, in_ptrs, out_ptrs, frames)

    # Close plugin
    effClose = 1
    e.dispatcher(effect_ptr, effClose, 0, 0, None, 0.0)
    print("PASS: Universal VST2 plugin smoke test passed completely!")

if __name__ == "__main__":
    main()
