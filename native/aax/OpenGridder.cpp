// OpenGridder's SDK adapter only. DSP, transport and callback buffering are Rust.
// Avid SDK headers/libraries are supplied externally and are not redistributed.
#include "opengridder.h"
#include "AAX_CEffectParameters.h"
#include "AAX_CBinaryTaperDelegate.h"
#include "AAX_CBinaryDisplayDelegate.h"
#include "AAX_CLinearTaperDelegate.h"
#include "AAX_CNumberDisplayDelegate.h"
#include "AAX_IController.h"
#include "AAX_ICollection.h"
#include "AAX_IEffectDescriptor.h"
#include "AAX_IComponentDescriptor.h"
#include "AAX_IPropertyMap.h"
#include "AAX_Exception.h"
#include <algorithm>
#include <cmath>
#include <cstring>
#include <cstdio>
#include <cstdlib>

namespace {
constexpr const char* kSelection = "remote_plugin";
constexpr uint32_t kTransportLatency = 256;
double unitValue(double value) {
#if __cplusplus >= 201703L
    return std::clamp(value, 0.0, 1.0);
#else
    return std::max(0.0, std::min(1.0, value));
#endif
}
struct Instance { NativePlugin* rust; int32_t channels; };
struct Context {
    float** input;
    float** output;
    int32_t* frames;
    Instance* instance;
};
constexpr auto kInput = AAX_FIELD_INDEX(Context, input);
constexpr auto kOutput = AAX_FIELD_INDEX(Context, output);
constexpr auto kFrames = AAX_FIELD_INDEX(Context, frames);
constexpr auto kInstance = AAX_FIELD_INDEX(Context, instance);

void AAX_CALLBACK process(Context* const begin[], const void* end) {
    for (auto cursor = begin; static_cast<const void*>(cursor) < end; ++cursor) {
        auto* c = *cursor;
        if (!c || !c->instance || !c->frames || !c->input || !c->output) continue;
        const int frames = *c->frames;
        if (frames < 0) continue;
        const auto result = og_native_process(c->instance->rust,
            const_cast<const float* const*>(c->input), c->output, static_cast<uint32_t>(frames));
        if (result != 0) {
            for (int ch = 0; ch < c->instance->channels; ++ch)
                if (c->output[ch]) std::fill_n(c->output[ch], frames, 0.0f);
        }
    }
}

class Parameters final : public AAX_CEffectParameters {
public:
    ~Parameters() override { og_native_destroy(instance_.rust); }
    static AAX_CEffectParameters* AAX_CALLBACK create() { return new Parameters; }
    AAX_Result EffectInit() override {
        AAX_CSampleRate rate = 48000.0f;
        AAX_EStemFormat stem = AAX_eStemFormat_Stereo;
        AAX_CheckedResult result;
        result = Controller()->GetSampleRate(&rate);
        result = Controller()->GetOutputStemFormat(&stem);
        instance_.channels = stem == AAX_eStemFormat_Mono ? 1 : 2;
        const uint32_t transport = og_native_default_latency();
        instance_.rust = og_native_create(rate, 8192, instance_.channels, transport);
        lastLatency_ = transport;
        if (!instance_.rust) return AAX_ERROR_NULL_OBJECT;
        auto* bypass = new AAX_CParameter<bool>(cDefaultMasterBypassID, AAX_CString("Master Bypass"), false,
            AAX_CBinaryTaperDelegate<bool>(), AAX_CBinaryDisplayDelegate<bool>("Active", "Bypass"), true);
        bypass->SetNumberOfSteps(2);
        bypass->SetType(AAX_eParameterType_Discrete);
        mParameterManager.AddParameter(bypass);
        // A chain-loading control changes latency and must not be automatable.
        auto* select = new AAX_CParameter<int32_t>(kSelection, AAX_CString("Remote Plugin"), 0,
            AAX_CLinearTaperDelegate<int32_t>(0, 128), AAX_CNumberDisplayDelegate<int32_t>(), false);
        select->SetNumberOfSteps(129);
        select->SetType(AAX_eParameterType_Discrete);
        mParameterManager.AddParameter(select);
        return Controller()->SetSignalLatency(static_cast<int32_t>(transport));
    }
    AAX_Result ResetFieldData(AAX_CFieldIndex field, void* data, uint32_t bytes) const override {
        if (field == kInstance) {
            if (!data || bytes != sizeof(Instance)) return AAX_ERROR_INVALID_FIELD_INDEX;
            std::memcpy(data, &instance_, sizeof(instance_));
            return AAX_SUCCESS;
        }
        return AAX_CEffectParameters::ResetFieldData(field, data, bytes);
    }
    AAX_Result UpdateParameterNormalizedValue(AAX_CParamID id, double value, AAX_EUpdateSource source) override {
        if (!id || !std::isfinite(value)) return AAX_ERROR_INVALID_ARGUMENT;
        const auto result = AAX_CEffectParameters::UpdateParameterNormalizedValue(id, value, source);
        if (result == AAX_SUCCESS && id && std::isfinite(value)) {
            if (!std::strcmp(id, kSelection)) og_native_select(instance_.rust,
                static_cast<uint32_t>(std::lround(unitValue(value) * 128.0)));
            else if (!std::strcmp(id, cDefaultMasterBypassID)) og_native_bypass(instance_.rust, value >= 0.5);
        }
        return result;
    }
    AAX_Result GetParameterValueString(AAX_CParamID id, AAX_IString* text, int32_t maxLength) const override {
        if (id && text && !std::strcmp(id, kSelection)) {
            double normalized = 0.0;
            const auto result = AAX_CEffectParameters::GetParameterNormalizedValue(id, &normalized);
            if (result != AAX_SUCCESS) return result;
            return GetParameterStringFromValue(id, normalized, text, maxLength);
        }
        return AAX_CEffectParameters::GetParameterValueString(id, text, maxLength);
    }
    AAX_Result GetParameterStringFromValue(AAX_CParamID id, double normalized, AAX_IString* text, int32_t maxLength) const override {
        if (id && text && !std::strcmp(id, kSelection)) {
            if (!std::isfinite(normalized)) return AAX_ERROR_INVALID_STRING_CONVERSION;
            const auto index = static_cast<uint32_t>(std::lround(unitValue(normalized) * 128.0));
            char name[256] = {};
            char display[288] = {};
            og_native_name(instance_.rust, index, name, sizeof(name));
            std::snprintf(display, sizeof(display), "%u: %s", index, name);
            if (maxLength > 0 && maxLength < static_cast<int32_t>(sizeof(display))) display[maxLength] = 0;
            text->Set(display);
            return AAX_SUCCESS;
        }
        return AAX_CEffectParameters::GetParameterStringFromValue(id, normalized, text, maxLength);
    }
    AAX_Result GetParameterValueFromString(AAX_CParamID id, double* value, const AAX_IString& text) const override {
        if (id && value && !std::strcmp(id, kSelection)) {
            char* end = nullptr;
            const char* start = text.Get();
            if (!start) return AAX_ERROR_INVALID_STRING_CONVERSION;
            const long index = std::strtol(start, &end, 10);
            if (end == start || index < 0 || index > 128 || (*end != 0 && *end != ':'))
                return AAX_ERROR_INVALID_STRING_CONVERSION;
            *value = static_cast<double>(index) / 128.0;
            return AAX_SUCCESS;
        }
        return AAX_CEffectParameters::GetParameterValueFromString(id, value, text);
    }
    AAX_Result TimerWakeup() override {
        const auto base = AAX_CEffectParameters::TimerWakeup();
        const auto latency = og_native_latency(instance_.rust);
        if (latency != lastLatency_) {
            if (Controller()->SetSignalLatency(static_cast<int32_t>(latency)) == AAX_SUCCESS) lastLatency_ = latency;
        }
        int32_t acknowledged = 0;
        if (Controller()->GetSignalLatency(&acknowledged) == AAX_SUCCESS && acknowledged >= 0)
            og_native_accept_latency(instance_.rust, static_cast<uint32_t>(acknowledged));
        return base;
    }
    AAX_Result NotificationReceived(AAX_CTypeID type, const void* data, uint32_t size) override {
        if (type == AAX_eNotificationEvent_SignalLatencyChanged) {
            int32_t acknowledged = 0;
            if (Controller()->GetSignalLatency(&acknowledged) == AAX_SUCCESS && acknowledged >= 0)
                og_native_accept_latency(instance_.rust, static_cast<uint32_t>(acknowledged));
        }
        return AAX_CEffectParameters::NotificationReceived(type, data, size);
    }
private:
    Instance instance_{nullptr, 2};
    uint32_t lastLatency_ = kTransportLatency;
};

AAX_Result describe(AAX_IEffectDescriptor* effect, bool stereo) {
    AAX_CheckedResult result;
    auto* component = effect->NewComponentDescriptor();
    if (!component) return AAX_ERROR_NULL_OBJECT;
    result = component->AddAudioIn(kInput);
    result = component->AddAudioOut(kOutput);
    result = component->AddAudioBufferLength(kFrames);
    result = component->AddPrivateData(kInstance, sizeof(Instance));
    auto* properties = component->NewPropertyMap();
    if (!properties) return AAX_ERROR_NULL_OBJECT;
    result = properties->AddProperty(AAX_eProperty_ManufacturerID, 'OpGr');
    result = properties->AddProperty(AAX_eProperty_ProductID, 'OGFx');
    result = properties->AddProperty(AAX_eProperty_PlugInID_Native, stereo ? 'OGS1' : 'OGM1');
    result = properties->AddProperty(AAX_eProperty_InputStemFormat, stereo ? AAX_eStemFormat_Stereo : AAX_eStemFormat_Mono);
    result = properties->AddProperty(AAX_eProperty_OutputStemFormat, stereo ? AAX_eStemFormat_Stereo : AAX_eStemFormat_Mono);
    result = properties->AddProperty(AAX_eProperty_CanBypass, true);
    result = properties->AddProperty(AAX_eProperty_UsesClientGUI, true);
    result = component->AddProcessProc_Native(process, properties);
    result = effect->AddComponent(component);
    return result;
}
}

AAX_Result GetEffectDescriptions(AAX_ICollection* collection) {
    if (!collection) return AAX_ERROR_NULL_OBJECT;
    AAX_CheckedResult result;
    auto* effect = collection->NewDescriptor();
    if (!effect) return AAX_ERROR_NULL_OBJECT;
    result = effect->AddName("OpenGridder");
    result = effect->AddCategory(AAX_ePlugInCategory_WrappedPlugin);
    result = effect->AddProcPtr(reinterpret_cast<void*>(Parameters::create), kAAX_ProcPtrID_Create_EffectParameters);
    result = describe(effect, false);
    result = describe(effect, true);
    result = collection->AddEffect("studio.opengrid.OpenGridder.fx", effect);
    result = collection->SetManufacturerName("OpenGrid Studio");
    result = collection->AddPackageName("OpenGridder");
    result = collection->SetPackageVersion(0x01020100);
    return result;
}
