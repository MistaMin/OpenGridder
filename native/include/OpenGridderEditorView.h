#pragma once
#import <Cocoa/Cocoa.h>
#include "opengridder.h"

#ifndef NSFontWeightRegular
#if !defined(MAC_OS_X_VERSION_10_11) || MAC_OS_X_VERSION_MAX_ALLOWED < 101100
typedef CGFloat NSFontWeight;
#define NSFontWeightUltraLight (-0.8)
#define NSFontWeightThin (-0.6)
#define NSFontWeightLight (-0.4)
#define NSFontWeightRegular (0.0)
#define NSFontWeightMedium (0.23)
#define NSFontWeightSemibold (0.3)
#define NSFontWeightBold (0.4)
#define NSFontWeightHeavy (0.56)
#define NSFontWeightBlack (0.62)
#endif
#endif

@interface OpenGridderEditorView : NSView {
@public
    NativePlugin *_rust;
    BOOL _isBypassed;
    int _activeProfile;
    uint32_t _activeSlot;
    int _oversamplingFactor;
    float _inputGain;
    float _wetDryMix;
    float _drive;
    float _outputGain;
    void (^_onParameterChanged)(uint32_t, float);
    void (^_onProfileChanged)(int);

@private
    int _draggedKnob;
    NSPoint _dragStartPoint;
    float _dragStartValue;
    NSTimer *_meterTimer;
    float _meterLeft;
    float _meterRight;
    int _statusCode;
    uint32_t _latencySamples;
}

@property (nonatomic, assign) NativePlugin *rust;
@property (nonatomic, assign) BOOL isBypassed;
@property (nonatomic, assign) int activeProfile;
@property (nonatomic, assign) uint32_t activeSlot;
@property (nonatomic, assign) int oversamplingFactor; // 1, 2, 4
@property (nonatomic, assign) float inputGain;   // 0..1
@property (nonatomic, assign) float wetDryMix;   // 0..1
@property (nonatomic, assign) float drive;       // 0..1
@property (nonatomic, assign) float outputGain;  // 0..1

@property (nonatomic, copy) void (^onParameterChanged)(uint32_t paramId, float normalizedValue);
@property (nonatomic, copy) void (^onProfileChanged)(int newProfile);

- (instancetype)initWithFrame:(NSRect)frame;
- (void)updateStatus:(int)statusCode latencySamples:(uint32_t)latency;

@end
