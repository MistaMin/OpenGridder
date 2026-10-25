// OpenGridder AUv2 adapter for Logic 8/9 and older macOS machines.
// Dual entry points: Component Manager (OpenGridderAUEntry) and AudioComponent (OpenGridderAUFactory).
#include "opengridder.h"

#include <CoreFoundation/CoreFoundation.h>
#include <CoreAudio/CoreAudioTypes.h>
#include <AudioUnit/AudioUnit.h>
#include <AudioUnit/AudioUnitProperties.h>
#include <AudioUnit/AudioUnitParameters.h>

#if defined(__i386__) || defined(__PPC__)
#include <CoreServices/CoreServices.h>
#endif

#ifndef noErr
#define noErr 0
#endif
#ifndef paramErr
#define paramErr (-50)
#endif
#ifndef badComponentSelector
#define badComponentSelector (-32767)
#endif
#ifndef badComponentInstance
#define badComponentInstance (-32766)
#endif

typedef OSStatus ComponentResult;

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <vector>

namespace {

enum {
    kParamBypass = 0,
    kParamSelection = 1,
    kParamLatencyProfile = 2,
    kNumParams = 3
};

struct PropertyListener {
    AudioUnitPropertyID id;
    AudioUnitPropertyListenerProc proc;
    void* userData;
};

class OpenGridderAUInstance {
public:
    explicit OpenGridderAUInstance(void* compInstance = nullptr)
        : compInstance_(compInstance),
          rust_(nullptr),
          sampleRate_(44100.0),
          maxFrames_(1024),
          channels_(2),
          bypass_(false),
          selection_(0),
          profile_(0),
          initialized_(false)
    {
        std::memset(&inputCallback_, 0, sizeof(inputCallback_));
        std::memset(&format_, 0, sizeof(format_));
        format_.mSampleRate = sampleRate_;
        format_.mFormatID = kAudioFormatLinearPCM;
        format_.mFormatFlags = kAudioFormatFlagsNativeFloatPacked | kAudioFormatFlagIsNonInterleaved;
        format_.mBytesPerPacket = sizeof(float);
        format_.mFramesPerPacket = 1;
        format_.mBytesPerFrame = sizeof(float);
        format_.mChannelsPerFrame = channels_;
        format_.mBitsPerChannel = 32;

        scratchBuffers_[0].resize(8192, 0.0f);
        scratchBuffers_[1].resize(8192, 0.0f);
    }

    ~OpenGridderAUInstance() {
        cleanupRust();
    }

    ComponentResult Initialize() {
        if (initialized_) return noErr;
        createRust();
        initialized_ = (rust_ != nullptr);
        return initialized_ ? noErr : -1;
    }

    ComponentResult Uninitialize() {
        cleanupRust();
        initialized_ = false;
        return noErr;
    }

    ComponentResult Reset() {
        return noErr;
    }

