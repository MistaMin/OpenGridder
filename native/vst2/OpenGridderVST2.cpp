// OpenGridder VST 2.4 adapter for vintage Mac DAWs (Cubase 4/5/6/7, Nuendo, Wavelab).
#include "vst2_abi.h"
#include "opengridder.h"

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <cstdio>

namespace {

enum {
    kParamBypass = 0,
    kParamSelection = 1,
    kParamLatencyProfile = 2,
    kNumParams = 3
};

class OpenGridderVST2 {
public:
    explicit OpenGridderVST2(audioMasterCallback audioMaster)
        : audioMaster_(audioMaster),
          rust_(nullptr),
          sampleRate_(44100.0),
          blockSize_(1024),
          bypass_(false),
          selection_(0),
          profile_(0),
          active_(false)
    {
        std::memset(&effect_, 0, sizeof(AEffect));
        effect_.magic = kEffectMagic;
        effect_.dispatcher = Dispatcher;
        effect_.process = ProcessDeprecated;
        effect_.setParameter = SetParameter;
        effect_.getParameter = GetParameter;
        effect_.processReplacing = ProcessReplacing;
        effect_.processDoubleReplacing = nullptr;

        effect_.numPrograms = 1;
        effect_.numParams = kNumParams;
        effect_.numInputs = 2;
        effect_.numOutputs = 2;
        effect_.flags = effFlagsCanReplacing;
        effect_.initialDelay = targetLatencyFromProfile(profile_);
        effect_.uniqueID = 0x4F474678; // 'OGFx'
        effect_.version = 1210;
        effect_.object = this;

        createRust();
    }

    ~OpenGridderVST2() {
        cleanupRust();
    }

    AEffect* getAEffect() { return &effect_; }

    static intptr_t Dispatcher(AEffect* effect, int32_t opcode, int32_t index, intptr_t value, void* ptr, float opt) {
        if (!effect || !effect->object) return 0;
        return static_cast<OpenGridderVST2*>(effect->object)->dispatch(opcode, index, value, ptr, opt);
    }

    static void ProcessDeprecated(AEffect* effect, float** inputs, float** outputs, int32_t sampleFrames) {
        if (effect && effect->object) {
            static_cast<OpenGridderVST2*>(effect->object)->process(inputs, outputs, sampleFrames, false);
        }
    }

    static void ProcessReplacing(AEffect* effect, float** inputs, float** outputs, int32_t sampleFrames) {
        if (effect && effect->object) {
            static_cast<OpenGridderVST2*>(effect->object)->process(inputs, outputs, sampleFrames, true);
        }
    }

    static void SetParameter(AEffect* effect, int32_t index, float parameter) {
        if (effect && effect->object) {
            static_cast<OpenGridderVST2*>(effect->object)->setParam(index, parameter);
        }
    }

    static float GetParameter(AEffect* effect, int32_t index) {
        if (effect && effect->object) {
            return static_cast<OpenGridderVST2*>(effect->object)->getParam(index);
        }
        return 0.0f;
    }

private:
    static uint32_t targetLatencyFromProfile(int32_t profile) {
        if (profile == 1) return 512;
        if (profile == 2) return 384;
        if (profile == 3) return 256;
        return 1024;
    }

    void createRust() {
        cleanupRust();
        uint32_t target = targetLatencyFromProfile(profile_);
        rust_ = og_native_create(sampleRate_, blockSize_, 2, target);
        if (rust_) {
            og_native_bypass(rust_, bypass_);
            og_native_select(rust_, selection_);
        }
    }

    void cleanupRust() {
        if (rust_) {
            og_native_destroy(rust_);
            rust_ = nullptr;
        }
    }

    void setParam(int32_t index, float value) {
        switch (index) {
            case kParamBypass:
                bypass_ = (value >= 0.5f);
                if (rust_) og_native_bypass(rust_, bypass_);
                break;
            case kParamSelection:
                selection_ = static_cast<uint32_t>(std::max(0.0f, std::min(128.0f, value * 128.0f)));
                if (rust_) og_native_select(rust_, selection_);
                break;
            case kParamLatencyProfile: {
                profile_ = static_cast<int32_t>(std::max(0.0f, std::min(3.0f, std::round(value * 3.0f))));
                uint32_t target = targetLatencyFromProfile(profile_);
                effect_.initialDelay = target;
                if (rust_) {
                    og_native_set_transport_latency(rust_, target);
                }
                // Notify host that delay compensation changed (PDC)
                if (audioMaster_) {
                    audioMaster_(&effect_, audioMasterIOChanged, 0, 0, nullptr, 0.0f);
                }
                break;
            }
            default:
                break;
        }
    }

    float getParam(int32_t index) const {
        switch (index) {
            case kParamBypass:
                return bypass_ ? 1.0f : 0.0f;
            case kParamSelection:
                return static_cast<float>(selection_) / 128.0f;
            case kParamLatencyProfile:
                return static_cast<float>(profile_) / 3.0f;
            default:
                return 0.0f;
        }
    }

