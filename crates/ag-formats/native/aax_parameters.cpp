// AAX data model: parameters, chunks, latency and transport for one ag_insert processor.
#include "aax_parameters.h"
#include "bridge.h"

#include "AAX_CBinaryDisplayDelegate.h"
#include "AAX_CBinaryTaperDelegate.h"
#include "AAX_CLinearTaperDelegate.h"
#include "AAX_CParameter.h"
#include "AAX_IController.h"
#include "AAX_ITransport.h"
#include "AAX_IDisplayDelegate.h"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <memory>

namespace {

// Display delegate that asks Rust for the text, so AAX shows the same units and
// choice labels ("-3.0 dB", "100%", "Tube") as VST3/AU/CLAP. Values are plain (lo..hi).
class AgDisplayDelegate : public AAX_IDisplayDelegate<float> {
public:
    AgDisplayDelegate(uint32_t index, float lo, float hi, uint32_t states)
        : mIndex(index), mLo(lo), mHi(hi), mStates(states) {}
    AgDisplayDelegate* Clone() const AAX_OVERRIDE { return new AgDisplayDelegate(*this); }

    bool ValueToString(float value, AAX_CString* out) const AAX_OVERRIDE {
        char text[128] = {};
        if (!out || !ag_insert_param_text(mIndex, value, text, sizeof text)) return false;
        out->Set(text);
        return true;
    }
    bool ValueToString(float value, int32_t maxChars, AAX_CString* out) const AAX_OVERRIDE {
        char text[128] = {};
        if (!out || !ag_insert_param_text(mIndex, value, text, sizeof text)) return false;
        std::string s(text);
        if (maxChars > 0 && (int32_t)s.size() > maxChars) {
            s.erase(std::remove(s.begin(), s.end(), ' '), s.end());  // "-3.0 dB" -> "-3.0dB"
            if ((int32_t)s.size() > maxChars) s.resize((size_t)maxChars);
        }
        out->Set(s.c_str());
        return true;
    }
    bool StringToValue(const AAX_CString& in, float* value) const AAX_OVERRIDE {
        if (!value) return false;
        std::string s = in.StdString();
        auto lower = [](std::string v) {
            for (auto& c : v) c = (char)std::tolower((unsigned char)c);
            return v;
        };
        if (mStates > 1) {  // match a choice label first
            for (uint32_t step = 0; step < mStates; ++step) {
                const float plain = mLo + (float)step;
                char text[128] = {};
                if (ag_insert_param_text(mIndex, plain, text, sizeof text) && lower(text) == lower(s)) {
                    *value = plain;
                    return true;
                }
            }
        }
        const char* begin = s.c_str();
        char* end = nullptr;
        const double parsed = std::strtod(begin, &end);
        if (end == begin || !std::isfinite(parsed)) return false;
        float v = std::clamp((float)parsed, std::min(mLo, mHi), std::max(mLo, mHi));
        if (mStates > 1) v = std::round(v);
        *value = v;
        return true;
    }

private:
    uint32_t mIndex;
    float mLo, mHi;
    uint32_t mStates;  // number of discrete states, 0 for continuous
};

}  // namespace

AAX_CEffectParameters* AAX_CALLBACK OpenGridder_Parameters::Create() { return new OpenGridder_Parameters(); }

OpenGridder_Parameters::OpenGridder_Parameters() : AAX_CEffectParameters() {}

OpenGridder_Parameters::~OpenGridder_Parameters() {
    if (mProcessor) {
        ag_insert_destroy(mProcessor);
        mProcessor = nullptr;
    }
}

const char* OpenGridder_Parameters::IdForIndex(uint32_t index) const {
    return index < mIds.size() ? mIds[index].c_str() : nullptr;
}

