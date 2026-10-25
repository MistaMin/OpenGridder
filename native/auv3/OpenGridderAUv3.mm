// OpenGridder AUv3 (Audio Unit v3) Extension
// Native Apple AudioToolbox implementation - Zero JUCE.
#import <AudioToolbox/AudioToolbox.h>
#import <AVFoundation/AVFoundation.h>
#include <cstring>
#include "opengridder.h"

static const AUParameterAddress kParamBypass = 0;
static const AUParameterAddress kParamSelection = 1;
static const AUParameterAddress kParamLatencyProfile = 2;

@interface OpenGridderAUv3 : AUAudioUnit
@property (nonatomic, strong) AUAudioUnitBusArray *inputBusArray;
@property (nonatomic, strong) AUAudioUnitBusArray *outputBusArray;
@property (nonatomic, strong) AUAudioUnitBus *inputBus;
@property (nonatomic, strong) AUAudioUnitBus *outputBus;
@property (nonatomic, strong) AUParameterTree *paramTree;
@end

@implementation OpenGridderAUv3 {
    NativePlugin *_rust;
    AVAudioFormat *_defaultFormat;
    AUParameter *_bypassParam;
    AUParameter *_selectionParam;
    AUParameter *_profileParam;
    
    // Scratch buffer list for pulling upstream audio
    AudioBufferList *_pullBufferList;
    float *_pullScratchLeft;
    float *_pullScratchRight;
    size_t _pullCapacityFrames;
}

- (instancetype)initWithComponentDescription:(AudioComponentDescription)componentDescription
                                     options:(AudioComponentInstantiationOptions)options
                                       error:(NSError **)outError {
    self = [super initWithComponentDescription:componentDescription options:options error:outError];
    if (!self) return nil;

    _rust = nullptr;
    _pullCapacityFrames = 4096;
    _pullScratchLeft = (float *)calloc(_pullCapacityFrames, sizeof(float));
    _pullScratchRight = (float *)calloc(_pullCapacityFrames, sizeof(float));
    
    _pullBufferList = (AudioBufferList *)malloc(sizeof(AudioBufferList) + sizeof(AudioBuffer));
    _pullBufferList->mNumberBuffers = 2;
    _pullBufferList->mBuffers[0].mNumberChannels = 1;
    _pullBufferList->mBuffers[0].mDataByteSize = (UInt32)(_pullCapacityFrames * sizeof(float));
    _pullBufferList->mBuffers[0].mData = _pullScratchLeft;
    _pullBufferList->mBuffers[1].mNumberChannels = 1;
    _pullBufferList->mBuffers[1].mDataByteSize = (UInt32)(_pullCapacityFrames * sizeof(float));
    _pullBufferList->mBuffers[1].mData = _pullScratchRight;

    _defaultFormat = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:44100.0 channels:2];

    NSError *busError = nil;
    _inputBus = [[AUAudioUnitBus alloc] initWithFormat:_defaultFormat error:&busError];
    _outputBus = [[AUAudioUnitBus alloc] initWithFormat:_defaultFormat error:&busError];

    _inputBusArray = [[AUAudioUnitBusArray alloc] initWithAudioUnit:self
                                                            busType:AUAudioUnitBusTypeInput
                                                             busses:@[_inputBus]];
    _outputBusArray = [[AUAudioUnitBusArray alloc] initWithAudioUnit:self
                                                             busType:AUAudioUnitBusTypeOutput
                                                              busses:@[_outputBus]];

    _bypassParam = [AUParameterTree createParameterWithIdentifier:@"bypass"
                                                             name:@"Bypass"
                                                          address:kParamBypass
                                                              min:0.0f
                                                              max:1.0f
                                                             unit:kAudioUnitParameterUnit_Boolean
                                                         unitName:nil
                                                            flags:kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_IsWritable
                                                     valueStrings:nil
                                              dependentParameters:nil];
    _bypassParam.value = 0.0f;

    _selectionParam = [AUParameterTree createParameterWithIdentifier:@"selection"
                                                                name:@"Plugin Slot"
                                                             address:kParamSelection
                                                                 min:0.0f
                                                                 max:63.0f
                                                                unit:kAudioUnitParameterUnit_Indexed
                                                            unitName:nil
                                                               flags:kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_IsWritable
                                                        valueStrings:nil
                                                 dependentParameters:nil];
    _selectionParam.value = 0.0f;

    NSArray *profileNames = @[@"Low (128)", @"Standard (256)", @"High (512)", @"Studio (1024)"];
    _profileParam = [AUParameterTree createParameterWithIdentifier:@"latencyProfile"
                                                              name:@"Latency Profile"
                                                           address:kParamLatencyProfile
                                                               min:0.0f
                                                               max:3.0f
                                                              unit:kAudioUnitParameterUnit_Indexed
                                                          unitName:nil
                                                             flags:kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_IsWritable
                                                      valueStrings:profileNames
                                               dependentParameters:nil];
    _profileParam.value = 1.0f; // Standard 256

    _paramTree = [AUParameterTree createTreeWithChildren:@[_bypassParam, _selectionParam, _profileParam]];
    self.parameterTree = _paramTree;

    return self;
}

- (void)dealloc {
    [self deallocateRenderResources];
    if (_pullScratchLeft) free(_pullScratchLeft);
    if (_pullScratchRight) free(_pullScratchRight);
    if (_pullBufferList) free(_pullBufferList);
}

- (AUAudioUnitBusArray *)inputBusses {
    return _inputBusArray;
}