    void process(float** inputs, float** outputs, int32_t sampleFrames, bool replacing) {
        if (sampleFrames <= 0 || !outputs) return;

        if (bypass_ || !rust_ || !active_) {
            if (replacing) {
                for (int ch = 0; ch < 2; ++ch) {
                    if (outputs[ch]) {
                        if (inputs && inputs[ch]) {
                            std::memcpy(outputs[ch], inputs[ch], sampleFrames * sizeof(float));
                        } else {
                            std::fill_n(outputs[ch], sampleFrames, 0.0f);
                        }
                    }
                }
            }
            return;
        }

        const float* inPtrs[2] = {
            (inputs && inputs[0]) ? inputs[0] : nullptr,
            (inputs && inputs[1]) ? inputs[1] : ((inputs && inputs[0]) ? inputs[0] : nullptr)
        };

        float* outPtrs[2] = { outputs[0], outputs[1] };

        int32_t res = og_native_process(rust_, inPtrs, outPtrs, sampleFrames);
        if (res != 0) {
            for (int ch = 0; ch < 2; ++ch) {
                if (outputs[ch]) std::fill_n(outputs[ch], sampleFrames, 0.0f);
            }
        }
    }

    intptr_t dispatch(int32_t opcode, int32_t index, intptr_t value, void* ptr, float opt) {
        switch (opcode) {
            case effOpen:
                return 0;
            case effClose:
                delete this;
                return 0;
            case effSetSampleRate:
                if (opt > 0.0f) {
                    sampleRate_ = opt;
                    createRust();
                }
                return 0;
            case effSetBlockSize:
                if (value > 0) {
                    blockSize_ = static_cast<uint32_t>(value);
                    createRust();
                }
                return 0;
            case effMainsChanged:
                active_ = (value != 0);
                return 0;
            case effGetParamName:
                if (ptr) {
                    char* str = static_cast<char*>(ptr);
                    if (index == kParamBypass) std::strncpy(str, "Bypass", 31);
                    else if (index == kParamSelection) std::strncpy(str, "Remote", 31);
                    else if (index == kParamLatencyProfile) std::strncpy(str, "Latency", 31);
                    else str[0] = 0;
                    str[31] = 0;
                }
                return 0;
            case effGetParamLabel:
                if (ptr) {
                    char* str = static_cast<char*>(ptr);
                    if (index == kParamLatencyProfile) std::strncpy(str, "profile", 31);
                    else str[0] = 0;
                    str[31] = 0;
                }
                return 0;
            case effGetParamDisplay:
                if (ptr) {
                    char* str = static_cast<char*>(ptr);
                    if (index == kParamBypass) {
                        std::snprintf(str, 32, "%s", bypass_ ? "On" : "Off");
                    } else if (index == kParamSelection) {
                        char name[256] = {};
                        if (rust_) og_native_name(rust_, selection_, name, sizeof(name));
                        std::snprintf(str, 32, "%u: %s", selection_, name);
                    } else if (index == kParamLatencyProfile) {
                        const char* names[] = {"1024s LAN", "512s 10GbE", "384s VSTi", "256s Direct"};
                        std::snprintf(str, 32, "%s", names[profile_ % 4]);
                    } else {
                        str[0] = 0;
                    }
                    str[31] = 0;
                }
                return 0;
            case effGetEffectName:
                if (ptr) {
                    std::strncpy(static_cast<char*>(ptr), "OpenGridder", 31);
                    static_cast<char*>(ptr)[31] = 0;
                }
                return 0;
            case effGetVendorString:
                if (ptr) {
                    std::strncpy(static_cast<char*>(ptr), "OpenGrid Studio", 31);
                    static_cast<char*>(ptr)[31] = 0;
                }
                return 0;
            case effGetProductString:
                if (ptr) {
                    std::strncpy(static_cast<char*>(ptr), "OpenGridder VST2", 31);
                    static_cast<char*>(ptr)[31] = 0;
                }
                return 0;
            case effGetVendorVersion:
                return 1210;
            case effGetVstVersion:
                return 2400;
            case effCanDo:
                if (ptr) {
                    const char* text = static_cast<const char*>(ptr);
                    if (!std::strcmp(text, "1in1out") || !std::strcmp(text, "2in2out") ||
                        !std::strcmp(text, "receiveVstTimeInfo") || !std::strcmp(text, "bypass")) {
                        return 1;
                    }
                }
                return 0;
            case effGetTailSize:
                return 0;
            case effSetBypass:
                bypass_ = (value != 0);
                if (rust_) og_native_bypass(rust_, bypass_);
                return 0;
            default:
                return 0;
        }
    }

    audioMasterCallback audioMaster_;
    AEffect effect_;
    NativePlugin* rust_;
    double sampleRate_;
    uint32_t blockSize_;
    bool bypass_;
    uint32_t selection_;
    int32_t profile_;
    bool active_;
};

} // anonymous namespace

// -----------------------------------------------------------------------------
// VST 2.4 Exported Entry Points
// -----------------------------------------------------------------------------
extern "C" {

#if defined(__GNUC__)
#define VST_EXPORT __attribute__((visibility("default")))
#else
#define VST_EXPORT
#endif

VST_EXPORT AEffect* VSTPluginMain(audioMasterCallback audioMaster) {
    if (!audioMaster) return nullptr;
    auto* plugin = new OpenGridderVST2(audioMaster);
    return plugin->getAEffect();
}

// Legacy Mach-O entry point alias for older hosts
VST_EXPORT AEffect* main_macho(audioMasterCallback audioMaster) {
    return VSTPluginMain(audioMaster);
}

VST_EXPORT AEffect* main_plugin(audioMasterCallback audioMaster) asm("main");
VST_EXPORT AEffect* main_plugin(audioMasterCallback audioMaster) {
    return VSTPluginMain(audioMaster);
}

} // extern "C"
