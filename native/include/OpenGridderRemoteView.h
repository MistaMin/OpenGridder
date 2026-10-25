#pragma once
#import <Cocoa/Cocoa.h>
#include "opengridder.h"

#ifndef OPENGRIDDER_REMOTE_VIEW_CLASS_NAME
#if defined(OG_BUILD_VST3)
#define OPENGRIDDER_REMOTE_VIEW_CLASS_NAME OpenGridderVST3RemoteView
#define OPENGRIDDER_REMOTE_WIN_CLASS_NAME OpenGridderVST3RemoteWindowController
#elif defined(OG_BUILD_AUV3)
#define OPENGRIDDER_REMOTE_VIEW_CLASS_NAME OpenGridderAUv3RemoteView
#define OPENGRIDDER_REMOTE_WIN_CLASS_NAME OpenGridderAUv3RemoteWindowController
#elif defined(OG_BUILD_AU)
#define OPENGRIDDER_REMOTE_VIEW_CLASS_NAME OpenGridderAURemoteView
#define OPENGRIDDER_REMOTE_WIN_CLASS_NAME OpenGridderAURemoteWindowController
#elif defined(OG_BUILD_AAX)
#define OPENGRIDDER_REMOTE_VIEW_CLASS_NAME OpenGridderAAXRemoteView
#define OPENGRIDDER_REMOTE_WIN_CLASS_NAME OpenGridderAAXRemoteWindowController
#else
#define OPENGRIDDER_REMOTE_VIEW_CLASS_NAME OpenGridderRemoteView
#define OPENGRIDDER_REMOTE_WIN_CLASS_NAME OpenGridderRemoteWindowController
#endif
#endif

#define OpenGridderRemoteView OPENGRIDDER_REMOTE_VIEW_CLASS_NAME
#define OpenGridderRemoteWindowController OPENGRIDDER_REMOTE_WIN_CLASS_NAME

@interface OpenGridderRemoteView : NSView {
@public
    NativePlugin *_rust;
@private
    uint64_t _lastFrameId;
    uint32_t _frameWidth;
    uint32_t _frameHeight;
    double _frameScale;
    NSMutableData *_frameData;
    CGImageRef _cgImage;
    NSTimer *_renderTimer;
}

@property (nonatomic, assign) NativePlugin *rust;

- (instancetype)initWithFrame:(NSRect)frame;
- (void)checkNewFrame;

@end

@interface OpenGridderRemoteWindowController : NSWindowController <NSWindowDelegate> {
@private
    NativePlugin *_currentRust;
    uint32_t _currentSlot;
}

+ (instancetype)sharedController;
- (void)showForPlugin:(NativePlugin *)rust slotIndex:(uint32_t)slot title:(NSString *)title;
- (void)hide;

@end
