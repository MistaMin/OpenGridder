#import "OpenGridderRemoteView.h"
#import <objc/message.h>
#include <cmath>

@implementation OpenGridderRemoteView

@synthesize rust = _rust;

- (instancetype)initWithFrame:(NSRect)frame {
    self = [super initWithFrame:frame];
    if (self) {
        _rust = NULL;
        _lastFrameId = 0;
        _frameWidth = 0;
        _frameHeight = 0;
        _frameScale = 1.0;
        _frameData = [[NSMutableData alloc] init];
        _cgImage = NULL;

        _renderTimer = [NSTimer timerWithTimeInterval:1.0 / 30.0
                                               target:self
                                             selector:@selector(checkNewFrame)
                                             userInfo:nil
                                              repeats:YES];
        [[NSRunLoop mainRunLoop] addTimer:_renderTimer forMode:NSRunLoopCommonModes];
    }
    return self;
}

- (void)viewWillMoveToWindow:(NSWindow *)newWindow {
    [super viewWillMoveToWindow:newWindow];
    if (newWindow == nil) {
        [_renderTimer invalidate];
        _renderTimer = nil;
    } else if (_renderTimer == nil) {
        _renderTimer = [NSTimer timerWithTimeInterval:1.0 / 30.0
                                               target:self
                                             selector:@selector(checkNewFrame)
                                             userInfo:nil
                                              repeats:YES];
        [[NSRunLoop mainRunLoop] addTimer:_renderTimer forMode:NSRunLoopCommonModes];
    }
}

- (void)dealloc {
    [_renderTimer invalidate];
    _renderTimer = nil;
    if (_cgImage) {
        CGImageRelease(_cgImage);
        _cgImage = NULL;
    }
#if !__has_feature(objc_arc)
    [_frameData release];
    [super dealloc];
#endif
}

- (BOOL)acceptsFirstResponder {
    return YES;
}

- (BOOL)becomeFirstResponder {
    return YES;
}

- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    return YES;
}

- (BOOL)isFlipped {
    return NO;
}

- (void)checkNewFrame {
    if (!_rust) return;

    uint32_t w = 0, h = 0;
    double s = 1.0;
    uint64_t fid = 0;
    if (og_native_get_screen_info(_rust, &w, &h, &s, &fid)) {
        if (fid != _lastFrameId && w > 0 && h > 0) {
            uint32_t needed = w * h * 4;
            if (_frameData.length < needed) {
                [_frameData setLength:needed];
            }
            if (og_native_copy_screen_frame(_rust, (uint8_t *)_frameData.mutableBytes, needed, &fid)) {
                _lastFrameId = fid;
                _frameWidth = w;
                _frameHeight = h;
                _frameScale = (s > 0.0) ? s : 1.0;

                if (_cgImage) {
                    CGImageRelease(_cgImage);
                    _cgImage = NULL;
                }
                CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
                CGDataProviderRef dp = CGDataProviderCreateWithData(NULL, _frameData.bytes, needed, NULL);
                _cgImage = CGImageCreate(
                    w, h, 8, 32, w * 4, cs,
                    kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big,
                    dp, NULL, false, kCGRenderingIntentDefault
                );
                CGDataProviderRelease(dp);
                CGColorSpaceRelease(cs);

                NSWindow *win = self.window;
                if (win) {
                    CGFloat targetW = w / _frameScale;
                    CGFloat targetH = h / _frameScale;
                    NSRect curContent = [win contentRectForFrameRect:win.frame];
                    if (std::abs(curContent.size.width - targetW) > 2.0 || std::abs(curContent.size.height - targetH) > 2.0) {
                        NSRect newContent = NSMakeRect(curContent.origin.x, curContent.origin.y + (curContent.size.height - targetH), targetW, targetH);
                        NSRect newWinFrame = [win frameRectForContentRect:newContent];
                        [win setFrame:newWinFrame display:YES animate:NO];
                    }
                }
                [self setNeedsDisplay:YES];
            }
        }
    }
}

- (void)drawRect:(NSRect)dirtyRect {
    NSGraphicsContext *nsCtx = [NSGraphicsContext currentContext];
    CGContextRef ctx = NULL;
    if ([nsCtx respondsToSelector:@selector(CGContext)]) {
        ctx = ((CGContextRef(*)(id, SEL))objc_msgSend)(nsCtx, @selector(CGContext));
    }
    if (!ctx) {
        ctx = (CGContextRef)[nsCtx graphicsPort];
    }
    CGRect bounds = NSRectToCGRect(self.bounds);

    if (_cgImage) {
        CGContextDrawImage(ctx, bounds, _cgImage);
    } else {
        CGContextSetRGBFillColor(ctx, 0.10, 0.12, 0.14, 1.0);
        CGContextFillRect(ctx, bounds);

        NSDictionary *attrs = @{
            NSFontAttributeName: [NSFont boldSystemFontOfSize:13],
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.6 alpha:1.0]
        };
        NSString *msg = @"Waiting for remote plugin GUI stream...";
        NSSize sz = [msg sizeWithAttributes:attrs];
        [msg drawAtPoint:NSMakePoint((bounds.size.width - sz.width) * 0.5, (bounds.size.height - sz.height) * 0.5) withAttributes:attrs];
    }
}

