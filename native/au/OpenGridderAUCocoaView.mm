#import <Cocoa/Cocoa.h>
#import <AudioUnit/AudioUnit.h>
#import <AudioUnit/AUCocoaUIView.h>
#import "OpenGridderEditorView.h"

@interface OpenGridderAUCocoaViewFactory : NSObject <AUCocoaUIBase>
- (NSView *)uiViewForAudioUnit:(AudioUnit)inAudioUnit withSize:(NSSize)inPreferredSize;
- (unsigned)interfaceVersion;
@end

@implementation OpenGridderAUCocoaViewFactory

- (unsigned)interfaceVersion {
    return 0;
}

- (NSView *)uiViewForAudioUnit:(AudioUnit)inAudioUnit withSize:(NSSize)inPreferredSize {
    OpenGridderEditorView *view = [[OpenGridderEditorView alloc] initWithFrame:NSMakeRect(0, 0, 720, 420)];

    view.rustProvider = ^NativePlugin* {
        NativePlugin* r = nullptr;
        UInt32 size = sizeof(NativePlugin*);
        if (AudioUnitGetProperty(inAudioUnit, 65536, kAudioUnitScope_Global, 0, &r, &size) == noErr) {
            return r;
        }
        return nullptr;
    };

    Float32 bypass = 0.0f;
    AudioUnitGetParameter(inAudioUnit, 0 /* kParamBypass */, kAudioUnitScope_Global, 0, &bypass);
    view.isBypassed = (bypass > 0.5f);

    Float32 profile = 1.0f;
    AudioUnitGetParameter(inAudioUnit, 2 /* kParamLatencyProfile */, kAudioUnitScope_Global, 0, &profile);
    view.activeProfile = (int)profile;

    view.onParameterChanged = ^(uint32_t paramId, float normalizedValue) {
        if (paramId == 0) {
            AudioUnitSetParameter(inAudioUnit, 0, kAudioUnitScope_Global, 0, normalizedValue, 0);
        }
    };
    view.onProfileChanged = ^(int newProfile) {
        AudioUnitSetParameter(inAudioUnit, 2, kAudioUnitScope_Global, 0, (Float32)newProfile, 0);
    };

    return view;
}

@end
