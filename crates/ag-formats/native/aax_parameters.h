#pragma once
// AAX data model for OpenGridder. It owns one ag_insert processor and bridges the AAX
// parameter manager, chunks, latency and transport to the C ABI in bridge.h. No JUCE.
#include "AAX_CEffectParameters.h"
#include "aax_defs.h"
#include <atomic>
#include <map>
#include <string>
#include <vector>

class OpenGridder_Parameters : public AAX_CEffectParameters {
public:
    OpenGridder_Parameters();
    ~OpenGridder_Parameters() AAX_OVERRIDE;
    static AAX_CEffectParameters* AAX_CALLBACK Create();

    // AAX_CEffectParameters
    AAX_Result EffectInit() AAX_OVERRIDE;
    AAX_Result UpdateParameterNormalizedValue(AAX_CParamID iParameterID, double iValue, AAX_EUpdateSource iSource) AAX_OVERRIDE;
    AAX_Result ResetFieldData(AAX_CFieldIndex inFieldIndex, void* oData, uint32_t inDataSize) const AAX_OVERRIDE;
    AAX_Result NotificationReceived(AAX_CTypeID inNotificationType, const void* inNotificationData, uint32_t inNotificationDataSize) AAX_OVERRIDE;
    AAX_Result TimerWakeup() AAX_OVERRIDE;
    AAX_Result GetCustomData(AAX_CTypeID iDataBlockID, uint32_t inDataSize, void* oData, uint32_t* oDataWritten) const AAX_OVERRIDE;

    // Chunks: the SDK controls chunk plus the full ag_insert state (cOpenGridder_StateChunkID).
    AAX_Result GetNumberOfChunks(int32_t* oNumChunks) const AAX_OVERRIDE;
    AAX_Result GetChunkIDFromIndex(int32_t iIndex, AAX_CTypeID* oChunkID) const AAX_OVERRIDE;
    AAX_Result GetChunkSize(AAX_CTypeID iChunkID, uint32_t* oSize) const AAX_OVERRIDE;
    AAX_Result GetChunk(AAX_CTypeID iChunkID, AAX_SPlugInChunk* oChunk) const AAX_OVERRIDE;
    AAX_Result SetChunk(AAX_CTypeID iChunkID, const AAX_SPlugInChunk* iChunk) AAX_OVERRIDE;
    AAX_Result CompareActiveChunk(const AAX_SPlugInChunk* iChunkP, AAX_CBoolean* oIsEqual) const AAX_OVERRIDE;

    // Called by the algorithm on the audio thread.
    void Process(float* const* inputs, float* const* outputs, int32_t frames);

private:
    void PublishLatency();
    void PushProcessorValuesToHost();
    std::vector<uint8_t> SaveState() const;
    const char* IdForIndex(uint32_t index) const;

    void* mProcessor = nullptr;
    int32_t mChannels = 2;
    bool mPrepared = false;
    std::atomic<bool> mOffline{false};
    int32_t mReportedLatency = -1;
    int64_t mFallbackSample = 0;
    std::vector<std::string> mIds;               // ag index -> AAX parameter id
    std::map<std::string, uint32_t> mIndexOfId;  // AAX parameter id -> ag index
    mutable std::vector<uint8_t> mChunkCache;    // bytes measured by GetChunkSize, copied by GetChunk
};