    ComponentResult Render(AudioUnitRenderActionFlags* ioActionFlags,
                           const AudioTimeStamp* inTimeStamp,
                           UInt32 inOutputBusNumber,
                           UInt32 inNumberFrames,
                           AudioBufferList* ioData)
    {
        if (!ioData || inNumberFrames == 0 || inOutputBusNumber != 0) return paramErr;
        if (!initialized_ || !rust_) return noErr;

        if (inNumberFrames > scratchBuffers_[0].size()) {
            scratchBuffers_[0].resize(inNumberFrames, 0.0f);
            scratchBuffers_[1].resize(inNumberFrames, 0.0f);
        }

        // Pull input audio from host callback
        AudioBufferList inputBufferList;
        inputBufferList.mNumberBuffers = channels_;
        for (UInt32 ch = 0; ch < channels_; ++ch) {
            inputBufferList.mBuffers[ch].mNumberChannels = 1;
            inputBufferList.mBuffers[ch].mDataByteSize = inNumberFrames * sizeof(float);
            inputBufferList.mBuffers[ch].mData = scratchBuffers_[ch].data();
        }

        if (inputCallback_.inputProc) {
            OSStatus err = inputCallback_.inputProc(inputCallback_.inputProcRefCon,
                                                   ioActionFlags,
                                                   inTimeStamp,
                                                   0,
                                                   inNumberFrames,
                                                   &inputBufferList);
            if (err != noErr) return err;
        } else {
            for (UInt32 ch = 0; ch < channels_; ++ch) {
                std::fill_n(scratchBuffers_[ch].data(), inNumberFrames, 0.0f);
            }
        }

        const float* inPtrs[2] = {
            scratchBuffers_[0].data(),
            channels_ > 1 ? scratchBuffers_[1].data() : scratchBuffers_[0].data()
        };

        float* outPtrs[2] = {
            static_cast<float*>(ioData->mBuffers[0].mData),
            channels_ > 1 ? static_cast<float*>(ioData->mBuffers[1].mData) : static_cast<float*>(ioData->mBuffers[0].mData)
        };

        if (bypass_) {
            for (UInt32 ch = 0; ch < ioData->mNumberBuffers; ++ch) {
                if (ioData->mBuffers[ch].mData && inPtrs[ch]) {
                    std::memcpy(ioData->mBuffers[ch].mData, inPtrs[ch], inNumberFrames * sizeof(float));
                }
            }
            return noErr;
        }

        int32_t res = og_native_process(rust_, inPtrs, outPtrs, inNumberFrames);
        if (res != 0) {
            for (UInt32 ch = 0; ch < ioData->mNumberBuffers; ++ch) {
                if (ioData->mBuffers[ch].mData) {
                    std::fill_n(static_cast<float*>(ioData->mBuffers[ch].mData), inNumberFrames, 0.0f);
                }
            }
        }

        return noErr;
    }

    ComponentResult GetPropertyInfo(AudioUnitPropertyID inID,
                                    AudioUnitScope inScope,
                                    AudioUnitElement inElement,
                                    UInt32* outDataSize,
                                    Boolean* outWritable)
    {
        if (outWritable) *outWritable = false;
        switch (inID) {
            case kAudioUnitProperty_ClassInfo:
                if (outDataSize) *outDataSize = sizeof(CFPropertyListRef);
                if (outWritable) *outWritable = true;
                return noErr;
            case kAudioUnitProperty_SampleRate:
                if (outDataSize) *outDataSize = sizeof(Float64);
                if (outWritable) *outWritable = true;
                return noErr;
            case kAudioUnitProperty_ParameterList:
                if (outDataSize) *outDataSize = kNumParams * sizeof(AudioUnitParameterID);
                return noErr;
            case kAudioUnitProperty_ParameterInfo:
                if (outDataSize) *outDataSize = sizeof(AudioUnitParameterInfo);
                return noErr;
            case kAudioUnitProperty_StreamFormat:
                if (outDataSize) *outDataSize = sizeof(AudioStreamBasicDescription);
                if (outWritable) *outWritable = true;
                return noErr;
            case kAudioUnitProperty_ElementCount:
                if (outDataSize) *outDataSize = sizeof(UInt32);
                return noErr;
            case kAudioUnitProperty_Latency:
                if (outDataSize) *outDataSize = sizeof(Float64);
                return noErr;
            case kAudioUnitProperty_SupportedNumChannels:
                if (outDataSize) *outDataSize = 2 * sizeof(AUChannelInfo);
                return noErr;
            case kAudioUnitProperty_MaximumFramesPerSlice:
                if (outDataSize) *outDataSize = sizeof(UInt32);
                if (outWritable) *outWritable = true;
                return noErr;
            case kAudioUnitProperty_SetRenderCallback:
                if (outDataSize) *outDataSize = sizeof(AURenderCallbackStruct);
                if (outWritable) *outWritable = true;
                return noErr;
            case kAudioUnitProperty_BypassEffect:
                if (outDataSize) *outDataSize = sizeof(UInt32);
                if (outWritable) *outWritable = true;
                return noErr;
            default:
                return kAudioUnitErr_InvalidProperty;
        }
    }