- (AUAudioUnitBusArray *)outputBusses {
    return _outputBusArray;
}

- (BOOL)allocateRenderResourcesAndReturnError:(NSError **)outError {
    if (![super allocateRenderResourcesAndReturnError:outError]) {
        return NO;
    }

    double sampleRate = _inputBus.format.sampleRate;
    uint32_t maxFrames = self.maximumFramesToRender;
    if (maxFrames == 0) maxFrames = 1024;
    uint32_t latency = 256;

    if (_rust) {
        og_native_destroy(_rust);
        _rust = nullptr;
    }

    _rust = og_native_create(sampleRate, maxFrames, 2, latency);
    if (!_rust) {
        if (outError) {
            *outError = [NSError errorWithDomain:@"org.opengridder.auv3"
                                            code:-1
                                        userInfo:@{NSLocalizedDescriptionKey: @"Failed to create OpenGridder native engine"}];
        }
        return NO;
    }

    og_native_bypass(_rust, _bypassParam.value > 0.5f);
    og_native_select(_rust, (uint32_t)_selectionParam.value);

    return YES;
}

- (void)deallocateRenderResources {
    if (_rust) {
        og_native_destroy(_rust);
        _rust = nullptr;
    }
    [super deallocateRenderResources];
}

- (NSTimeInterval)latency {
    if (!_rust) return 0.0;
    double sampleRate = _inputBus.format.sampleRate;
    if (sampleRate <= 0.0) sampleRate = 44100.0;
    return (NSTimeInterval)og_native_latency(_rust) / sampleRate;
}

- (AUInternalRenderBlock)internalRenderBlock {
    __block NativePlugin *rust = _rust;
    __block AudioBufferList *pullBufferList = _pullBufferList;
    __block AUParameter *bypassParam = _bypassParam;
    __block AUParameter *selectionParam = _selectionParam;
    __block AUParameter *profileParam = _profileParam;

    return [^AUAudioUnitStatus(
        AudioUnitRenderActionFlags *actionFlags,
        const AudioTimeStamp *timestamp,
        AVAudioFrameCount frameCount,
        NSInteger outputBusNumber,
        AudioBufferList *outputData,
        const AURenderEvent *realtimeEventListHead,
        AURenderPullInputBlock pullInputBlock
    ) {
        // 1. Handle realtime events
        for (const AURenderEvent *event = realtimeEventListHead; event != NULL; event = event->head.next) {
            if (event->head.eventType == AURenderEventParameter) {
                AUParameterAddress addr = event->parameter.parameterAddress;
                AUValue val = event->parameter.value;
                if (addr == kParamBypass) {
                    if (rust) og_native_bypass(rust, val > 0.5f);
                } else if (addr == kParamSelection) {
                    if (rust) og_native_select(rust, (uint32_t)val);
                } else if (addr == kParamLatencyProfile) {
                    uint32_t prof = (uint32_t)val;
                    uint32_t lat = (prof == 0) ? 128 : (prof == 1) ? 256 : (prof == 2) ? 512 : 1024;
                    if (rust) og_native_set_transport_latency(rust, lat);
                }
            }
        }

        // 2. Pull audio from input
        if (pullInputBlock) {
            AudioUnitRenderActionFlags pullFlags = 0;
            pullBufferList->mBuffers[0].mDataByteSize = frameCount * sizeof(float);
            pullBufferList->mBuffers[1].mDataByteSize = frameCount * sizeof(float);
            AUAudioUnitStatus status = pullInputBlock(&pullFlags, timestamp, frameCount, 0, pullBufferList);
            if (status != noErr) return status;
        } else {
            // Silence input
            std::memset(pullBufferList->mBuffers[0].mData, 0, frameCount * sizeof(float));
            std::memset(pullBufferList->mBuffers[1].mData, 0, frameCount * sizeof(float));
        }

        // 3. Process or pass-through
        bool bypassed = (bypassParam.value > 0.5f);
        const float *inPtrs[2] = {
            (const float *)pullBufferList->mBuffers[0].mData,
            (const float *)pullBufferList->mBuffers[1].mData
        };

        float *outPtrs[2] = {
            (float *)outputData->mBuffers[0].mData,
            (outputData->mNumberBuffers > 1) ? (float *)outputData->mBuffers[1].mData : (float *)outputData->mBuffers[0].mData
        };

        if (bypassed || !rust) {
            std::memcpy(outPtrs[0], inPtrs[0], frameCount * sizeof(float));
            std::memcpy(outPtrs[1], inPtrs[1], frameCount * sizeof(float));
            return noErr;
        }

        int32_t result = og_native_process(rust, inPtrs, outPtrs, (uint32_t)frameCount);
        if (result < 0) {
            // Disconnected fallback: pass through
            std::memcpy(outPtrs[0], inPtrs[0], frameCount * sizeof(float));
            std::memcpy(outPtrs[1], inPtrs[1], frameCount * sizeof(float));
        }

        return noErr;
    } copy];
}

@end

#pragma mark - AUv3 Factory Implementation

@interface OpenGridderAUFactory : NSObject <AUAudioUnitFactory>
@end

@implementation OpenGridderAUFactory

- (void)beginRequestWithExtensionContext:(NSExtensionContext *)context {
}

- (AUAudioUnit *)createAudioUnitWithComponentDescription:(AudioComponentDescription)desc error:(NSError **)error {
    return [[OpenGridderAUv3 alloc] initWithComponentDescription:desc options:0 error:error];
}

@end
