#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define kEffectMagic 0x56737450 // 'VstP'

enum VstAEffectFlags {
    effFlagsHasEditor     = 1 << 0,
    effFlagsCanReplacing  = 1 << 4,
    effFlagsProgramChunks = 1 << 5,
    effFlagsIsSynth       = 1 << 8,
    effFlagsNoSoundInStop = 1 << 9,
    effFlagsCanDoubleReplacing = 1 << 12
};

enum VstOpcodes {
    effOpen = 0,
    effClose = 1,
    effSetProgram = 2,
    effGetProgram = 3,
    effSetProgramName = 4,
    effGetProgramName = 5,
    effGetParamLabel = 6,
    effGetParamDisplay = 7,
    effGetParamName = 8,
    effSetSampleRate = 10,
    effSetBlockSize = 11,
    effMainsChanged = 12,
    effEditGetRect = 13,
    effEditOpen = 14,
    effEditClose = 15,
    effGetChunk = 23,
    effSetChunk = 24,
    effProcessEvents = 25,
    effCanBeAutomated = 26,
    effString2Parameter = 27,
    effGetProgramNameIndexed = 29,
    effGetInputProperties = 33,
    effGetOutputProperties = 34,
    effGetPlugCategory = 35,
    effSetSpeakerArrangement = 42,
    effSetBypass = 44,
    effGetEffectName = 45,
    effGetVendorString = 47,
    effGetProductString = 48,
    effGetVendorVersion = 49,
    effVendorSpecific = 50,
    effCanDo = 51,
    effGetTailSize = 52,
    effGetParameterProperties = 56,
    effGetVstVersion = 58,
    effEditKeyDown = 59,
    effEditKeyUp = 60,
    effSetEditKnobMode = 61,
    effGetMidiProgramName = 62,
    effGetCurrentMidiProgram = 63,
    effGetMidiProgramCategory = 64,
    effHasMidiProgramsChanged = 65,
    effGetMidiKeyBasedInstrumentProperties = 66,
    effBeginSetProgram = 67,
    effEndSetProgram = 68,
    effGetSpeakerArrangement = 69,
    effShellGetNextPlugin = 70,
    effStartProcess = 71,
    effStopProcess = 72,
    effSetTotalSampleToProcess = 73,
    effSetPanLaw = 74,
    effBeginLoadBank = 75,
    effBeginLoadProgram = 76,
    effSetProcessPrecision = 77,
    effGetNumMidiInputChannels = 78,
    effGetNumMidiOutputChannels = 79
};

enum AudioMasterOpcodes {
    audioMasterAutomate = 0,
    audioMasterVersion = 1,
    audioMasterCurrentId = 2,
    audioMasterIdle = 3,
    audioMasterPinConnected = 4,
    audioMasterWantMidi = 6,
    audioMasterGetTime = 7,
    audioMasterProcessEvents = 8,
    audioMasterSetTime = 9,
    audioMasterTempoAt = 10,
    audioMasterGetNumAutomatableParameters = 11,
    audioMasterGetParameterQuantization = 12,
    audioMasterIOChanged = 13,
    audioMasterNeedIdle = 14,
    audioMasterSizeWindow = 15,
    audioMasterGetSampleRate = 16,
    audioMasterGetBlockSize = 17,
    audioMasterGetInputLatency = 18,
    audioMasterGetOutputLatency = 19,
    audioMasterGetPreviousPlug = 20,
    audioMasterGetNextPlug = 21,
    audioMasterWillReplaceOrAccumulate = 22,
    audioMasterGetCurrentUniqueId = 23,
    audioMasterSetOutputSampleRate = 24,
    audioMasterGetOutputSpeakerArrangement = 25,
    audioMasterGetVendorString = 26,
    audioMasterGetProductString = 27,
    audioMasterGetVendorVersion = 28,
    audioMasterVendorSpecific = 29,
    audioMasterSetIcon = 30,
    audioMasterCanDo = 31,
    audioMasterGetLanguage = 32,
    audioMasterOpenWindow = 33,
    audioMasterCloseWindow = 34,
    audioMasterGetDirectory = 35,
    audioMasterUpdateDisplay = 36,
    audioMasterBeginEdit = 37,
    audioMasterEndEdit = 38,
    audioMasterOpenFileSelector = 39,
    audioMasterCloseFileSelector = 40,
    audioMasterEditFile = 41,
    audioMasterGetChunkFile = 42,
    audioMasterGetInputSpeakerArrangement = 43
};

struct AEffect;

typedef intptr_t (*audioMasterCallback)(struct AEffect* effect, int32_t opcode, int32_t index, intptr_t value, void* ptr, float opt);
typedef intptr_t (*AEffectDispatcherProc)(struct AEffect* effect, int32_t opcode, int32_t index, intptr_t value, void* ptr, float opt);
typedef void (*AEffectProcessProc)(struct AEffect* effect, float** inputs, float** outputs, int32_t sampleFrames);
typedef void (*AEffectProcessDoubleProc)(struct AEffect* effect, double** inputs, double** outputs, int32_t sampleFrames);
typedef void (*AEffectSetParameterProc)(struct AEffect* effect, int32_t index, float parameter);
typedef float (*AEffectGetParameterProc)(struct AEffect* effect, int32_t index);

#pragma pack(push, 8)
struct AEffect {
    int32_t magic;
    AEffectDispatcherProc dispatcher;
    AEffectProcessProc process; // Accumulating process (deprecated in 2.4)
    AEffectSetParameterProc setParameter;
    AEffectGetParameterProc getParameter;

    int32_t numPrograms;
    int32_t numParams;
    int32_t numInputs;
    int32_t numOutputs;

    int32_t flags;
    intptr_t resvd1;
    intptr_t resvd2;

    int32_t initialDelay;
    int32_t realQualities;
    int32_t offQualities;
    float ioRatio;

    void* object;
    void* user;

    int32_t uniqueID;
    int32_t version;

    AEffectProcessProc processReplacing;
    AEffectProcessDoubleProc processDoubleReplacing;

    char future[56];
};
#pragma pack(pop)

#ifdef __cplusplus
}
#endif
