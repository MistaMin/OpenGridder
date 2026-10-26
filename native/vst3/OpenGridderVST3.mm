// OpenGridder VST3 adapter for modern DAWs (Cubase, Studio One, Bitwig, Reaper).
// Zero-Overhead implementation adhering directly to Steinberg VST 3.7 C++ interfaces.

#include "pluginterfaces/base/ipluginbase.h"
#include "pluginterfaces/base/ibstream.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivstcomponent.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/ivstparameterchanges.h"
#include "pluginterfaces/vst/vstspeaker.h"
#include "pluginterfaces/gui/iplugview.h"

#import "OpenGridderEditorView.h"
#include "opengridder.h"

namespace Steinberg {
DEF_CLASS_IID(IPlugView)
}

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

using namespace Steinberg;
using namespace Steinberg::Vst;

namespace {

// 5DF7BBA0-CFDB-11F1-898A-00505692C25A
static const TUID kOpenGridderProcessorUID = {
    (char)0x5D, (char)0xF7, (char)0xBB, (char)0xA0, (char)0xCF, (char)0xDB, (char)0x11, (char)0xF1,
    (char)0x89, (char)0x8A, (char)0x00, (char)0x50, (char)0x56, (char)0x92, (char)0xC2, (char)0x5A
};

// 5DF7BBA1-CFDB-11F1-898A-00505692C25A
static const TUID kOpenGridderControllerUID = {
    (char)0x5D, (char)0xF7, (char)0xBB, (char)0xA1, (char)0xCF, (char)0xDB, (char)0x11, (char)0xF1,
    (char)0x89, (char)0x8A, (char)0x00, (char)0x50, (char)0x56, (char)0x92, (char)0xC2, (char)0x5A
};

enum ParamIDs : ParamID {
    kParamBypass = 0,
    kParamSelection = 1,
    kParamLatencyProfile = 2,
    kParamInputGain = 3,
    kParamWetDryMix = 4,
    kParamDrive = 5,
    kParamOutputGain = 6,
    kNumParams = 7
};

inline uint32_t targetLatencyFromProfile(int profile) {
    switch (profile) {
        case 0: return 128;
        case 1: return 256;
        case 2: return 512;
        default: return 1024;
    }
}

class OpenGridderVST3 : public IComponent,
                        public IAudioProcessor,
                        public IEditController {
public:
    OpenGridderVST3()
        : refCount_(1),
          rust_(nullptr),
          hostContext_(nullptr),
          componentHandler_(nullptr),
          sampleRate_(48000.0),
          maxBlockSize_(8192),
          bypass_(false),
          selection_(0),
          profile_(1),
          inputGain_(0.5),
          wetDryMix_(1.0),
          drive_(0.0),
          outputGain_(0.5),
          active_(false),
          lastLatency_(targetLatencyFromProfile(1))
    {
        createRust();
    }

    virtual ~OpenGridderVST3() {
        cleanupRust();
    }

    // --- FUnknown ---
    tresult PLUGIN_API queryInterface(const TUID _iid, void** obj) override {
        if (FUnknownPrivate::iidEqual(_iid, FUnknown::iid)) {
            addRef();
            *obj = static_cast<IComponent*>(this);
            return kResultOk;
        }
        if (FUnknownPrivate::iidEqual(_iid, IPluginBase::iid)) {
            addRef();
            *obj = static_cast<IComponent*>(this);
            return kResultOk;
        }
        QUERY_INTERFACE(_iid, obj, IComponent::iid, IComponent)
        QUERY_INTERFACE(_iid, obj, IAudioProcessor::iid, IAudioProcessor)
        QUERY_INTERFACE(_iid, obj, IEditController::iid, IEditController)
        *obj = nullptr;
        return kNoInterface;
    }

    uint32 PLUGIN_API addRef() override {
        return ++refCount_;
    }

    uint32 PLUGIN_API release() override {
        uint32 r = --refCount_;
        if (r == 0) delete this;
        return r;
    }

    // --- IPluginBase ---
    tresult PLUGIN_API initialize(FUnknown* context) override {
        hostContext_ = context;
        return kResultOk;
    }

    tresult PLUGIN_API terminate() override {
        hostContext_ = nullptr;
        componentHandler_ = nullptr;
        return kResultOk;
    }

    // --- IComponent ---
    tresult PLUGIN_API getControllerClassId(TUID classId) override {
        // Combined component and controller in single object
        return kResultFalse;
    }

    tresult PLUGIN_API setIoMode(IoMode mode) override {
        return (mode == kAdvanced) ? kResultFalse : kResultOk;
    }

    int32 PLUGIN_API getBusCount(MediaType type, BusDirection dir) override {
        return (type == kAudio) ? 1 : 0;
    }

    tresult PLUGIN_API getBusInfo(MediaType type, BusDirection dir, int32 index, BusInfo& bus) override {
        if (type != kAudio || index != 0) return kResultFalse;
        bus.mediaType = kAudio;
        bus.direction = dir;
        bus.channelCount = 2; // Stereo
        strToChar16((dir == kInput ? "Main In" : "Main Out"), bus.name);
        bus.busType = kMain;
        bus.flags = BusInfo::kDefaultActive;
        return kResultOk;
    }

    tresult PLUGIN_API getRoutingInfo(RoutingInfo& inInfo, RoutingInfo& outInfo) override {
        if (inInfo.mediaType == kAudio && inInfo.busIndex == 0) {
            outInfo.mediaType = kAudio;
            outInfo.busIndex = 0;
            outInfo.channel = inInfo.channel;
            return kResultOk;
        }
        return kResultFalse;
    }

    tresult PLUGIN_API activateBus(MediaType type, BusDirection dir, int32 index, TBool state) override {
        return (type == kAudio && index == 0) ? kResultOk : kResultFalse;
    }

    tresult PLUGIN_API setActive(TBool state) override {
        active_ = (state != 0);
        return kResultOk;
    }

    tresult PLUGIN_API setState(IBStream* state) override {
        if (!state) return kResultFalse;
        int32 magicOrVer = 0;
        int32 bytesRead = 0;
        if (state->read(&magicOrVer, sizeof(magicOrVer), &bytesRead) != kResultOk) return kResultFalse;

        if (magicOrVer == 0x4F475337) { // 'OGS7'
            uint32_t jsonLen = 0;
            if (state->read(&jsonLen, sizeof(jsonLen), &bytesRead) != kResultOk || jsonLen == 0 || jsonLen > 16 * 1024 * 1024) {
                return kResultFalse;
            }
            std::vector<uint8_t> buf(jsonLen);
            if (state->read(buf.data(), static_cast<int32>(jsonLen), &bytesRead) != kResultOk) {
                return kResultFalse;
            }
            if (rust_) {
                og_native_set_state(rust_, buf.data(), jsonLen);
                bypass_ = og_native_get_param(rust_, kParamBypass) >= 0.5f;
                selection_ = static_cast<int>(og_native_get_param(rust_, kParamSelection) * 128.0f + 0.5f);
                profile_ = static_cast<int>(og_native_get_param(rust_, kParamLatencyProfile) * 3.0f + 0.5f);
                inputGain_ = og_native_get_param(rust_, kParamInputGain);
                wetDryMix_ = og_native_get_param(rust_, kParamWetDryMix);
                drive_ = og_native_get_param(rust_, kParamDrive);
                outputGain_ = og_native_get_param(rust_, kParamOutputGain);
            }
            return kResultOk;
        }

        // Legacy v1 binary state fallback
        int32 count = 0;
        if (state->read(&count, sizeof(count), &bytesRead) != kResultOk) return kResultFalse;

        int32 sel = 0, prof = 1, byp = 0;
        if (count >= 1) state->read(&byp, sizeof(byp), &bytesRead);
        if (count >= 2) state->read(&sel, sizeof(sel), &bytesRead);
        if (count >= 3) state->read(&prof, sizeof(prof), &bytesRead);

        bypass_ = (byp != 0);
        selection_ = sel;
        profile_ = std::clamp(prof, 0, 3);

        if (rust_) {
            og_native_bypass(rust_, bypass_);
            og_native_select(rust_, static_cast<uint32_t>(selection_));
            og_native_set_transport_latency(rust_, targetLatencyFromProfile(profile_));
        }
        return kResultOk;
    }

    tresult PLUGIN_API getState(IBStream* state) override {
        if (!state) return kResultFalse;
        if (rust_) {
            uint32_t jsonLen = og_native_get_state(rust_, nullptr, 0);
            if (jsonLen > 0) {
                std::vector<uint8_t> buf(jsonLen);
                uint32_t written = og_native_get_state(rust_, buf.data(), jsonLen);
                int32 magic = 0x4F475337; // 'OGS7'
                int32 bytesWritten = 0;
                state->write(&magic, sizeof(magic), &bytesWritten);
                state->write(&written, sizeof(written), &bytesWritten);
                state->write(buf.data(), static_cast<int32>(written), &bytesWritten);
                return kResultOk;
            }
        }
        int32 version = 1;
        int32 count = 3;
        int32 byp = bypass_ ? 1 : 0;
        int32 sel = selection_;
        int32 prof = profile_;
        int32 bytesWritten = 0;

        state->write(&version, sizeof(version), &bytesWritten);
        state->write(&count, sizeof(count), &bytesWritten);
        state->write(&byp, sizeof(byp), &bytesWritten);
        state->write(&sel, sizeof(sel), &bytesWritten);
        state->write(&prof, sizeof(prof), &bytesWritten);
        return kResultOk;
    }

    // --- IAudioProcessor ---
    tresult PLUGIN_API setBusArrangements(SpeakerArrangement* inputs, int32 numIns,
                                          SpeakerArrangement* outputs, int32 numOuts) override {
        if (numIns != 1 || numOuts != 1) return kResultFalse;
        if (inputs[0] != SpeakerArr::kStereo || outputs[0] != SpeakerArr::kStereo) return kResultFalse;
        return kResultOk;
    }

    tresult PLUGIN_API getBusArrangement(BusDirection dir, int32 index, SpeakerArrangement& arr) override {
        if (index != 0) return kResultFalse;
        arr = SpeakerArr::kStereo;
        return kResultOk;
    }

    tresult PLUGIN_API canProcessSampleSize(int32 symbolicSampleSize) override {
        return (symbolicSampleSize == kSample32 || symbolicSampleSize == kSample64) ? kResultOk : kResultFalse;
    }

    uint32 PLUGIN_API getLatencySamples() override {
        if (rust_) {
            uint32_t lat = og_native_latency(rust_);
            if (lat > 0) return lat;
        }
        return targetLatencyFromProfile(profile_);
    }

    tresult PLUGIN_API setupProcessing(ProcessSetup& setup) override {
        sampleRate_ = setup.sampleRate;
        maxBlockSize_ = std::max<uint32_t>(setup.maxSamplesPerBlock, 1024);
        if (!rust_) {
            createRust();
        }
        return kResultOk;
    }

    tresult PLUGIN_API setProcessing(TBool state) override {
        return kResultOk;
    }

    tresult PLUGIN_API process(ProcessData& data) override {
        if (data.inputParameterChanges) {
            int32 numChanges = data.inputParameterChanges->getParameterCount();
            for (int32 i = 0; i < numChanges; ++i) {
                IParamValueQueue* queue = data.inputParameterChanges->getParameterData(i);
                if (queue && queue->getPointCount() > 0) {
                    ParamValue val = 0.0;
                    int32 sampleOffset = 0;
                    if (queue->getPoint(queue->getPointCount() - 1, sampleOffset, val) == kResultOk) {
                        applyParameter(queue->getParameterId(), val);
                    }
                }
            }
        }

        if (data.numSamples <= 0) return kResultOk;
        if (data.numInputs < 1 || data.numOutputs < 1) return kResultOk;

        if (data.symbolicSampleSize == kSample64) {
            double* inL = data.inputs[0].channelBuffers64[0];
            double* inR = (data.inputs[0].numChannels > 1) ? data.inputs[0].channelBuffers64[1] : inL;
            double* outL = data.outputs[0].channelBuffers64[0];
            double* outR = (data.outputs[0].numChannels > 1) ? data.outputs[0].channelBuffers64[1] : outL;
            const double* inputs[2] = { inL, inR };
            double* outputs[2] = { outL, outR };
            if (!rust_ || bypass_) {
                if (outL != inL) std::memcpy(outL, inL, sizeof(double) * data.numSamples);
                if (outR != inR) std::memcpy(outR, inR, sizeof(double) * data.numSamples);
                return kResultOk;
            }
            uint32_t curLat = og_native_latency(rust_);
            if (curLat != lastLatency_) {
                lastLatency_ = curLat;
                if (componentHandler_) componentHandler_->restartComponent(kLatencyChanged);
            }
            og_native_accept_latency(rust_, lastLatency_);
            og_native_process_f64(rust_, inputs, outputs, static_cast<uint32_t>(data.numSamples));
            return kResultOk;
        }

        float* inL = data.inputs[0].channelBuffers32[0];
        float* inR = (data.inputs[0].numChannels > 1) ? data.inputs[0].channelBuffers32[1] : inL;
        float* outL = data.outputs[0].channelBuffers32[0];
        float* outR = (data.outputs[0].numChannels > 1) ? data.outputs[0].channelBuffers32[1] : outL;

        const float* inputs[2] = { inL, inR };
        float* outputs[2] = { outL, outR };

        if (!rust_ || bypass_) {
            if (outL != inL) std::memcpy(outL, inL, sizeof(float) * data.numSamples);
            if (outR != inR) std::memcpy(outR, inR, sizeof(float) * data.numSamples);
            return kResultOk;
        }

        uint32_t curLat = og_native_latency(rust_);
        if (curLat != lastLatency_) {
            lastLatency_ = curLat;
            if (componentHandler_) {
                componentHandler_->restartComponent(kLatencyChanged);
            }
        }
        og_native_accept_latency(rust_, lastLatency_);

        og_native_process(rust_, inputs, outputs, static_cast<uint32_t>(data.numSamples));
        return kResultOk;
    }

    uint32 PLUGIN_API getTailSamples() override {
        return kNoTail;
    }

    // --- IEditController ---
    tresult PLUGIN_API setComponentState(IBStream* state) override {
        return setState(state);
    }

    tresult PLUGIN_API setComponentHandler(IComponentHandler* handler) override {
        componentHandler_ = handler;
        return kResultOk;
    }

    int32 PLUGIN_API getParameterCount() override {
        return kNumParams;
    }

    tresult PLUGIN_API getParameterInfo(int32 paramIndex, ParameterInfo& info) override {
        std::memset(&info, 0, sizeof(info));
        switch (paramIndex) {
            case kParamBypass:
                info.id = kParamBypass;
                strToChar16("Bypass", info.title);
                strToChar16("Bypass", info.shortTitle);
                info.stepCount = 1;
                info.defaultNormalizedValue = 0.0;
                info.flags = ParameterInfo::kCanAutomate | ParameterInfo::kIsBypass;
                return kResultOk;

            case kParamSelection:
                info.id = kParamSelection;
                strToChar16("Remote Plugin", info.title);
                strToChar16("Plugin", info.shortTitle);
                info.stepCount = 128;
                info.defaultNormalizedValue = 0.0;
                info.flags = ParameterInfo::kCanAutomate;
                return kResultOk;

            case kParamLatencyProfile:
                info.id = kParamLatencyProfile;
                strToChar16("Latency Profile", info.title);
                strToChar16("Latency", info.shortTitle);
                info.stepCount = 3;
                info.defaultNormalizedValue = 1.0 / 3.0;
                info.flags = ParameterInfo::kCanAutomate;
                return kResultOk;

            case kParamInputGain:
                info.id = kParamInputGain;
                strToChar16("Input Gain", info.title);
                strToChar16("InGain", info.shortTitle);
                strToChar16("dB", info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue = 0.5;
                info.flags = ParameterInfo::kCanAutomate;
                return kResultOk;

            case kParamWetDryMix:
                info.id = kParamWetDryMix;
                strToChar16("Wet/Dry Mix", info.title);
                strToChar16("Mix", info.shortTitle);
                strToChar16("%", info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue = 1.0;
                info.flags = ParameterInfo::kCanAutomate;
                return kResultOk;

            case kParamDrive:
                info.id = kParamDrive;
                strToChar16("Drive / Sat", info.title);
                strToChar16("Drive", info.shortTitle);
                info.stepCount = 0;
                info.defaultNormalizedValue = 0.0;
                info.flags = ParameterInfo::kCanAutomate;
                return kResultOk;

            case kParamOutputGain:
                info.id = kParamOutputGain;
                strToChar16("Output Trim", info.title);
                strToChar16("OutTrim", info.shortTitle);
                strToChar16("dB", info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue = 0.5;
                info.flags = ParameterInfo::kCanAutomate;
                return kResultOk;

            default:
                return kResultFalse;
        }
    }

    tresult PLUGIN_API getParamStringByValue(ParamID id, ParamValue valueNormalized, String128 string) override {
        char buf[128];
        if (id == kParamBypass) {
            snprintf(buf, sizeof(buf), "%s", valueNormalized >= 0.5 ? "Bypassed" : "Active");
        } else if (id == kParamSelection) {
            int sel = static_cast<int>(valueNormalized * 128.0 + 0.5);
            snprintf(buf, sizeof(buf), "%d: Remote", sel);
        } else if (id == kParamLatencyProfile) {
            int p = static_cast<int>(valueNormalized * 3.0 + 0.5);
            switch (p) {
                case 0: snprintf(buf, sizeof(buf), "LOW (128)"); break;
                case 1: snprintf(buf, sizeof(buf), "STD (256)"); break;
                case 2: snprintf(buf, sizeof(buf), "HIGH (512)"); break;
                default: snprintf(buf, sizeof(buf), "STUDIO (1024)"); break;
            }
        } else if (id == kParamInputGain || id == kParamOutputGain) {
            snprintf(buf, sizeof(buf), "%+.1f dB", (valueNormalized - 0.5) * 48.0);
        } else if (id == kParamWetDryMix) {
            snprintf(buf, sizeof(buf), "%.0f %%", valueNormalized * 100.0);
        } else if (id == kParamDrive) {
            snprintf(buf, sizeof(buf), "%.1f", valueNormalized * 10.0);
        } else {
            return kResultFalse;
        }
        strToChar16(buf, string);
        return kResultOk;
    }

    tresult PLUGIN_API getParamValueByString(ParamID id, TChar* string, ParamValue& valueNormalized) override {
        return kResultFalse;
    }

    ParamValue PLUGIN_API normalizedParamToPlain(ParamID id, ParamValue valueNormalized) override {
        return valueNormalized;
    }

    ParamValue PLUGIN_API plainParamToNormalized(ParamID id, ParamValue plainValue) override {
        return plainValue;
    }

    ParamValue PLUGIN_API getParamNormalized(ParamID id) override {
        switch (id) {
            case kParamBypass: return bypass_ ? 1.0 : 0.0;
            case kParamSelection: return static_cast<double>(selection_) / 128.0;
            case kParamLatencyProfile: return static_cast<double>(profile_) / 3.0;
            case kParamInputGain: return inputGain_;
            case kParamWetDryMix: return wetDryMix_;
            case kParamDrive: return drive_;
            case kParamOutputGain: return outputGain_;
            default: return 0.0;
        }
    }

    tresult PLUGIN_API setParamNormalized(ParamID id, ParamValue value) override {
        applyParameter(id, value);
        return kResultOk;
    }

    NativePlugin* rust() const { return rust_; }
    bool bypass() const { return bypass_; }
    int profile() const { return profile_; }
    uint32_t selection() const { return static_cast<uint32_t>(selection_); }
    void setProfile(int p) { applyParameterFromUI(kParamLatencyProfile, static_cast<double>(p) / 3.0); }
    void applyParameterFromUI(ParamID id, ParamValue val) {
        applyParameter(id, val);
        if (componentHandler_) {
            componentHandler_->beginEdit(id);
            componentHandler_->performEdit(id, val);
            componentHandler_->endEdit(id);
        }
    }

    IPlugView* PLUGIN_API createView(FIDString name) override;

private:
    void applyParameter(ParamID id, ParamValue val) {
        val = std::clamp(val, 0.0, 1.0);
        if (id == kParamBypass) {
            bypass_ = (val >= 0.5);
            if (rust_) og_native_set_param(rust_, kParamBypass, static_cast<float>(val));
        } else if (id == kParamSelection) {
            selection_ = static_cast<int>(val * 128.0 + 0.5);
            if (rust_) og_native_set_param(rust_, kParamSelection, static_cast<float>(val));
        } else if (id == kParamLatencyProfile) {
            int newProfile = std::clamp(static_cast<int>(val * 3.0 + 0.5), 0, 3);
            profile_ = newProfile;
            uint32_t targetSamples = targetLatencyFromProfile(profile_);
            lastLatency_ = targetSamples;
            if (rust_) {
                og_native_set_param(rust_, kParamLatencyProfile, static_cast<float>(val));
                og_native_set_transport_latency(rust_, targetSamples);
                og_native_accept_latency(rust_, og_native_latency(rust_));
            }
            if (componentHandler_) {
                componentHandler_->restartComponent(kLatencyChanged);
            }
        } else if (id == kParamInputGain) {
            inputGain_ = val;
            if (rust_) og_native_set_param(rust_, kParamInputGain, static_cast<float>(val));
        } else if (id == kParamWetDryMix) {
            wetDryMix_ = val;
            if (rust_) og_native_set_param(rust_, kParamWetDryMix, static_cast<float>(val));
        } else if (id == kParamDrive) {
            drive_ = val;
            if (rust_) og_native_set_param(rust_, kParamDrive, static_cast<float>(val));
        } else if (id == kParamOutputGain) {
            outputGain_ = val;
            if (rust_) og_native_set_param(rust_, kParamOutputGain, static_cast<float>(val));
        }
    }

    void createRust() {
        if (!rust_) {
            lastLatency_ = targetLatencyFromProfile(profile_);
            rust_ = og_native_create(sampleRate_, 8192, 2, lastLatency_);
            if (rust_) {
                og_native_bypass(rust_, bypass_);
                og_native_select(rust_, static_cast<uint32_t>(selection_));
                og_native_set_param(rust_, kParamInputGain, static_cast<float>(inputGain_));
                og_native_set_param(rust_, kParamWetDryMix, static_cast<float>(wetDryMix_));
                og_native_set_param(rust_, kParamDrive, static_cast<float>(drive_));
                og_native_set_param(rust_, kParamOutputGain, static_cast<float>(outputGain_));
                og_native_accept_latency(rust_, lastLatency_);
            }
        }
    }

    void cleanupRust() {
        if (rust_) {
            og_native_destroy(rust_);
            rust_ = nullptr;
        }
    }

    void recreateRust() {
        cleanupRust();
        createRust();
    }

    static void strToChar16(const char* src, TChar* dst) {
        while (*src) {
            *dst++ = static_cast<TChar>(*src++);
        }
        *dst = 0;
    }

    std::atomic<uint32> refCount_;
    NativePlugin* rust_;
    FUnknown* hostContext_;
    IComponentHandler* componentHandler_;
    double sampleRate_;
    uint32_t maxBlockSize_;
    bool bypass_;
    int selection_;
    int profile_;
    double inputGain_;
    double wetDryMix_;
    double drive_;
    double outputGain_;
    bool active_;
    uint32_t lastLatency_;
};

// --- GoodLookin VST3 Editor View Adapter ---
class OpenGridderPlugView : public IPlugView {
public:
    OpenGridderPlugView(OpenGridderVST3* controller) : controller_(controller), view_(nil) {}
    virtual ~OpenGridderPlugView() { removed(); }

    tresult PLUGIN_API queryInterface(const TUID _iid, void** obj) override {
        QUERY_INTERFACE(_iid, obj, IPlugView::iid, IPlugView)
        QUERY_INTERFACE(_iid, obj, FUnknown::iid, FUnknown)
        *obj = nullptr;
        return kNoInterface;
    }
    uint32 PLUGIN_API addRef() override { return ++refCount_; }
    uint32 PLUGIN_API release() override {
        uint32 r = --refCount_;
        if (r == 0) { delete this; return 0; }
        return r;
    }

    tresult PLUGIN_API isPlatformTypeSupported(FIDString type) override {
        if (type && std::strcmp(type, kPlatformTypeNSView) == 0) return kResultTrue;
        return kResultFalse;
    }

    tresult PLUGIN_API attached(void* parent, FIDString type) override {
        if (!type || std::strcmp(type, kPlatformTypeNSView) != 0 || !parent) return kResultFalse;
        NSView* parentView = (__bridge NSView*)parent;
        NSRect frame = NSMakeRect(0, 0, 864, 420);
        view_ = [[OpenGridderEditorView alloc] initWithFrame:frame];
        OpenGridderVST3* c = controller_;
        view_.rustProvider = ^NativePlugin* {
            return c ? c->rust() : nullptr;
        };
        view_.rust = controller_->rust();
        view_.isBypassed = controller_->bypass();
        view_.activeProfile = controller_->profile();
        view_.activeSlot = controller_->selection();

        view_.onProfileChanged = ^(int newProfile) {
            c->setProfile(newProfile);
        };
        view_.onParameterChanged = ^(uint32_t paramId, float val) {
            c->applyParameterFromUI(paramId, val);
        };
        OpenGridderPlugView* selfView = this;
        view_.onSizeChanged = ^(CGFloat newW, CGFloat newH) {
            if (selfView->plugFrame_) {
                ViewRect vr{0, 0, static_cast<int32>(newW), static_cast<int32>(newH)};
                selfView->plugFrame_->resizeView(selfView, &vr);
            }
        };

        [parentView addSubview:view_];
        [view_ setNeedsDisplay:YES];
        return kResultOk;
    }

    tresult PLUGIN_API removed() override {
        if (view_) {
            [view_ removeFromSuperview];
            view_ = nil;
        }
        return kResultOk;
    }

    tresult PLUGIN_API onWheel(float distance) override { return kResultOk; }
    tresult PLUGIN_API onKeyDown(char16 key, int16 keyCode, int16 modifiers) override { return kResultFalse; }
    tresult PLUGIN_API onKeyUp(char16 key, int16 keyCode, int16 modifiers) override { return kResultFalse; }
    tresult PLUGIN_API getSize(ViewRect* size) override {
        if (!size) return kInvalidArgument;
        NSSize sz = view_ ? [view_ preferredViewSize] : NSMakeSize(864, 420);
        size->left = 0; size->top = 0;
        size->right = static_cast<int32>(sz.width);
        size->bottom = static_cast<int32>(sz.height);
        return kResultOk;
    }
    tresult PLUGIN_API onSize(ViewRect* newSize) override { return kResultOk; }
    tresult PLUGIN_API onFocus(TBool state) override { return kResultOk; }
    tresult PLUGIN_API setFrame(IPlugFrame* frame) override { plugFrame_ = frame; return kResultOk; }
    tresult PLUGIN_API canResize() override { return kResultTrue; }
    tresult PLUGIN_API checkSizeConstraint(ViewRect* rect) override { return kResultOk; }

private:
    std::atomic<uint32> refCount_{1};
    OpenGridderVST3* controller_;
    IPlugFrame* plugFrame_{nullptr};
    OpenGridderEditorView* view_{nil};
};

IPlugView* PLUGIN_API OpenGridderVST3::createView(FIDString name) {
    if (!name || std::strcmp(name, "editor") == 0) {
        return new OpenGridderPlugView(this);
    }
    return nullptr;
}

// --- Plugin Factory ---
class OpenGridderFactory : public IPluginFactory2 {
public:
    tresult PLUGIN_API queryInterface(const TUID _iid, void** obj) override {
        if (FUnknownPrivate::iidEqual(_iid, FUnknown::iid)) {
            addRef();
            *obj = static_cast<IPluginFactory*>(this);
            return kResultOk;
        }
        QUERY_INTERFACE(_iid, obj, IPluginFactory::iid, IPluginFactory)
        QUERY_INTERFACE(_iid, obj, IPluginFactory2::iid, IPluginFactory2)
        *obj = nullptr;
        return kNoInterface;
    }

    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }

    tresult PLUGIN_API getFactoryInfo(PFactoryInfo* info) override {
        strncpy(info->vendor, "OpenGrid Studio", sizeof(info->vendor));
        strncpy(info->url, "https://github.com/MistaMin/OpenGridder", sizeof(info->url));
        strncpy(info->email, "info@opengrid.studio", sizeof(info->email));
        info->flags = PFactoryInfo::kNoFlags;
        return kResultOk;
    }

    int32 PLUGIN_API countClasses() override {
        return 1;
    }

    tresult PLUGIN_API getClassInfo(int32 index, PClassInfo* info) override {
        if (index != 0) return kResultFalse;
        memcpy(info->cid, kOpenGridderProcessorUID, sizeof(TUID));
        info->cardinality = PClassInfo::kManyInstances;
        strncpy(info->category, kVstAudioEffectClass, sizeof(info->category));
        strncpy(info->name, "OpenGridder", sizeof(info->name));
        return kResultOk;
    }

    tresult PLUGIN_API getClassInfo2(int32 index, PClassInfo2* info) override {
        if (index != 0) return kResultFalse;
        memcpy(info->cid, kOpenGridderProcessorUID, sizeof(TUID));
        info->cardinality = PClassInfo::kManyInstances;
        strncpy(info->category, kVstAudioEffectClass, sizeof(info->category));
        strncpy(info->name, "OpenGridder", sizeof(info->name));
        strncpy(info->vendor, "OpenGrid Studio", sizeof(info->vendor));
        strncpy(info->version, "0.3.1-alpha", sizeof(info->version));
        strncpy(info->sdkVersion, "VST 3.7.0", sizeof(info->sdkVersion));
        strncpy(info->subCategories, "Fx|Network", sizeof(info->subCategories));
        info->classFlags = 0;
        return kResultOk;
    }

    tresult PLUGIN_API createInstance(FIDString cid, FIDString _iid, void** obj) override {
        if (!FUnknownPrivate::iidEqual(cid, kOpenGridderProcessorUID) &&
            !FUnknownPrivate::iidEqual(cid, kOpenGridderControllerUID)) {
            return kNoInterface;
        }

        OpenGridderVST3* plug = new OpenGridderVST3();
        tresult res = plug->queryInterface(_iid, obj);
        plug->release(); // Balance initial refCount of 1
        return res;
    }
};

static OpenGridderFactory gFactory;

} // namespace

extern "C" {

SMTG_EXPORT_SYMBOL IPluginFactory* PLUGIN_API GetPluginFactory() {
    return &gFactory;
}

} // extern "C"