AAX_Result OpenGridder_Parameters::EffectInit() {
    mProcessor = ag_insert_create();
    if (!mProcessor) return AAX_ERROR_NULL_OBJECT;

    const uint32_t count = ag_insert_param_count();
    mIds.assign(count, std::string());
    for (uint32_t i = 0; i < count; ++i) {
        char name[128] = {};
        double lo = 0, hi = 1, def = 0;
        int32_t steps = 0;
        if (!ag_insert_param_info(i, name, &lo, &hi, &def, &steps)) continue;

        if (i == kOpenGridder_BypassIndex) {
            // Rust "bypass" is the AAX Master Bypass; the Rust side keeps the dry path latency-aligned.
            mIds[i] = cDefaultMasterBypassID;
            std::unique_ptr<AAX_IParameter> param(new AAX_CParameter<bool>(
                cDefaultMasterBypassID, AAX_CString("Master Bypass"), def >= 0.5,
                AAX_CBinaryTaperDelegate<bool>(), AAX_CBinaryDisplayDelegate<bool>("off", "on"), true));
            param->SetNumberOfSteps(2);
            param->SetType(AAX_eParameterType_Discrete);
            mParameterManager.AddParameter(param.release());
            mIndexOfId[mIds[i]] = i;
            continue;
        }

        char id[64] = {};
        if (!ag_insert_param_id(i, id, sizeof id) || !id[0]) snprintf(id, sizeof id, "p%u", i);
        mIds[i] = id;

        // Toggles and choices report steps; the remote-plugin selector is an index too.
        uint32_t states = steps > 0 ? (uint32_t)steps + 1 : 0;
        if (i == kOpenGridder_SelectionIndex && hi > lo) states = (uint32_t)std::lround(hi - lo) + 1;

        const float flo = (float)lo, fhi = (float)hi;
        const float plainDefault = (float)(lo + def * (hi - lo));
        // Latency changes must not be automated (AAX_IController::SetSignalLatency warning).
        const bool automatable = std::strcmp(id, "latency_profile") != 0;
        std::unique_ptr<AAX_IParameter> param(new AAX_CParameter<float>(
            id, AAX_CString(name), plainDefault, AAX_CLinearTaperDelegate<float>(flo, fhi),
            AgDisplayDelegate(i, flo, fhi, states), automatable));
        if (states > 1) {
            param->SetNumberOfSteps(states);
            param->SetType(AAX_eParameterType_Discrete);
        } else {
            param->SetType(AAX_eParameterType_Continuous);
        }
        mParameterManager.AddParameter(param.release());
        mIndexOfId[mIds[i]] = i;
    }

    AAX_CSampleRate rate = 48000;
    AAX_EStemFormat stem = AAX_eStemFormat_Stereo;
    if (Controller()) {
        Controller()->GetSampleRate(&rate);
        Controller()->GetInputStemFormat(&stem);
    }
    mChannels = std::clamp<int32_t>(AAX_STEM_FORMAT_CHANNEL_COUNT(stem), 1, 2);
    mPrepared = ag_insert_prepare(mProcessor, rate > 0 ? rate : 48000.0, kOpenGridder_MaxBlock, (uint32_t)mChannels);
    PublishLatency();
    return AAX_SUCCESS;
}

void OpenGridder_Parameters::PublishLatency() {
    if (!mProcessor || !Controller()) return;
    const int32_t latency = (int32_t)ag_insert_latency(mProcessor);
    if (latency != mReportedLatency) {
        mReportedLatency = latency;
        Controller()->SetSignalLatency(latency);
    }
}

AAX_Result OpenGridder_Parameters::UpdateParameterNormalizedValue(AAX_CParamID iParameterID, double iValue,
                                                                  AAX_EUpdateSource iSource) {
    const AAX_Result result = AAX_CEffectParameters::UpdateParameterNormalizedValue(iParameterID, iValue, iSource);
    if (result == AAX_SUCCESS && mProcessor && iParameterID) {
        auto found = mIndexOfId.find(iParameterID);
        if (found != mIndexOfId.end()) ag_insert_set(mProcessor, found->second, std::clamp(iValue, 0.0, 1.0));
    }
    return result;
}

AAX_Result OpenGridder_Parameters::ResetFieldData(AAX_CFieldIndex inFieldIndex, void* oData, uint32_t inDataSize) const {
    if (inFieldIndex == eAlgFieldID_Private && oData && inDataSize >= sizeof(SOpenGridder_Private)) {
        std::memset(oData, 0, inDataSize);
        auto* priv = static_cast<SOpenGridder_Private*>(oData);
        priv->mOwner = const_cast<OpenGridder_Parameters*>(this);
        priv->mChannels = mChannels;
        return AAX_SUCCESS;
    }
    return AAX_CEffectParameters::ResetFieldData(inFieldIndex, oData, inDataSize);
}

AAX_Result OpenGridder_Parameters::NotificationReceived(AAX_CTypeID inNotificationType, const void* inNotificationData,
                                                        uint32_t inNotificationDataSize) {
    switch (inNotificationType) {
        case AAX_eNotificationEvent_EnteringOfflineMode: mOffline.store(true); break;
        case AAX_eNotificationEvent_ExitingOfflineMode: mOffline.store(false); break;
        case AAX_eNotificationEvent_SignalLatencyChanged: break;
        default: break;
    }
    return AAX_CEffectParameters::NotificationReceived(inNotificationType, inNotificationData, inNotificationDataSize);
}