#pragma mark - Input Forwarding Helpers

- (NSPoint)pluginPointForEvent:(NSEvent *)event {
    NSPoint loc = [self convertPoint:event.locationInWindow fromView:nil];
    CGFloat yFromTop = self.bounds.size.height - loc.y;
    double s = (_frameScale > 0.0) ? _frameScale : 1.0;
    return NSMakePoint(loc.x * s, yFromTop * s);
}

- (void)sendMouseEv:(uint8_t)evType event:(NSEvent *)event deltaX:(float)dx deltaY:(float)dy {
    if (!_rust) return;
    NSPoint pt = [self pluginPointForEvent:event];
    NSUInteger flags = [event modifierFlags];
    BOOL shift = (flags & (1U << 17)) != 0;
    BOOL ctrl = (flags & (1U << 18)) != 0;
    BOOL alt = (flags & (1U << 19)) != 0;

    og_native_send_mouse(_rust, evType, (float)pt.x, (float)pt.y, shift, ctrl, alt, dx, dy, NO);
}

- (void)mouseDown:(NSEvent *)event {
    [self sendMouseEv:1 event:event deltaX:0 deltaY:0]; // LeftDown
}

- (void)mouseUp:(NSEvent *)event {
    [self sendMouseEv:2 event:event deltaX:0 deltaY:0]; // LeftUp
}

- (void)mouseDragged:(NSEvent *)event {
    [self sendMouseEv:3 event:event deltaX:(float)event.deltaX deltaY:(float)event.deltaY]; // LeftDrag
}

- (void)rightMouseDown:(NSEvent *)event {
    [self sendMouseEv:4 event:event deltaX:0 deltaY:0]; // RightDown
}

- (void)rightMouseUp:(NSEvent *)event {
    [self sendMouseEv:5 event:event deltaX:0 deltaY:0]; // RightUp
}

- (void)rightMouseDragged:(NSEvent *)event {
    [self sendMouseEv:6 event:event deltaX:(float)event.deltaX deltaY:(float)event.deltaY]; // RightDrag
}

- (void)otherMouseDown:(NSEvent *)event {
    [self sendMouseEv:7 event:event deltaX:0 deltaY:0]; // OtherDown
}

- (void)otherMouseUp:(NSEvent *)event {
    [self sendMouseEv:8 event:event deltaX:0 deltaY:0]; // OtherUp
}

- (void)otherMouseDragged:(NSEvent *)event {
    [self sendMouseEv:9 event:event deltaX:(float)event.deltaX deltaY:(float)event.deltaY]; // OtherDrag
}

- (void)scrollWheel:(NSEvent *)event {
    [self sendMouseEv:10 event:event deltaX:(float)event.scrollingDeltaX deltaY:(float)event.scrollingDeltaY]; // Wheel
}

- (void)mouseMoved:(NSEvent *)event {
    [self sendMouseEv:0 event:event deltaX:0 deltaY:0]; // Move
}

- (void)keyDown:(NSEvent *)event {
    if (!_rust) return;
    unsigned short kc = event.keyCode;
    og_native_send_key(_rust, &kc, 1);
}

@end

#pragma mark - Window Controller

@implementation OpenGridderRemoteWindowController

static OpenGridderRemoteWindowController *s_instance = nil;

+ (instancetype)sharedController {
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        s_instance = [[OpenGridderRemoteWindowController alloc] init];
    });
    return s_instance;
}

- (instancetype)init {
    self = [super init];
    if (self) {
        _currentRust = NULL;
        _currentSlot = 0;
    }
    return self;
}

- (void)showForPlugin:(NativePlugin *)rust slotIndex:(uint32_t)slot title:(NSString *)title {
    _currentRust = rust;
    _currentSlot = slot;

    if (!self.window) {
        NSRect frame = NSMakeRect(200, 200, 640, 420);
        NSUInteger style = (1U << 0) | (1U << 1) | (1U << 2) | (1U << 3);
        NSWindow *win = [[NSWindow alloc] initWithContentRect:frame
                                                    styleMask:style
                                                      backing:NSBackingStoreBuffered
                                                        defer:NO];
        win.delegate = self;
        win.level = NSFloatingWindowLevel;
        win.releasedWhenClosed = NO;
        self.window = win;
#if !__has_feature(objc_arc)
        [win release];
#endif

        OpenGridderRemoteView *view = [[OpenGridderRemoteView alloc] initWithFrame:frame];
        view.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        self.window.contentView = view;
#if !__has_feature(objc_arc)
        [view release];
#endif
    }

    self.window.title = [NSString stringWithFormat:@"[OpenGridder] %@", title ?: @"Remote Plugin"];
    OpenGridderRemoteView *view = (OpenGridderRemoteView *)self.window.contentView;
    view.rust = rust;
    [view checkNewFrame];

    [self.window makeKeyAndOrderFront:nil];
}

- (void)hide {
    if (self.window && self.window.isVisible) {
        [self.window orderOut:nil];
        if (_currentRust) {
            og_native_hide_plugin(_currentRust);
        }
    }
}

- (void)windowWillClose:(NSNotification *)notification {
    if (_currentRust) {
        og_native_hide_plugin(_currentRust);
    }
}

@end
