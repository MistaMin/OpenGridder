// OpenGridder AAX plug-in adapter for Pro Tools.
// Zero-Overhead: Pure Rust core + Native Cocoa GoodLookin UI + Avid AAX SDK.
#import "opengridder.h"
#import "OpenGridderEditorView.h"
#include "AAX_CEffectParameters.h"
#include "AAX_CEffectGUI.h"
#include "AAX_IViewContainer.h"
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
constexpr const char* kLatencyProfile = "latency_profile";
constexpr const char* kInputGain = "input_gain";
constexpr const char* kWetDryMix = "wet_dry_mix";
constexpr const char* kDrive = "drive";
constexpr const char* kOutputGain = "output_gain";
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

    NativePlugin* getRustHandle() const { return instance_.rust; }

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
        auto* profile = new AAX_CParameter<int32_t>(kLatencyProfile, AAX_CString("Latency Profile"), 1,
            AAX_CLinearTaperDelegate<int32_t>(0, 3), AAX_CNumberDisplayDelegate<int32_t>(), false);
        profile->SetNumberOfSteps(4);
        profile->SetType(AAX_eParameterType_Discrete);
        mParameterManager.AddParameter(profile);
        auto* inGain = new AAX_CParameter<float>(kInputGain, AAX_CString("Input Gain"), 0.0f,
            AAX_CLinearTaperDelegate<float>(-24.0f, 24.0f), AAX_CNumberDisplayDelegate<float, 1>(), true);
        mParameterManager.AddParameter(inGain);
        auto* mix = new AAX_CParameter<float>(kWetDryMix, AAX_CString("Wet/Dry Mix"), 100.0f,
            AAX_CLinearTaperDelegate<float>(0.0f, 100.0f), AAX_CNumberDisplayDelegate<float, 0>(), true);
        mParameterManager.AddParameter(mix);
        auto* drive = new AAX_CParameter<float>(kDrive, AAX_CString("Drive / Sat"), 0.0f,
            AAX_CLinearTaperDelegate<float>(0.0f, 10.0f), AAX_CNumberDisplayDelegate<float, 1>(), true);
        mParameterManager.AddParameter(drive);
        auto* outGain = new AAX_CParameter<float>(kOutputGain, AAX_CString("Output Trim"), 0.0f,
            AAX_CLinearTaperDelegate<float>(-24.0f, 24.0f), AAX_CNumberDisplayDelegate<float, 1>(), true);
        mParameterManager.AddParameter(outGain);
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
            else if (!std::strcmp(id, kLatencyProfile)) {
                float val = static_cast<float>(unitValue(value));
                int prof = std::clamp(static_cast<int>(val * 3.0f + 0.5f), 0, 3);
                uint32_t target = (prof == 0) ? 128 : (prof == 1) ? 256 : (prof == 2) ? 512 : 1024;
                if (instance_.rust) {
                    og_native_set_param(instance_.rust, 2, val);
                    og_native_set_transport_latency(instance_.rust, target);
                    og_native_accept_latency(instance_.rust, og_native_latency(instance_.rust));
                }
                lastLatency_ = instance_.rust ? og_native_latency(instance_.rust) : target;
                Controller()->SetSignalLatency(static_cast<int32_t>(lastLatency_));
            }
            else if (!std::strcmp(id, kInputGain)) og_native_set_param(instance_.rust, 3, static_cast<float>(unitValue(value)));
            else if (!std::strcmp(id, kWetDryMix)) og_native_set_param(instance_.rust, 4, static_cast<float>(unitValue(value)));
            else if (!std::strcmp(id, kDrive)) og_native_set_param(instance_.rust, 5, static_cast<float>(unitValue(value)));
            else if (!std::strcmp(id, kOutputGain)) og_native_set_param(instance_.rust, 6, static_cast<float>(unitValue(value)));
        }
        return result;
    }
    AAX_Result GetParameterValueString(AAX_CParamID id, AAX_IString* text, int32_t maxLength) const override {
        if (id && text && (!std::strcmp(id, kSelection) || !std::strcmp(id, kLatencyProfile))) {
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
        if (id && text && !std::strcmp(id, kLatencyProfile)) {
            if (!std::isfinite(normalized)) return AAX_ERROR_INVALID_STRING_CONVERSION;
            const auto index = static_cast<uint32_t>(std::lround(unitValue(normalized) * 3.0));
            const char* names[] = {"LOW (128)", "STD (256)", "HIGH (512)", "STUDIO (1024)"};
            text->Set(names[index % 4]);
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
        if (id && value && !std::strcmp(id, kLatencyProfile)) {
            char* end = nullptr;
            const char* start = text.Get();
            if (!start) return AAX_ERROR_INVALID_STRING_CONVERSION;
            const long index = std::strtol(start, &end, 10);
            if (end == start || index < 0 || index > 3)
                return AAX_ERROR_INVALID_STRING_CONVERSION;
            *value = static_cast<double>(index) / 3.0;
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

class OpenGridderAAXGUI final : public AAX_CEffectGUI {
public:
    static AAX_IEffectGUI* AAX_CALLBACK create() { return new OpenGridderAAXGUI; }

    void CreateViewContents() override {}

    AAX_Result GetViewSize(AAX_Point* oViewSize) const override {
        if (oViewSize) {
            NSSize sz = mEditorView ? [mEditorView preferredViewSize] : NSZeroSize;
            if (sz.width < 100.0 || sz.height < 100.0) sz = NSMakeSize(864.0, 420.0);
            oViewSize->vert = static_cast<float>(sz.height);
            oViewSize->horz = static_cast<float>(sz.width);
        }
        return AAX_SUCCESS;
    }

    void CreateViewContainer() override {
        if (!GetViewContainer()) return;
        if (GetViewContainer()->GetType() == AAX_eViewContainer_Type_NSView) {
            NSView* parentView = (__bridge NSView*)GetViewContainer()->GetPtr();
            NSRect frame = NSMakeRect(0, 0, 864, 420);
            mEditorView = [[OpenGridderAAXEditorView alloc] initWithFrame:frame];
            mEditorView.rustProvider = ^NativePlugin* {
                auto* p = static_cast<Parameters*>(GetEffectParameters());
                return p ? p->getRustHandle() : nullptr;
            };
            auto* params = static_cast<Parameters*>(GetEffectParameters());
            if (params) {
                mEditorView.rust = params->getRustHandle();
            }
            mEditorView.onParameterChanged = ^(uint32_t paramId, float val) {
                auto* p = static_cast<Parameters*>(GetEffectParameters());
                if (p) {
                    if (paramId == 0) {
                        p->SetParameterNormalizedValue(cDefaultMasterBypassID, val);
                    } else if (paramId == 1) {
                        p->SetParameterNormalizedValue(kSelection, val);
                    } else if (paramId == 3) {
                        p->SetParameterNormalizedValue(kInputGain, val);
                    } else if (paramId == 4) {
                        p->SetParameterNormalizedValue(kWetDryMix, val);
                    } else if (paramId == 5) {
                        p->SetParameterNormalizedValue(kDrive, val);
                    } else if (paramId == 6) {
                        p->SetParameterNormalizedValue(kOutputGain, val);
                    }
                }
            };
            mEditorView.onProfileChanged = ^(int newProfile) {
                auto* p = static_cast<Parameters*>(GetEffectParameters());
                if (p) {
                    p->SetParameterNormalizedValue(kLatencyProfile, static_cast<double>(newProfile) / 3.0);
                }
            };
            OpenGridderAAXGUI* selfGui = this;
            mEditorView.onSizeChanged = ^(CGFloat newW, CGFloat newH) {
                if (selfGui->GetViewContainer()) {
                    AAX_Point pt(static_cast<float>(newH), static_cast<float>(newW));
                    selfGui->GetViewContainer()->SetViewSize(pt);
                }
            };
            [parentView addSubview:mEditorView];
            NSSize initSz = [mEditorView preferredViewSize];
            AAX_Point size(static_cast<float>(initSz.height), static_cast<float>(initSz.width));
            GetViewContainer()->SetViewSize(size);
        }
    }

    void DeleteViewContainer() override {
        if (mEditorView) {
            [mEditorView removeFromSuperview];
            mEditorView = nil;
        }
    }

    AAX_Result TimerWakeup() override {
        if (mEditorView) {
            auto* params = static_cast<Parameters*>(GetEffectParameters());
            NativePlugin* rust = params ? params->getRustHandle() : nullptr;
            if (rust) {
                if (!mEditorView.rust) {
                    mEditorView.rust = rust;
                }
                int status = og_native_status(rust);
                uint32_t lat = og_native_latency(rust);
                [mEditorView updateStatus:status latencySamples:lat];
            }
        }
        return AAX_CEffectGUI::TimerWakeup();
    }

private:
    OpenGridderAAXEditorView* mEditorView = nil;
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
    result = effect->AddProcPtr(reinterpret_cast<void*>(OpenGridderAAXGUI::create), kAAX_ProcPtrID_Create_EffectGUI);
    result = describe(effect, false);
    result = describe(effect, true);
    result = effect->AddResourceInfo(AAX_eResourceType_PageTable, "OpenGridderPages.xml");
    result = collection->AddEffect("studio.opengrid.OpenGridder.fx", effect);
    result = collection->SetManufacturerName("OpenGrid Studio");
    result = collection->AddPackageName("OpenGridder");
    result = collection->SetPackageVersion(0x01020100);
    return result;
}