    ComponentResult GetProperty(AudioUnitPropertyID inID,
                                AudioUnitScope inScope,
                                AudioUnitElement inElement,
                                void* outData,
                                UInt32* ioDataSize)
    {
        if (!outData || !ioDataSize) return paramErr;
        switch (inID) {
            case kAudioUnitProperty_SampleRate:
                if (*ioDataSize < sizeof(Float64)) return kAudioUnitErr_InvalidPropertyValue;
                *static_cast<Float64*>(outData) = sampleRate_;
                *ioDataSize = sizeof(Float64);
                return noErr;
            case kAudioUnitProperty_StreamFormat:
                if (*ioDataSize < sizeof(AudioStreamBasicDescription)) return kAudioUnitErr_InvalidPropertyValue;
                *static_cast<AudioStreamBasicDescription*>(outData) = format_;
                *ioDataSize = sizeof(AudioStreamBasicDescription);
                return noErr;
            case kAudioUnitProperty_ElementCount:
                if (*ioDataSize < sizeof(UInt32)) return kAudioUnitErr_InvalidPropertyValue;
                *static_cast<UInt32*>(outData) = 1;
                *ioDataSize = sizeof(UInt32);
                return noErr;
            case kAudioUnitProperty_ParameterList: {
                if (*ioDataSize < kNumParams * sizeof(AudioUnitParameterID)) return kAudioUnitErr_InvalidPropertyValue;
                AudioUnitParameterID* params = static_cast<AudioUnitParameterID*>(outData);
                params[0] = kParamBypass;
                params[1] = kParamSelection;
                params[2] = kParamLatencyProfile;
                *ioDataSize = kNumParams * sizeof(AudioUnitParameterID);
                return noErr;
            }
            case kAudioUnitProperty_ParameterInfo: {
                if (*ioDataSize < sizeof(AudioUnitParameterInfo)) return kAudioUnitErr_InvalidPropertyValue;
                AudioUnitParameterInfo* info = static_cast<AudioUnitParameterInfo*>(outData);
                std::memset(info, 0, sizeof(AudioUnitParameterInfo));
                if (inElement == kParamBypass) {
                    std::strncpy(info->name, "Master Bypass", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Boolean;
                    info->minValue = 0.0f;
                    info->maxValue = 1.0f;
                    info->defaultValue = 0.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable;
                } else if (inElement == kParamSelection) {
                    std::strncpy(info->name, "Remote Plugin", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Indexed;
                    info->minValue = 0.0f;
                    info->maxValue = 128.0f;
                    info->defaultValue = 0.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable;
                } else if (inElement == kParamLatencyProfile) {
                    std::strncpy(info->name, "Latency Profile", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Indexed;
                    info->minValue = 0.0f;
                    info->maxValue = 3.0f;
                    info->defaultValue = 0.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable;
                } else {
                    return kAudioUnitErr_InvalidParameter;
                }
                *ioDataSize = sizeof(AudioUnitParameterInfo);
                return noErr;
            }
            case kAudioUnitProperty_Latency: {
                if (*ioDataSize < sizeof(Float64)) return kAudioUnitErr_InvalidPropertyValue;
                uint32_t samples = rust_ ? og_native_latency(rust_) : targetLatencyFromProfile(profile_);
                *static_cast<Float64*>(outData) = static_cast<Float64>(samples) / sampleRate_;
                *ioDataSize = sizeof(Float64);
                return noErr;
            }
            case kAudioUnitProperty_SupportedNumChannels: {
                if (*ioDataSize < 2 * sizeof(AUChannelInfo)) return kAudioUnitErr_InvalidPropertyValue;
                AUChannelInfo* info = static_cast<AUChannelInfo*>(outData);
                info[0].inChannels = 1; info[0].outChannels = 1;
                info[1].inChannels = 2; info[1].outChannels = 2;
                *ioDataSize = 2 * sizeof(AUChannelInfo);
                return noErr;
            }
            case kAudioUnitProperty_MaximumFramesPerSlice:
                if (*ioDataSize < sizeof(UInt32)) return kAudioUnitErr_InvalidPropertyValue;
                *static_cast<UInt32*>(outData) = maxFrames_;
                *ioDataSize = sizeof(UInt32);
                return noErr;
            case kAudioUnitProperty_BypassEffect:
                if (*ioDataSize < sizeof(UInt32)) return kAudioUnitErr_InvalidPropertyValue;
                *static_cast<UInt32*>(outData) = bypass_ ? 1 : 0;
                *ioDataSize = sizeof(UInt32);
                return noErr;
            default:
                return kAudioUnitErr_InvalidProperty;
        }
    }

    ComponentResult SetProperty(AudioUnitPropertyID inID,
                                AudioUnitScope inScope,
                                AudioUnitElement inElement,
                                const void* inData,
                                UInt32 inDataSize)
    {
        if (!inData) return paramErr;
        switch (inID) {
            case kAudioUnitProperty_SampleRate:
                if (inDataSize < sizeof(Float64)) return kAudioUnitErr_InvalidPropertyValue;
                sampleRate_ = *static_cast<const Float64*>(inData);
                format_.mSampleRate = sampleRate_;
                if (initialized_) { createRust(); }
                return noErr;
            case kAudioUnitProperty_StreamFormat: {
                if (inDataSize < sizeof(AudioStreamBasicDescription)) return kAudioUnitErr_InvalidPropertyValue;
                const auto* asbd = static_cast<const AudioStreamBasicDescription*>(inData);
                if (asbd->mChannelsPerFrame < 1 || asbd->mChannelsPerFrame > 2) return kAudioUnitErr_FormatNotSupported;
                channels_ = asbd->mChannelsPerFrame;
                sampleRate_ = asbd->mSampleRate;
                format_ = *asbd;
                if (initialized_) { createRust(); }
                return noErr;
            }
            case kAudioUnitProperty_MaximumFramesPerSlice:
                if (inDataSize < sizeof(UInt32)) return kAudioUnitErr_InvalidPropertyValue;
                maxFrames_ = *static_cast<const UInt32*>(inData);
                if (initialized_) { createRust(); }
                return noErr;
            case kAudioUnitProperty_SetRenderCallback:
                if (inDataSize < sizeof(AURenderCallbackStruct)) return kAudioUnitErr_InvalidPropertyValue;
                inputCallback_ = *static_cast<const AURenderCallbackStruct*>(inData);
                return noErr;
            case kAudioUnitProperty_BypassEffect:
                if (inDataSize < sizeof(UInt32)) return kAudioUnitErr_InvalidPropertyValue;
                bypass_ = (*static_cast<const UInt32*>(inData) != 0);
                if (rust_) og_native_bypass(rust_, bypass_);
                return noErr;
            default:
                return kAudioUnitErr_InvalidProperty;
        }
    }

    ComponentResult GetParameter(AudioUnitParameterID inID,
                                 AudioUnitScope inScope,
                                 AudioUnitElement inElement,
                                 AudioUnitParameterValue* outValue)
    {
        if (!outValue) return paramErr;
        switch (inID) {
            case kParamBypass:
                *outValue = bypass_ ? 1.0f : 0.0f;
                return noErr;
            case kParamSelection:
                *outValue = static_cast<AudioUnitParameterValue>(selection_);
                return noErr;
            case kParamLatencyProfile:
                *outValue = static_cast<AudioUnitParameterValue>(profile_);
                return noErr;
            default:
                return kAudioUnitErr_InvalidParameter;
        }
    }

    ComponentResult SetParameter(AudioUnitParameterID inID,
                                 AudioUnitScope inScope,
                                 AudioUnitElement inElement,
                                 AudioUnitParameterValue inValue,
                                 UInt32 inBufferOffsetInFrames)
    {
        switch (inID) {
            case kParamBypass:
                bypass_ = (inValue >= 0.5f);
                if (rust_) og_native_bypass(rust_, bypass_);
                return noErr;
            case kParamSelection:
                selection_ = static_cast<uint32_t>(std::max(0.0f, std::min(128.0f, inValue)));
                if (rust_) og_native_select(rust_, selection_);
                return noErr;
            case kParamLatencyProfile: {
                profile_ = static_cast<int32_t>(std::max(0.0f, std::min(3.0f, inValue)));
                uint32_t target = targetLatencyFromProfile(profile_);
                if (rust_) {
                    og_native_set_transport_latency(rust_, target);
                }
                notifyPropertyListeners(kAudioUnitProperty_Latency);
                return noErr;
            }
            default:
                return kAudioUnitErr_InvalidParameter;
        }
    }

    ComponentResult AddPropertyListener(AudioUnitPropertyID inID,
                                        AudioUnitPropertyListenerProc inProc,
                                        void* inUserData)
    {
        if (!inProc) return paramErr;
        PropertyListener listener;
        listener.id = inID;
        listener.proc = inProc;
        listener.userData = inUserData;
        listeners_.push_back(listener);
        return noErr;
    }

    ComponentResult RemovePropertyListener(AudioUnitPropertyID inID,
                                           AudioUnitPropertyListenerProc inProc)
    {
        listeners_.erase(
            std::remove_if(listeners_.begin(), listeners_.end(),
                           [=](const PropertyListener& l) { return l.id == inID && l.proc == inProc; }),
            listeners_.end());
        return noErr;
    }

    ComponentResult RemovePropertyListenerWithUserData(AudioUnitPropertyID inID,
                                                       AudioUnitPropertyListenerProc inProc,
                                                       void* inUserData)
    {
        listeners_.erase(
            std::remove_if(listeners_.begin(), listeners_.end(),
                           [=](const PropertyListener& l) {
                               return l.id == inID && l.proc == inProc && l.userData == inUserData;
                           }),
            listeners_.end());
        return noErr;
    }

#if defined(__i386__) || defined(__PPC__)
    ComponentResult DispatchComponent(ComponentParameters* params) {
        if (!params) return paramErr;
        switch (params->what) {
            case kAudioUnitInitializeSelect:
                return Initialize();
            case kAudioUnitUninitializeSelect:
                return Uninitialize();
            case kAudioUnitResetSelect:
                return Reset();
            case kAudioUnitGetPropertyInfoSelect:
                return GetPropertyInfo(static_cast<AudioUnitPropertyID>(params->params[0]),
                                       static_cast<AudioUnitScope>(params->params[1]),
                                       static_cast<AudioUnitElement>(params->params[2]),
                                       reinterpret_cast<UInt32*>(params->params[3]),
                                       reinterpret_cast<Boolean*>(params->params[4]));
            case kAudioUnitGetPropertySelect:
                return GetProperty(static_cast<AudioUnitPropertyID>(params->params[0]),
                                   static_cast<AudioUnitScope>(params->params[1]),
                                   static_cast<AudioUnitElement>(params->params[2]),
                                   reinterpret_cast<void*>(params->params[3]),
                                   reinterpret_cast<UInt32*>(params->params[4]));
            case kAudioUnitSetPropertySelect:
                return SetProperty(static_cast<AudioUnitPropertyID>(params->params[0]),
                                   static_cast<AudioUnitScope>(params->params[1]),
                                   static_cast<AudioUnitElement>(params->params[2]),
                                   reinterpret_cast<const void*>(params->params[3]),
                                   static_cast<UInt32>(params->params[4]));
            case kAudioUnitGetParameterSelect:
                return GetParameter(static_cast<AudioUnitParameterID>(params->params[0]),
                                    static_cast<AudioUnitScope>(params->params[1]),
                                    static_cast<AudioUnitElement>(params->params[2]),
                                    reinterpret_cast<AudioUnitParameterValue*>(params->params[3]));
            case kAudioUnitSetParameterSelect: {
                AudioUnitParameterValue val;
                std::memcpy(&val, &params->params[3], sizeof(AudioUnitParameterValue));
                return SetParameter(static_cast<AudioUnitParameterID>(params->params[0]),
                                    static_cast<AudioUnitScope>(params->params[1]),
                                    static_cast<AudioUnitElement>(params->params[2]),
                                    val,
                                    static_cast<UInt32>(params->params[4]));
            }
            case kAudioUnitRenderSelect:
                return Render(reinterpret_cast<AudioUnitRenderActionFlags*>(params->params[0]),
                              reinterpret_cast<const AudioTimeStamp*>(params->params[1]),
                              static_cast<UInt32>(params->params[2]),
                              static_cast<UInt32>(params->params[3]),
                              reinterpret_cast<AudioBufferList*>(params->params[4]));
            case kAudioUnitAddPropertyListenerSelect:
                return AddPropertyListener(static_cast<AudioUnitPropertyID>(params->params[0]),
                                           reinterpret_cast<AudioUnitPropertyListenerProc>(params->params[1]),
                                           reinterpret_cast<void*>(params->params[2]));
            case kAudioUnitRemovePropertyListenerSelect:
                return RemovePropertyListener(static_cast<AudioUnitPropertyID>(params->params[0]),
                                              reinterpret_cast<AudioUnitPropertyListenerProc>(params->params[1]));
            case kAudioUnitRemovePropertyListenerWithUserDataSelect:
                return RemovePropertyListenerWithUserData(static_cast<AudioUnitPropertyID>(params->params[0]),
                                                          reinterpret_cast<AudioUnitPropertyListenerProc>(params->params[1]),
                                                          reinterpret_cast<void*>(params->params[2]));
            default:
                return badComponentSelector;
        }
    }
#endif

private:
    static uint32_t targetLatencyFromProfile(int32_t profile) {
        if (profile == 1) return 512;
        if (profile == 2) return 384;
        if (profile == 3) return 256;
        return 1024;
    }

    void notifyPropertyListeners(AudioUnitPropertyID inID) {
        for (const auto& l : listeners_) {
            if (l.id == inID && l.proc) {
                l.proc(l.userData, static_cast<AudioUnit>(compInstance_), inID, kAudioUnitScope_Global, 0);
            }
        }
    }

    void createRust() {
        cleanupRust();
        uint32_t target = targetLatencyFromProfile(profile_);
        rust_ = og_native_create(sampleRate_, maxFrames_, channels_, target);
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

    void* compInstance_;
    NativePlugin* rust_;
    Float64 sampleRate_;
    UInt32 maxFrames_;
    UInt32 channels_;
    bool bypass_;
    uint32_t selection_;
    int32_t profile_;
    bool initialized_;
    AURenderCallbackStruct inputCallback_;
    AudioStreamBasicDescription format_;
    std::vector<float> scratchBuffers_[2];
    std::vector<PropertyListener> listeners_;
};

// C Wrappers for AudioComponent dispatch
struct AudioComponentPlugInInstanceWrapper {
    AudioComponentPlugInInterface mPlugInInterface;
    OpenGridderAUInstance* mInstance;
};

OSStatus AU_Open(void* self, AudioComponentInstance compInstance) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    wrapper->mInstance = new OpenGridderAUInstance(compInstance);
    return noErr;
}

OSStatus AU_Close(void* self) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    delete wrapper->mInstance;
    wrapper->mInstance = nullptr;
    std::free(wrapper);
    return noErr;
}

OSStatus AU_Initialize(void* self) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->Initialize() : static_cast<ComponentResult>(-1);
}

OSStatus AU_Uninitialize(void* self) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->Uninitialize() : static_cast<ComponentResult>(-1);
}

OSStatus AU_Reset(void* self, AudioUnitScope scope, AudioUnitElement elem) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->Reset() : static_cast<ComponentResult>(-1);
}