AAX_Result OpenGridder_Parameters::TimerWakeup() {
    PublishLatency();  // the latency profile is applied on the audio thread, then reported here
    return AAX_CEffectParameters::TimerWakeup();
}

AAX_Result OpenGridder_Parameters::GetCustomData(AAX_CTypeID iDataBlockID, uint32_t inDataSize, void* oData,
                                                 uint32_t* oDataWritten) const {
    if (iDataBlockID == cOpenGridder_ProcessorDataID) {
        if (!oData || inDataSize < sizeof(void*)) return AAX_ERROR_INVALID_ARGUMENT;
        std::memcpy(oData, &mProcessor, sizeof(void*));
        if (oDataWritten) *oDataWritten = sizeof(void*);
        return AAX_SUCCESS;
    }
    return AAX_CEffectParameters::GetCustomData(iDataBlockID, inDataSize, oData, oDataWritten);
}

// ---- Chunks ------------------------------------------------------------------------------

std::vector<uint8_t> OpenGridder_Parameters::SaveState() const {
    std::vector<uint8_t> bytes;
    if (!mProcessor) return bytes;
    const uint32_t size = ag_insert_save(mProcessor, nullptr, 0);
    if (size == 0) return bytes;
    bytes.resize(size);
    const uint32_t written = ag_insert_save(mProcessor, bytes.data(), size);
    bytes.resize(written <= size ? written : 0);
    return bytes;
}

AAX_Result OpenGridder_Parameters::GetNumberOfChunks(int32_t* oNumChunks) const {
    if (!oNumChunks) return AAX_ERROR_NULL_ARGUMENT;
    const AAX_Result result = AAX_CEffectParameters::GetNumberOfChunks(oNumChunks);
    if (result == AAX_SUCCESS) *oNumChunks += 1;
    return result;
}

AAX_Result OpenGridder_Parameters::GetChunkIDFromIndex(int32_t iIndex, AAX_CTypeID* oChunkID) const {
    if (!oChunkID) return AAX_ERROR_NULL_ARGUMENT;
    int32_t base = 0;
    AAX_CEffectParameters::GetNumberOfChunks(&base);
    if (iIndex == base) {
        *oChunkID = cOpenGridder_StateChunkID;
        return AAX_SUCCESS;
    }
    return AAX_CEffectParameters::GetChunkIDFromIndex(iIndex, oChunkID);
}

AAX_Result OpenGridder_Parameters::GetChunkSize(AAX_CTypeID iChunkID, uint32_t* oSize) const {
    if (iChunkID != cOpenGridder_StateChunkID) return AAX_CEffectParameters::GetChunkSize(iChunkID, oSize);
    if (!oSize) return AAX_ERROR_NULL_ARGUMENT;
    mChunkCache = SaveState();
    *oSize = (uint32_t)mChunkCache.size();
    return AAX_SUCCESS;
}

AAX_Result OpenGridder_Parameters::GetChunk(AAX_CTypeID iChunkID, AAX_SPlugInChunk* oChunk) const {
    if (iChunkID != cOpenGridder_StateChunkID) return AAX_CEffectParameters::GetChunk(iChunkID, oChunk);
    if (!oChunk) return AAX_ERROR_NULL_ARGUMENT;
    // The host sized the chunk from GetChunkSize; write exactly those bytes.
    if (oChunk->fSize < 0 || (size_t)oChunk->fSize < mChunkCache.size()) return AAX_ERROR_INCORRECT_CHUNK_SIZE;
    oChunk->fSize = (int32_t)mChunkCache.size();
    oChunk->fVersion = 1;
    std::memset(oChunk->fName, 0, sizeof oChunk->fName);
    std::strncpy(reinterpret_cast<char*>(oChunk->fName), "OpenGridder State", sizeof oChunk->fName - 1);
    if (!mChunkCache.empty()) std::memcpy(oChunk->fData, mChunkCache.data(), mChunkCache.size());
    return AAX_SUCCESS;
}

