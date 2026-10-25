// OpenGridder AUv2 adapter for Logic 8/9 and older macOS machines.
// Dual entry points: Component Manager (OpenGridderAUEntry) and AudioComponent (OpenGridderAUFactory).
#include "opengridder.h"

#include <CoreFoundation/CoreFoundation.h>
#include <CoreAudio/CoreAudioTypes.h>
#include <AudioUnit/AudioUnit.h>
#include <AudioUnit/AudioUnitProperties.h>
#include <AudioUnit/AudioUnitParameters.h>
#include <dlfcn.h>

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

static void* AU_Anchor() { return (void*)&AU_Anchor; }

enum {
    kParamBypass = 0,
    kParamSelection = 1,
    kParamLatencyProfile = 2,
    kParamInputGain = 3,
    kParamWetDryMix = 4,
    kParamDrive = 5,
    kParamOutputGain = 6,
    kNumParams = 7
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
          sampleRate_(48000.0),
          maxFrames_(1024),
          channels_(2),
          bypass_(false),
          selection_(0),
          profile_(1),
          inputGain_(0.0f),
          wetDryMix_(100.0f),
          drive_(0.0f),
          outputGain_(0.0f),
          lastLatency_(256),
          initialized_(false),
          hasConnection_(false)
    {
        std::memset(&connection_, 0, sizeof(connection_));
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

        AudioUnitRenderActionFlags preFlags = kAudioUnitRenderAction_PreRender;
        for (const auto& n : renderNotifies_) {
            if (n.proc) n.proc(n.refCon, &preFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData);
        }

        // Pull input audio from host callback
        AudioBufferList inputBufferList;
        inputBufferList.mNumberBuffers = channels_;
        for (UInt32 ch = 0; ch < channels_; ++ch) {
            inputBufferList.mBuffers[ch].mNumberChannels = 1;
            inputBufferList.mBuffers[ch].mDataByteSize = inNumberFrames * sizeof(float);
            inputBufferList.mBuffers[ch].mData = scratchBuffers_[ch].data();
        }

        if (hasConnection_ && connection_.sourceAudioUnit) {
            AudioUnitRenderActionFlags flags = 0;
            OSStatus err = AudioUnitRender(connection_.sourceAudioUnit, &flags, inTimeStamp, connection_.sourceOutputNumber, inNumberFrames, &inputBufferList);
            if (err != noErr) return err;
        } else if (inputCallback_.inputProc) {
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

        uint32_t curLat = og_native_latency(rust_);
        if (curLat != lastLatency_) {
            lastLatency_ = curLat;
            notifyPropertyListeners(kAudioUnitProperty_Latency);
        }
        og_native_accept_latency(rust_, lastLatency_);

        int32_t res = og_native_process(rust_, inPtrs, outPtrs, inNumberFrames);
        if (res != 0) {
            for (UInt32 ch = 0; ch < ioData->mNumberBuffers; ++ch) {
                if (ioData->mBuffers[ch].mData) {
                    std::fill_n(static_cast<float*>(ioData->mBuffers[ch].mData), inNumberFrames, 0.0f);
                }
            }
        }

        AudioUnitRenderActionFlags postFlags = kAudioUnitRenderAction_PostRender;
        for (const auto& n : renderNotifies_) {
            if (n.proc) n.proc(n.refCon, &postFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData);
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
            case kAudioUnitProperty_PresentPreset:
                if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
                if (outDataSize) *outDataSize = sizeof(AUPreset);
                if (outWritable) *outWritable = true;
                return noErr;
            case kAudioUnitProperty_SampleRate:
                if (outDataSize) *outDataSize = sizeof(Float64);
                if (outWritable) *outWritable = true;
                return noErr;
            case kAudioUnitProperty_ParameterList:
                if (inScope != kAudioUnitScope_Global) {
                    if (outDataSize) *outDataSize = 0;
                    return noErr;
                }
                if (outDataSize) *outDataSize = kNumParams * sizeof(AudioUnitParameterID);
                return noErr;
            case kAudioUnitProperty_ParameterInfo:
                if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
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
                if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
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
            case kAudioUnitProperty_MakeConnection:
                if (inScope != kAudioUnitScope_Input) return kAudioUnitErr_InvalidScope;
                if (outDataSize) *outDataSize = sizeof(AudioUnitConnection);
                if (outWritable) *outWritable = true;
                return noErr;
            case kAudioUnitProperty_BypassEffect:
                if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
                if (outDataSize) *outDataSize = sizeof(UInt32);
                if (outWritable) *outWritable = true;
                return noErr;
            case kAudioUnitProperty_CocoaUI:
                if (outDataSize) *outDataSize = sizeof(AudioUnitCocoaViewInfo);
                return noErr;
            case 65536: // kAudioUnitProperty_OpenGridderRust
                if (outDataSize) *outDataSize = sizeof(NativePlugin*);
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
            case kAudioUnitProperty_ClassInfo: {
                if (*ioDataSize < sizeof(CFPropertyListRef)) return kAudioUnitErr_InvalidPropertyValue;
                CFMutableDictionaryRef dict = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
                SInt32 version = 1;
                CFNumberRef verNum = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &version);
                CFDictionarySetValue(dict, CFSTR(kAUPresetVersionKey), verNum);
                CFRelease(verNum);

                UInt32 type = 'aufx';
                CFNumberRef typeNum = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &type);
                CFDictionarySetValue(dict, CFSTR(kAUPresetTypeKey), typeNum);
                CFRelease(typeNum);

                UInt32 subtype = 'OGFx';
                CFNumberRef subNum = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &subtype);
                CFDictionarySetValue(dict, CFSTR(kAUPresetSubtypeKey), subNum);
                CFRelease(subNum);

                UInt32 manuf = 'OpGr';
                CFNumberRef manufNum = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &manuf);
                CFDictionarySetValue(dict, CFSTR(kAUPresetManufacturerKey), manufNum);
                CFRelease(manufNum);

