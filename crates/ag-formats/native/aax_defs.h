#pragma once
#include <stdint.h>
#include "AAX.h"

// OpenGridder AAX Identifiers. Native only: the insert streams to the OpenGridder
// server, so AudioSuite (offline, random access) is not described.
constexpr AAX_CTypeID cOpenGridder_ManufacturerID = 'OpGd';
constexpr AAX_CTypeID cOpenGridder_ProductID      = 'AGrd';
constexpr AAX_CTypeID cOpenGridder_PlugInID_Native = 'OpGa';      // stereo -> stereo
constexpr AAX_CTypeID cOpenGridder_PlugInID_NativeMono = 'OpGm';  // mono -> mono
constexpr AAX_CTypeID cOpenGridder_PlugInID_AudioSuite = 'OpGs';  // reserved, not described

#define kOpenGridder_EffectID "com.opengrid.opengridder.aax"

// Session state chunk, written with ag_insert_save and restored with ag_insert_load.
// It sits beside the SDK's controls chunk, which keeps Pro Tools' compare light working.
constexpr AAX_CTypeID cOpenGridder_StateChunkID = 'OGst';
// GetCustomData block that hands the GUI the ag_insert processor pointer.
constexpr AAX_CTypeID cOpenGridder_ProcessorDataID = 'OGpp';

// Parameter IDs are the Rust ParamInfo ids (ag_insert_param_id), so the AAX ids match the
// parameter table in ag-plugin: 1 "selection", 2 "latency_profile", 3 "input_gain", 4 "mix",
// 5 "drive", 6 "output_gain", 7 "drive_character", 8 "drive_quality", 9..24 "slot_0".."slot_15".
// Index 0 (Rust "bypass") is the AAX Master Bypass (cDefaultMasterBypassID).
constexpr uint32_t kOpenGridder_BypassIndex = 0;
constexpr uint32_t kOpenGridder_SelectionIndex = 1;  // remote plugin index 0..128, stepped

// Pro Tools' Native buffers never exceed 1024 frames; longer blocks are split anyway.
constexpr uint32_t kOpenGridder_MaxBlock = 1024;

class OpenGridder_Parameters;

// Private data written once by ResetFieldData: the data model that owns the processor.
struct SOpenGridder_Private {
    OpenGridder_Parameters* mOwner;
    int32_t mChannels;  // stem channel count (1 or 2)
};

// Context structure passed to the AAX processing callback
struct SOpenGridder_Context {
    float**               mInputPP;     // Audio input channel buffers [channel][sample]
    float**               mOutputPP;    // Audio output channel buffers [channel][sample]
    int32_t*              mBufferSize;  // Number of samples in current buffer
    SOpenGridder_Private* mPrivate;     // Owning data model (processor, offline flag)
};

// Algorithm Field IDs
enum EOpenGridder_FieldID {
    eAlgFieldID_AudioIn    = AAX_FIELD_INDEX(SOpenGridder_Context, mInputPP),
    eAlgFieldID_AudioOut   = AAX_FIELD_INDEX(SOpenGridder_Context, mOutputPP),
    eAlgFieldID_BufferSize = AAX_FIELD_INDEX(SOpenGridder_Context, mBufferSize),
    eAlgFieldID_Private    = AAX_FIELD_INDEX(SOpenGridder_Context, mPrivate),
};

void AAX_CALLBACK OpenGridder_AlgorithmProcessFunction(SOpenGridder_Context* const inInstancesBegin[],
                                                       const void* inInstancesEnd);