AAX_Result OpenGridder_Parameters::SetChunk(AAX_CTypeID iChunkID, const AAX_SPlugInChunk* iChunk) {
    if (iChunkID != cOpenGridder_StateChunkID) return AAX_CEffectParameters::SetChunk(iChunkID, iChunk);
    if (!iChunk) return AAX_ERROR_NULL_ARGUMENT;
    if (!mProcessor || iChunk->fSize <= 0) return AAX_ERROR_INCORRECT_CHUNK_SIZE;
    if (!ag_insert_load(mProcessor, reinterpret_cast<const uint8_t*>(iChunk->fData), (uint32_t)iChunk->fSize))
        return AAX_ERROR_INVALID_CHUNK_ID;
    PushProcessorValuesToHost();
    return AAX_SUCCESS;
}

AAX_Result OpenGridder_Parameters::CompareActiveChunk(const AAX_SPlugInChunk* iChunkP, AAX_CBoolean* oIsEqual) const {
    if (!iChunkP || !oIsEqual) return AAX_ERROR_NULL_ARGUMENT;
    if (iChunkP->fChunkID != cOpenGridder_StateChunkID) return AAX_CEffectParameters::CompareActiveChunk(iChunkP, oIsEqual);
    const std::vector<uint8_t> current = SaveState();
    *oIsEqual = iChunkP->fSize >= 0 && (size_t)iChunkP->fSize == current.size() &&
                (current.empty() || std::memcmp(current.data(), iChunkP->fData, current.size()) == 0);
    return AAX_SUCCESS;
}

// After a state load the processor holds new values; route them through the host so the
// parameter manager, automation lanes and control surfaces agree with the processor.
void OpenGridder_Parameters::PushProcessorValuesToHost() {
    for (uint32_t i = 0; i < mIds.size(); ++i) {
        const char* id = IdForIndex(i);
        if (!id || !*id) continue;
        AAX_IParameter* param = mParameterManager.GetParameterByID(id);
        if (!param) continue;
        const double value = ag_insert_get(mProcessor, i);
        if (std::fabs(param->GetNormalizedValue() - value) > 1e-7) SetParameterNormalizedValue(id, value);
    }
}

// ---- Audio thread ------------------------------------------------------------------------

void OpenGridder_Parameters::Process(float* const* inputs, float* const* outputs, int32_t frames) {
    const int32_t channels = mChannels;
    auto passThrough = [&](int32_t from) {
        for (int32_t ch = 0; ch < channels; ++ch)
            if (outputs[ch] && inputs[ch] && outputs[ch] != inputs[ch])
                std::memcpy(outputs[ch] + from, inputs[ch] + from, sizeof(float) * (size_t)(frames - from));
    };
    if (!mProcessor || !mPrepared || frames <= 0) {
        if (frames > 0) passThrough(0);
        return;
    }

    // Transport for the server (tempo-synced remote plugins).
    double bpm = 120.0, rate = 48000.0;
    int32_t num = 4, den = 4;
    int64_t sample = mFallbackSample, ticks = 0;
    bool playing = false, looping = false;
    if (const AAX_ITransport* transport = Transport()) {
        transport->GetCurrentTempo(&bpm);
        transport->GetCurrentMeter(&num, &den);
        transport->IsTransportPlaying(&playing);
        transport->GetCurrentNativeSampleLocation(&sample);
        transport->GetCurrentTickPosition(&ticks);
        int64_t loopStart = 0, loopEnd = 0;
        transport->GetCurrentLoopPosition(&looping, &loopStart, &loopEnd);
    }
    if (const AAX_IController* controller = Controller()) {
        AAX_CSampleRate sr = 0;
        if (controller->GetSampleRate(&sr) == AAX_SUCCESS && sr > 0) rate = sr;
    }
    mFallbackSample += frames;
    const bool offline = mOffline.load(std::memory_order_relaxed);

    for (int32_t cursor = 0; cursor < frames;) {
        const int32_t block = std::min<int32_t>(frames - cursor, (int32_t)kOpenGridder_MaxBlock);
        const float* in[2] = {};
        float* out[2] = {};
        for (int32_t ch = 0; ch < channels; ++ch) {
            in[ch] = inputs[ch] ? inputs[ch] + cursor : nullptr;
            out[ch] = outputs[ch] ? outputs[ch] + cursor : nullptr;
        }
        const int64_t at = sample + cursor;
        ag_insert_position(mProcessor, bpm, num, den, at, (double)at / rate,
                           (double)ticks / 960000.0 + (double)cursor / rate * bpm / 60.0, playing, false, looping);
        if (!ag_insert_process(mProcessor, in, out, (uint32_t)channels, (uint32_t)block, offline)) {
            passThrough(cursor);
            return;
        }
        cursor += block;
    }
}