OSStatus AU_GetPropertyInfo(void* self, AudioUnitPropertyID prop, AudioUnitScope scope, AudioUnitElement elem, UInt32* outSize, Boolean* outWritable) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->GetPropertyInfo(prop, scope, elem, outSize, outWritable) : static_cast<ComponentResult>(-1);
}

OSStatus AU_GetProperty(void* self, AudioUnitPropertyID prop, AudioUnitScope scope, AudioUnitElement elem, void* outData, UInt32* ioSize) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->GetProperty(prop, scope, elem, outData, ioSize) : static_cast<ComponentResult>(-1);
}

OSStatus AU_SetProperty(void* self, AudioUnitPropertyID prop, AudioUnitScope scope, AudioUnitElement elem, const void* inData, UInt32 inSize) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->SetProperty(prop, scope, elem, inData, inSize) : static_cast<ComponentResult>(-1);
}

OSStatus AU_GetParameter(void* self, AudioUnitParameterID param, AudioUnitScope scope, AudioUnitElement elem, AudioUnitParameterValue* outValue) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->GetParameter(param, scope, elem, outValue) : static_cast<ComponentResult>(-1);
}

OSStatus AU_SetParameter(void* self, AudioUnitParameterID param, AudioUnitScope scope, AudioUnitElement elem, AudioUnitParameterValue inValue, UInt32 offset) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->SetParameter(param, scope, elem, inValue, offset) : static_cast<ComponentResult>(-1);
}

