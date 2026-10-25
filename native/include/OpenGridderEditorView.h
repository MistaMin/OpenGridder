#pragma once
#import <Cocoa/Cocoa.h>
#include "opengridder.h"

#ifndef OPENGRIDDER_VIEW_CLASS_NAME
#if defined(OG_BUILD_VST3)
#define OPENGRIDDER_VIEW_CLASS_NAME OpenGridderVST3EditorView
#elif defined(OG_BUILD_AUV3)
#define OPENGRIDDER_VIEW_CLASS_NAME OpenGridderAUv3EditorView
#elif defined(OG_BUILD_AU)
#define OPENGRIDDER_VIEW_CLASS_NAME OpenGridderAUEditorView
#elif defined(OG_BUILD_AAX)
#define OPENGRIDDER_VIEW_CLASS_NAME OpenGridderAAXEditorView
#else
#define OPENGRIDDER_VIEW_CLASS_NAME OpenGridderEditorView
#endif
#endif

#define OpenGridderEditorView OPENGRIDDER_VIEW_CLASS_NAME

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
    NativePlugin* (^_rustProvider)(void);
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
    float _vuLeft;
    float _vuRight;
    float _peakHoldLeft;
    float _peakHoldRight;
    BOOL _isVuMeterMode;
    BOOL _isVuCalibratedMinus18;
    BOOL _isMeterInputMode;
    BOOL _rackCollapsed;
    BOOL _consoleCollapsed;
    BOOL _meterCollapsed;
    void (^_onSizeChanged)(CGFloat, CGFloat);
    int _statusCode;
    uint32_t _latencySamples;
    NSMenu *_cachedPluginMenu;
    uint32_t _cachedPluginCount;
}

@property (nonatomic, assign) NativePlugin *rust;
@property (nonatomic, copy) NativePlugin* (^rustProvider)(void);
@property (nonatomic, assign) BOOL isBypassed;
@property (nonatomic, assign) int activeProfile;
@property (nonatomic, assign) uint32_t activeSlot;
@property (nonatomic, assign) int oversamplingFactor; // 1, 2, 4
@property (nonatomic, assign) float inputGain;   // 0..1
@property (nonatomic, assign) float wetDryMix;   // 0..1
@property (nonatomic, assign) float drive;       // 0..1
@property (nonatomic, assign) float outputGain;  // 0..1
@property (nonatomic, assign) BOOL isVuMeterMode;         // YES = Analog VU (2x width), NO = Digital PPM (1x width)
@property (nonatomic, assign) BOOL isVuCalibratedMinus18; // YES = 0 VU = +4 dBu = -18 dBFS, NO = 0 VU = -3 dBFS
@property (nonatomic, assign) BOOL isMeterInputMode;      // YES = Input metering (IN), NO = Output metering (OUT)
@property (nonatomic, assign) BOOL rackCollapsed;         // YES = Plugin Selection Bank clapped closed
@property (nonatomic, assign) BOOL consoleCollapsed;      // YES = Input/WetDry/Drive/Output Bank clapped closed
@property (nonatomic, assign) BOOL meterCollapsed;        // YES = Metering Bank clapped closed

@property (nonatomic, copy) void (^onParameterChanged)(uint32_t paramId, float normalizedValue);
@property (nonatomic, copy) void (^onProfileChanged)(int newProfile);
@property (nonatomic, copy) void (^onSizeChanged)(CGFloat newWidth, CGFloat newHeight);

- (instancetype)initWithFrame:(NSRect)frame;
- (NSSize)preferredViewSize;
- (void)updateLayoutSize;
- (void)updateStatus:(int)statusCode latencySamples:(uint32_t)latency;
- (void)showPluginMenuAtPoint:(NSPoint)pt;
- (void)onSelectPlugin:(NSMenuItem *)sender;

@end