                CFDictionarySetValue(dict, CFSTR(kAUPresetNameKey), CFSTR("Default"));
                if (rust_) {
                    uint32_t stateLen = og_native_get_state(rust_, nullptr, 0);
                    if (stateLen > 0) {
                        std::vector<uint8_t> buf(stateLen);
                        uint32_t written = og_native_get_state(rust_, buf.data(), stateLen);
                        CFDataRef dataRef = CFDataCreate(kCFAllocatorDefault, buf.data(), written);
                        if (dataRef) {
                            CFDictionarySetValue(dict, CFSTR("OpenGridderState"), dataRef);
                            CFRelease(dataRef);
                        }
                    }
                }
                *static_cast<CFPropertyListRef*>(outData) = dict;
                *ioDataSize = sizeof(CFPropertyListRef);
                return noErr;
            }
            case kAudioUnitProperty_PresentPreset: {
                if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
                if (*ioDataSize < sizeof(AUPreset)) return kAudioUnitErr_InvalidPropertyValue;
                AUPreset* preset = static_cast<AUPreset*>(outData);
                preset->presetNumber = 0;
                preset->presetName = CFSTR("Default");
                *ioDataSize = sizeof(AUPreset);
                return noErr;
            }
            case kAudioUnitProperty_MakeConnection: {
                if (inScope != kAudioUnitScope_Input) return kAudioUnitErr_InvalidScope;
                if (*ioDataSize < sizeof(AudioUnitConnection)) return kAudioUnitErr_InvalidPropertyValue;
                *static_cast<AudioUnitConnection*>(outData) = connection_;
                *ioDataSize = sizeof(AudioUnitConnection);
                return noErr;
            }
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
                if (inScope != kAudioUnitScope_Global) {
                    *ioDataSize = 0;
                    return noErr;
                }
                if (*ioDataSize < kNumParams * sizeof(AudioUnitParameterID)) return kAudioUnitErr_InvalidPropertyValue;
                AudioUnitParameterID* params = static_cast<AudioUnitParameterID*>(outData);
                for (UInt32 i = 0; i < kNumParams; ++i) {
                    params[i] = i;
                }
                *ioDataSize = kNumParams * sizeof(AudioUnitParameterID);
                return noErr;
            }
            case kAudioUnitProperty_ParameterInfo: {
                if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
                if (*ioDataSize < sizeof(AudioUnitParameterInfo)) return kAudioUnitErr_InvalidPropertyValue;
                AudioUnitParameterInfo* info = static_cast<AudioUnitParameterInfo*>(outData);
                std::memset(info, 0, sizeof(AudioUnitParameterInfo));
                if (inElement == kParamBypass) {
                    std::strncpy(info->name, "Master Bypass", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Boolean;
                    info->minValue = 0.0f;
                    info->maxValue = 1.0f;
                    info->defaultValue = 0.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_HasCFNameString;
                    info->cfNameString = CFSTR("Master Bypass");
                } else if (inElement == kParamSelection) {
                    std::strncpy(info->name, "Remote Plugin", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Indexed;
                    info->minValue = 0.0f;
                    info->maxValue = 128.0f;
                    info->defaultValue = 0.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_HasCFNameString;
                    info->cfNameString = CFSTR("Remote Plugin");
                } else if (inElement == kParamLatencyProfile) {
                    std::strncpy(info->name, "Latency Profile", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Indexed;
                    info->minValue = 0.0f;
                    info->maxValue = 3.0f;
                    info->defaultValue = 1.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_HasCFNameString;
                    info->cfNameString = CFSTR("Latency Profile");
                } else if (inElement == kParamInputGain) {
                    std::strncpy(info->name, "Input Gain", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Decibels;
                    info->minValue = -24.0f;
                    info->maxValue = 24.0f;
                    info->defaultValue = 0.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_HasCFNameString;
                    info->cfNameString = CFSTR("Input Gain");
                } else if (inElement == kParamWetDryMix) {
                    std::strncpy(info->name, "Wet/Dry Mix", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Percent;
                    info->minValue = 0.0f;
                    info->maxValue = 100.0f;
                    info->defaultValue = 100.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_HasCFNameString;
                    info->cfNameString = CFSTR("Wet/Dry Mix");
                } else if (inElement == kParamDrive) {
                    std::strncpy(info->name, "Drive / Sat", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Generic;
                    info->minValue = 0.0f;
                    info->maxValue = 10.0f;
                    info->defaultValue = 0.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_HasCFNameString;
                    info->cfNameString = CFSTR("Drive / Sat");
                } else if (inElement == kParamOutputGain) {
                    std::strncpy(info->name, "Output Trim", sizeof(info->name));
                    info->unit = kAudioUnitParameterUnit_Decibels;
                    info->minValue = -24.0f;
                    info->maxValue = 24.0f;
                    info->defaultValue = 0.0f;
                    info->flags = kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_HasCFNameString;
                    info->cfNameString = CFSTR("Output Trim");
                } else {
                    return kAudioUnitErr_InvalidParameter;
                }
                *ioDataSize = sizeof(AudioUnitParameterInfo);
                return noErr;
            }
            case kAudioUnitProperty_Latency: {
                if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
                if (*ioDataSize < sizeof(Float64)) return kAudioUnitErr_InvalidPropertyValue;
                uint32_t samples = rust_ ? og_native_latency(rust_) : targetLatencyFromProfile(profile_);
                *static_cast<Float64*>(outData) = static_cast<Float64>(samples) / sampleRate_;
                *ioDataSize = sizeof(Float64);
                if (rust_) {
                    og_native_accept_latency(rust_, samples);
                }
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
                if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
                if (*ioDataSize < sizeof(UInt32)) return kAudioUnitErr_InvalidPropertyValue;
                *static_cast<UInt32*>(outData) = bypass_ ? 1 : 0;
                *ioDataSize = sizeof(UInt32);
                return noErr;
            case kAudioUnitProperty_CocoaUI: {
                if (*ioDataSize < sizeof(AudioUnitCocoaViewInfo)) return kAudioUnitErr_InvalidPropertyValue;
                AudioUnitCocoaViewInfo* viewInfo = static_cast<AudioUnitCocoaViewInfo*>(outData);
                CFBundleRef bundle = CFBundleGetBundleWithIdentifier(CFSTR("studio.opengrid.OpenGridder.component"));
                if (!bundle) bundle = CFBundleGetBundleWithIdentifier(CFSTR("com.opengridder.audiounit"));
                if (!bundle) {
                    Dl_info info;
                    if (dladdr((const void*)AU_Anchor, &info) && info.dli_fname) {
                        CFStringRef path = CFStringCreateWithCString(kCFAllocatorDefault, info.dli_fname, kCFStringEncodingUTF8);
                        if (path) {
                            CFURLRef url = CFURLCreateWithFileSystemPath(kCFAllocatorDefault, path, kCFURLPOSIXPathStyle, false);
                            if (url) {
                                CFURLRef contents = CFURLCreateCopyDeletingLastPathComponent(kCFAllocatorDefault, url);
                                if (contents) {
                                    CFURLRef bundleUrl = CFURLCreateCopyDeletingLastPathComponent(kCFAllocatorDefault, contents);
                                    if (bundleUrl) {
                                        bundle = CFBundleCreate(kCFAllocatorDefault, bundleUrl);
                                        CFRelease(bundleUrl);
                                    }
                                    CFRelease(contents);
                                }
                                CFRelease(url);
                            }
                            CFRelease(path);
                        }
                    }
                }
                if (!bundle) bundle = CFBundleGetMainBundle();
                viewInfo->mCocoaAUViewBundleLocation = bundle ? CFBundleCopyBundleURL(bundle) : NULL;
                viewInfo->mCocoaAUViewClass[0] = CFStringCreateCopy(kCFAllocatorDefault, CFSTR("OpenGridderAUCocoaViewFactory"));
                *ioDataSize = sizeof(AudioUnitCocoaViewInfo);
                return noErr;
            }
            case 65536: {
                if (*ioDataSize < sizeof(NativePlugin*)) return kAudioUnitErr_InvalidPropertyValue;
                *reinterpret_cast<NativePlugin**>(outData) = rust_;
                *ioDataSize = sizeof(NativePlugin*);
                return noErr;
            }
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
            case kAudioUnitProperty_ClassInfo: {
                if (inDataSize < sizeof(CFPropertyListRef)) return kAudioUnitErr_InvalidPropertyValue;
                CFPropertyListRef plist = *static_cast<const CFPropertyListRef*>(inData);
                if (plist && CFGetTypeID(plist) == CFDictionaryGetTypeID()) {
                    CFDictionaryRef dict = static_cast<CFDictionaryRef>(plist);
                    CFTypeRef stateVal = CFDictionaryGetValue(dict, CFSTR("OpenGridderState"));
                    if (stateVal && CFGetTypeID(stateVal) == CFDataGetTypeID()) {
                        CFDataRef dataRef = static_cast<CFDataRef>(stateVal);
                        const UInt8* ptr = CFDataGetBytePtr(dataRef);
                        CFIndex len = CFDataGetLength(dataRef);
                        if (ptr && len > 0) {
                            savedState_.assign(ptr, ptr + len);
                            if (rust_) {
                                og_native_set_state(rust_, savedState_.data(), static_cast<uint32_t>(savedState_.size()));
                                bypass_ = og_native_get_param(rust_, kParamBypass) >= 0.5f;
                                selection_ = static_cast<uint32_t>(og_native_get_param(rust_, kParamSelection) * 128.0f + 0.5f);
                                profile_ = static_cast<int32_t>(og_native_get_param(rust_, kParamLatencyProfile) * 3.0f + 0.5f);
                                inputGain_ = (og_native_get_param(rust_, kParamInputGain) - 0.5f) * 48.0f;
                                wetDryMix_ = og_native_get_param(rust_, kParamWetDryMix) * 100.0f;
                                drive_ = og_native_get_param(rust_, kParamDrive) * 10.0f;
                                outputGain_ = (og_native_get_param(rust_, kParamOutputGain) - 0.5f) * 48.0f;
                            }
                        }
                    }
                }
                return noErr;
            }
            case kAudioUnitProperty_PresentPreset: {
                if (inDataSize < sizeof(AUPreset)) return kAudioUnitErr_InvalidPropertyValue;
                return noErr;
            }
            case kAudioUnitProperty_MakeConnection: {
                if (inScope != kAudioUnitScope_Input) return kAudioUnitErr_InvalidScope;
                if (inDataSize < sizeof(AudioUnitConnection)) return kAudioUnitErr_InvalidPropertyValue;
                connection_ = *static_cast<const AudioUnitConnection*>(inData);
                hasConnection_ = true;
                return noErr;
            }
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
                notifyPropertyListeners(kAudioUnitProperty_MaximumFramesPerSlice);
                return noErr;
            case kAudioUnitProperty_SetRenderCallback:
                if (inDataSize < sizeof(AURenderCallbackStruct)) return kAudioUnitErr_InvalidPropertyValue;
                inputCallback_ = *static_cast<const AURenderCallbackStruct*>(inData);
                return noErr;
            case kAudioUnitProperty_BypassEffect:
                if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
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
        if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
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
            case kParamInputGain:
                *outValue = inputGain_;
                return noErr;
            case kParamWetDryMix:
                *outValue = wetDryMix_;
                return noErr;
            case kParamDrive:
                *outValue = drive_;
                return noErr;
            case kParamOutputGain:
                *outValue = outputGain_;
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
        if (inScope != kAudioUnitScope_Global) return kAudioUnitErr_InvalidScope;
        switch (inID) {
            case kParamBypass:
                bypass_ = (inValue >= 0.5f);
                if (rust_) og_native_set_param(rust_, kParamBypass, bypass_ ? 1.0f : 0.0f);
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
            case kParamInputGain:
                inputGain_ = std::max(-24.0f, std::min(24.0f, inValue));
                if (rust_) og_native_set_param(rust_, kParamInputGain, (inputGain_ + 24.0f) / 48.0f);
                return noErr;
            case kParamWetDryMix:
                wetDryMix_ = std::max(0.0f, std::min(100.0f, inValue));
                if (rust_) og_native_set_param(rust_, kParamWetDryMix, wetDryMix_ / 100.0f);
                return noErr;
            case kParamDrive:
                drive_ = std::max(0.0f, std::min(10.0f, inValue));
                if (rust_) og_native_set_param(rust_, kParamDrive, drive_ / 10.0f);
                return noErr;
            case kParamOutputGain:
                outputGain_ = std::max(-24.0f, std::min(24.0f, inValue));
                if (rust_) og_native_set_param(rust_, kParamOutputGain, (outputGain_ + 24.0f) / 48.0f);
                return noErr;
            default:
                return kAudioUnitErr_InvalidParameter;
        }
    }

    struct RenderNotify {
        AURenderCallback proc;
        void* refCon;
    };
    std::vector<RenderNotify> renderNotifies_;

    ComponentResult AddRenderNotify(AURenderCallback inProc, void* inProcRefCon) {
        if (!inProc) return paramErr;
        renderNotifies_.push_back({inProc, inProcRefCon});
        return noErr;
    }

    ComponentResult RemoveRenderNotify(AURenderCallback inProc, void* inProcRefCon) {
        renderNotifies_.erase(
            std::remove_if(renderNotifies_.begin(), renderNotifies_.end(),
                           [=](const RenderNotify& n) { return n.proc == inProc && n.refCon == inProcRefCon; }),
            renderNotifies_.end());
        return noErr;
    }

    ComponentResult ScheduleParameters(const AudioUnitParameterEvent* inParameterEvents, UInt32 inNumEvents) {
        if (!inParameterEvents) return paramErr;
        for (UInt32 i = 0; i < inNumEvents; ++i) {
            const auto& ev = inParameterEvents[i];
            if (ev.eventType == kParameterEvent_Immediate) {
                SetParameter(ev.parameter, ev.scope, ev.element, ev.eventValues.immediate.value, ev.eventValues.immediate.bufferOffset);
            }
        }
        return noErr;
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
            case kAudioUnitScheduleParametersSelect:
                return ScheduleParameters(reinterpret_cast<const AudioUnitParameterEvent*>(params->params[0]),
                                          static_cast<UInt32>(params->params[1]));
            case kAudioUnitAddRenderNotifySelect:
                return AddRenderNotify(reinterpret_cast<AURenderCallback>(params->params[0]),
                                       reinterpret_cast<void*>(params->params[1]));
            case kAudioUnitRemoveRenderNotifySelect:
                return RemoveRenderNotify(reinterpret_cast<AURenderCallback>(params->params[0]),
                                          reinterpret_cast<void*>(params->params[1]));
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
        if (profile == 0) return 128;
        if (profile == 1) return 256;
        if (profile == 2) return 512;
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
        lastLatency_ = target;
        rust_ = og_native_create(sampleRate_, std::max<UInt32>(maxFrames_, 1024), channels_, target);
        if (rust_) {
            og_native_bypass(rust_, bypass_);
            og_native_select(rust_, selection_);
            og_native_set_param(rust_, kParamInputGain, (inputGain_ + 24.0f) / 48.0f);
            og_native_set_param(rust_, kParamWetDryMix, wetDryMix_ / 100.0f);
            og_native_set_param(rust_, kParamDrive, drive_ / 10.0f);
            og_native_set_param(rust_, kParamOutputGain, (outputGain_ + 24.0f) / 48.0f);
            if (!savedState_.empty()) {
                og_native_set_state(rust_, savedState_.data(), static_cast<uint32_t>(savedState_.size()));
            }
            og_native_accept_latency(rust_, lastLatency_);
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
    Float32 inputGain_;
    Float32 wetDryMix_;
    Float32 drive_;
    Float32 outputGain_;
    uint32_t lastLatency_;
    bool initialized_;
    std::vector<uint8_t> savedState_;
    AURenderCallbackStruct inputCallback_;
    AudioStreamBasicDescription format_;
    AudioUnitConnection connection_;
    bool hasConnection_;
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

OSStatus AU_ScheduleParameters(void* self, const AudioUnitParameterEvent* inEvents, UInt32 inNumEvents) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->ScheduleParameters(inEvents, inNumEvents) : static_cast<ComponentResult>(-1);
}

OSStatus AU_Render(void* self, AudioUnitRenderActionFlags* ioFlags, const AudioTimeStamp* inTimeStamp, UInt32 inOutputBusNumber, UInt32 inNumberFrames, AudioBufferList* ioData) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->Render(ioFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData) : static_cast<ComponentResult>(-1);
}

OSStatus AU_AddRenderNotify(void* self, AURenderCallback inProc, void* inProcRefCon) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->AddRenderNotify(inProc, inProcRefCon) : static_cast<ComponentResult>(-1);
}

OSStatus AU_RemoveRenderNotify(void* self, AURenderCallback inProc, void* inProcRefCon) {
    auto* wrapper = static_cast<AudioComponentPlugInInstanceWrapper*>(self);
    return wrapper->mInstance ? wrapper->mInstance->RemoveRenderNotify(inProc, inProcRefCon) : static_cast<ComponentResult>(-1);
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
        case kAudioUnitScheduleParametersSelect: return reinterpret_cast<AudioComponentMethod>(AU_ScheduleParameters);
        case kAudioUnitRenderSelect: return reinterpret_cast<AudioComponentMethod>(AU_Render);
        case kAudioUnitAddRenderNotifySelect: return reinterpret_cast<AudioComponentMethod>(AU_AddRenderNotify);
        case kAudioUnitRemoveRenderNotifySelect: return reinterpret_cast<AudioComponentMethod>(AU_RemoveRenderNotify);
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
                sel == kAudioUnitScheduleParametersSelect ||
                sel == kAudioUnitRenderSelect ||
                sel == kAudioUnitAddRenderNotifySelect ||
                sel == kAudioUnitRemoveRenderNotifySelect ||
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