OSStatus AU_Render(void* self, AudioUnitRenderActionFlags* ioFlags, const AudioTimeStamp* inTimeStamp, UInt32 inOutputBusNumber, UInt32 inNumberFrames, AudioBufferList* ioData) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->Render(ioFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData) : static_cast<ComponentResult>(-1);
}

OSStatus AU_AddPropertyListener(void* self, AudioUnitPropertyID prop, AudioUnitPropertyListenerProc proc, void* userData) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->AddPropertyListener(prop, proc, userData) : static_cast<ComponentResult>(-1);
}

OSStatus AU_RemovePropertyListener(void* self, AudioUnitPropertyID prop, AudioUnitPropertyListenerProc proc) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->RemovePropertyListener(prop, proc) : static_cast<ComponentResult>(-1);
}

OSStatus AU_RemovePropertyListenerWithUserData(void* self, AudioUnitPropertyID prop, AudioUnitPropertyListenerProc proc, void* userData) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->RemovePropertyListenerWithUserData(prop, proc, userData) : static_cast<ComponentResult>(-1);
}

AudioComponentMethod AU_Lookup(SInt16 selector) {
    switch (selector) {
        case kAudioUnitInitializeSelect: return reinterpret_cast<AudioComponentMethod>(AU_Initialize);
        case kAudioUnitUninitializeSelect: return reinterpret_cast<AudioComponentMethod>(AU_Uninitialize);
        case kAudioUnitResetSelect: return reinterpret_cast<AudioComponentMethod>(AU_Reset);
        case kAudioUnitGetPropertyInfoSelect: return reinterpret_cast<AudioComponentMethod>(AU_GetPropertyInfo);
        case kAudioUnitGetPropertySelect: return reinterpret_cast<AudioComponentMethod>(AU_GetProperty);
        case kAudioUnitSetPropertySelect: return reinterpret_cast<AudioComponentMethod>(AU_SetProperty);
        case kAudioUnitGetParameterSelect: return reinterpret_cast<AudioComponentMethod>(AU_GetParameter);
        case kAudioUnitSetParameterSelect: return reinterpret_cast<AudioComponentMethod>(AU_SetParameter);
        case kAudioUnitRenderSelect: return reinterpret_cast<AudioComponentMethod>(AU_Render);
        case kAudioUnitAddPropertyListenerSelect: return reinterpret_cast<AudioComponentMethod>(AU_AddPropertyListener);
        case kAudioUnitRemovePropertyListenerSelect: return reinterpret_cast<AudioComponentMethod>(AU_RemovePropertyListener);
        case kAudioUnitRemovePropertyListenerWithUserDataSelect: return reinterpret_cast<AudioComponentMethod>(AU_RemovePropertyListenerWithUserData);
        default: return nullptr;
    }
}

} // anonymous namespace

// -----------------------------------------------------------------------------
// Logic 8 & 32-bit Logic 9 Component Manager Entry Point
// -----------------------------------------------------------------------------
#if defined(__i386__) || defined(__PPC__)
extern "C" __attribute__((visibility("default")))
ComponentResult OpenGridderAUEntry(ComponentParameters* params, void* self) {
    if (!params) return paramErr;
    if (params->what == -1) { // kComponentOpenSelect
        ComponentInstance ci = reinterpret_cast<ComponentInstance>(params->params[0]);
        OpenGridderAUInstance* inst = new OpenGridderAUInstance(ci);
        SetComponentInstanceStorage(ci, reinterpret_cast<Handle>(inst));
        return noErr;
    }
    if (params->what == -2) { // kComponentCloseSelect
        OpenGridderAUInstance* inst = reinterpret_cast<OpenGridderAUInstance*>(self);
        delete inst;
        return noErr;
    }
    if (params->what == -4) { // kComponentVersionSelect
        return 0x00010201;
    }
    if (params->what == -3) { // kComponentCanDoSelect
        SInt16 sel = static_cast<SInt16>(params->params[0]);
        return (sel == -1 || sel == -2 || sel == -3 || sel == -4 ||
                sel == kAudioUnitInitializeSelect ||
                sel == kAudioUnitUninitializeSelect ||
                sel == kAudioUnitResetSelect ||
                sel == kAudioUnitGetPropertyInfoSelect ||
                sel == kAudioUnitGetPropertySelect ||
                sel == kAudioUnitSetPropertySelect ||
                sel == kAudioUnitGetParameterSelect ||
                sel == kAudioUnitSetParameterSelect ||
                sel == kAudioUnitRenderSelect ||
                sel == kAudioUnitAddPropertyListenerSelect ||
                sel == kAudioUnitRemovePropertyListenerSelect ||
                sel == kAudioUnitRemovePropertyListenerWithUserDataSelect) ? 1 : 0;
    }
    OpenGridderAUInstance* inst = reinterpret_cast<OpenGridderAUInstance*>(self);
    if (!inst) return badComponentInstance;
    return inst->DispatchComponent(params);
}
#endif

// -----------------------------------------------------------------------------
// 64-bit Logic 9 & Modern AudioComponent Entry Point
// -----------------------------------------------------------------------------
extern "C" __attribute__((visibility("default")))
AudioComponentPlugInInterface* OpenGridderAUFactory(const AudioComponentDescription* /* inDesc */) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(std::malloc(sizeof(AudioComponentPlugInInstanceWrapper)));
    if (!wrapper) return nullptr;
    wrapper->mPlugInInterface.Open = AU_Open;
    wrapper->mPlugInInterface.Close = AU_Close;
    wrapper->mPlugInInterface.Lookup = AU_Lookup;
    wrapper->mPlugInInterface.reserved = nullptr;
    wrapper->mInstance = nullptr;
    return &wrapper->mPlugInInterface;
}
