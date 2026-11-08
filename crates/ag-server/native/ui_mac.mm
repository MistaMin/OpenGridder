// Plugin editor windows for the server, the way AudioGridder's ProcessorWindow works:
// the plugin's real editor is shown in a real window on the server's screen, the
// window's pixels are captured (CGWindowListCreateImage on this window only, so other
// windows covering it do not matter) and streamed to the client, and the client's mouse
// and key events are delivered to that window.
//
// Input is posted to the editor window as NSEvents instead of global CGEvents, so the
// server Mac's own pointer and keyboard focus are never moved (AudioGridder posts global
// events; on a server that is also the DAW machine that fights the user's mouse).
//
// Everything that touches AppKit or a plugin view runs on the main thread, which runs
// the application loop (ag_ui_run). Capture runs on any thread.

#import <Cocoa/Cocoa.h>
#import <AudioToolbox/AudioToolbox.h>
#import <AudioUnit/AUCocoaUIView.h>
#import <CoreAudioKit/CoreAudioKit.h>
#import <Carbon/Carbon.h>
#import <ImageIO/ImageIO.h>
#import <objc/runtime.h>

#include <atomic>
#include <map>
#include <mutex>

extern "C" int ag_native_editor_open(void* raw, void* nsview, int* width, int* height);
extern "C" void ag_native_editor_close(void* raw);

namespace {

std::atomic<bool> g_loop_running{false};
std::atomic<uint64_t> g_next_id{1};

// What a capture needs, readable from any thread.
struct CaptureInfo {
    CGWindowID window = 0;
    CGRect rect = CGRectZero;  // content area in global display coordinates (top-left origin)
    int width = 0;
    int height = 0;
    bool open = false;
};

std::mutex g_capture_mutex;
std::map<uint64_t, CaptureInfo>& captures() {
    static std::map<uint64_t, CaptureInfo> map;
    return map;
}

void run_on_main(void (^work)(void)) {
    if ([NSThread isMainThread] || !g_loop_running.load()) {
        work();
    } else {
        dispatch_sync(dispatch_get_main_queue(), work);
    }
}

}  // namespace

// The editor window. It may become key so AppKit routes posted key events and first
// clicks to the plugin view.
@interface AGEditorWindow : NSWindow
@end
@implementation AGEditorWindow
- (BOOL)canBecomeKeyWindow { return YES; }
- (BOOL)canBecomeMainWindow { return NO; }
@end

@interface AGEditor : NSObject <NSWindowDelegate>
@property (nonatomic) uint64_t editorId;
@property (nonatomic, strong) AGEditorWindow* window;
@property (nonatomic, strong) NSView* pluginView;   // AU view, or the host container for VST3/CLAP
@property (nonatomic) void* native;                  // AgNative* for VST3/CLAP, else NULL
@property (nonatomic) BOOL closed;
@end

static NSMutableDictionary<NSNumber*, AGEditor*>* editors() {
    static NSMutableDictionary* map = nil;
    if (!map) map = [NSMutableDictionary dictionary];
    return map;
}

static void publish_capture(AGEditor* editor) {
    CaptureInfo info;
    if (editor.window && !editor.closed) {
        NSRect content = [editor.window contentRectForFrameRect:editor.window.frame];
        CGFloat primary = NSMaxY([NSScreen screens].firstObject.frame);
        info.window = (CGWindowID)editor.window.windowNumber;
        info.rect = CGRectMake(content.origin.x, primary - NSMaxY(content), content.size.width, content.size.height);
        info.width = (int)lround(content.size.width);
        info.height = (int)lround(content.size.height);
        info.open = true;
    }
    std::lock_guard<std::mutex> lock(g_capture_mutex);
    captures()[editor.editorId] = info;
}

@implementation AGEditor

- (void)teardown {
    if (self.closed) return;
    self.closed = YES;
    [[NSNotificationCenter defaultCenter] removeObserver:self];
    if (self.native) {
        ag_native_editor_close(self.native);  // IPlugView::removed / clap gui destroy
        self.native = NULL;
    }
    [self.pluginView removeFromSuperview];
    self.pluginView = nil;
    publish_capture(self);
}

- (void)windowWillClose:(NSNotification*)note {
    // Closed on the server's screen: tear the view down now, the session sees the
    // editor as closed and tells the client.
    [self teardown];
}

- (void)windowDidMove:(NSNotification*)note { publish_capture(self); }
- (void)windowDidResize:(NSNotification*)note { publish_capture(self); }

- (void)pluginViewFrameChanged:(NSNotification*)note {
    // AU views resize themselves; the window follows.
    NSSize size = self.pluginView.frame.size;
    if (size.width >= 1 && size.height >= 1 && !NSEqualSizes(size, [self.window contentRectForFrameRect:self.window.frame].size)) {
        [self.window setContentSize:size];
        publish_capture(self);
    }
}

@end

// JUCE (and others) ask the window server which window is topmost at the click point
// before accepting a click. Injected clicks land on a window that may sit behind other
// applications' windows, so answer with this process's own front-most window there.
static void install_window_at_point_hook(void) {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        SEL sel = @selector(windowNumberAtPoint:belowWindowWithWindowNumber:);
        Method method = class_getClassMethod([NSWindow class], sel);
        if (!method) return;
        typedef NSInteger (*Original)(id, SEL, NSPoint, NSInteger);
        Original original = (Original)method_getImplementation(method);
        IMP replacement = imp_implementationWithBlock(^NSInteger(id self_, NSPoint point, NSInteger below) {
            if (below == 0) {
                for (NSWindow* candidate in [NSApp orderedWindows]) {
                    if (candidate.isVisible && NSPointInRect(point, candidate.frame)) return candidate.windowNumber;
                }
            }
            return original(self_, sel, point, below);
        });
        method_setImplementation(method, replacement);
    });
}

static NSView* au_cocoa_view(AudioUnit unit) {
    UInt32 size = 0;
    Boolean writable = false;
    if (AudioUnitGetPropertyInfo(unit, kAudioUnitProperty_CocoaUI, kAudioUnitScope_Global, 0, &size, &writable) != noErr ||
        size < sizeof(AudioUnitCocoaViewInfo)) {
        return nil;
    }
    AudioUnitCocoaViewInfo* info = (AudioUnitCocoaViewInfo*)malloc(size);
    NSView* view = nil;
    if (AudioUnitGetProperty(unit, kAudioUnitProperty_CocoaUI, kAudioUnitScope_Global, 0, info, &size) == noErr) {
        const int classes = (int)((size - sizeof(CFURLRef)) / sizeof(CFStringRef));
        NSURL* url = (__bridge NSURL*)info->mCocoaAUViewBundleLocation;
        NSString* className = classes > 0 ? (__bridge NSString*)info->mCocoaAUViewClass[0] : nil;
        NSBundle* bundle = url ? [NSBundle bundleWithURL:url] : nil;
        Class factoryClass = (bundle && className) ? [bundle classNamed:className] : Nil;
        if (factoryClass && [factoryClass conformsToProtocol:@protocol(AUCocoaUIBase)]) {
            id<AUCocoaUIBase> factory = [[factoryClass alloc] init];
            view = [factory uiViewForAudioUnit:unit withSize:NSZeroSize];
        }
        for (int i = 0; i < classes; ++i) {
            if (info->mCocoaAUViewClass[i]) CFRelease(info->mCocoaAUViewClass[i]);
        }
        if (info->mCocoaAUViewBundleLocation) CFRelease(info->mCocoaAUViewBundleLocation);
    }
    free(info);
    return view;
}

// AUv3 (and v2 units that offer it): ask for a view controller. The answer may come
// asynchronously on the main queue, so spin the run loop while waiting.
static NSView* au_view_controller_view(AudioUnit unit) {
    __block NSViewController* controller = nil;
    __block BOOL answered = NO;
    void (^completion)(AUViewControllerBase*) = ^(AUViewControllerBase* vc) {
        controller = (NSViewController*)vc;
        answered = YES;
    };
    void* block = (__bridge void*)completion;
    if (AudioUnitSetProperty(unit, kAudioUnitProperty_RequestViewController, kAudioUnitScope_Global, 0, &block, sizeof(block)) != noErr) {
        return nil;
    }
    NSDate* until = [NSDate dateWithTimeIntervalSinceNow:3.0];
    while (!answered && [until timeIntervalSinceNow] > 0) {
        [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
    }
    if (!controller) return nil;
    NSView* view = controller.view;
    NSSize preferred = controller.preferredContentSize;
    if (preferred.width >= 50 && preferred.height >= 50) [view setFrameSize:preferred];
    // Keep the controller alive as long as its view.
    objc_setAssociatedObject(view, "ag.controller", controller, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    return view;
}

static uint64_t show_editor(NSView* view, void* native, const char* title, int x, int y) {
    NSSize size = view.frame.size;
    if (size.width < 50 || size.height < 50) size = NSMakeSize(640, 420);
    AGEditorWindow* window = [[AGEditorWindow alloc] initWithContentRect:NSMakeRect(0, 0, size.width, size.height)
                                                               styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                                                                          NSWindowStyleMaskMiniaturizable)
                                                                 backing:NSBackingStoreBuffered
                                                                   defer:NO];
    window.releasedWhenClosed = NO;
    window.title = title && title[0] ? [NSString stringWithUTF8String:title] : @"Plugin";
    window.acceptsMouseMovedEvents = YES;
    window.contentView = view;
    // (x, y) is the top-left corner in top-left-origin screen coordinates, as the client
    // sends it (ProcessorWindow::move). Without one, use the main screen's top-left.
    NSRect visible = [NSScreen mainScreen].visibleFrame;
    CGFloat primary = NSMaxY([NSScreen screens].firstObject.frame);
    NSPoint topLeft = (x > 0 || y > 0) ? NSMakePoint(x, primary - y) : NSMakePoint(NSMinX(visible) + 40, NSMaxY(visible) - 40);
    [window setFrameTopLeftPoint:topLeft];
    // Keep the whole window on a screen: AudioGridder can only capture a fully visible window.
    NSRect frame = window.frame;
    NSScreen* screen = window.screen ?: [NSScreen mainScreen];
    NSRect area = screen.visibleFrame;
    if (NSMaxX(frame) > NSMaxX(area)) frame.origin.x = MAX(NSMinX(area), NSMaxX(area) - frame.size.width);
    if (NSMinY(frame) < NSMinY(area)) frame.origin.y = MIN(NSMaxY(area) - frame.size.height, NSMinY(area));
    [window setFrame:frame display:NO];

    AGEditor* editor = [[AGEditor alloc] init];
    editor.editorId = g_next_id.fetch_add(1);
    editor.window = window;
    editor.pluginView = view;
    editor.native = native;
    window.delegate = editor;
    if (!native) {
        view.postsFrameChangedNotifications = YES;
        [[NSNotificationCenter defaultCenter] addObserver:editor
                                                 selector:@selector(pluginViewFrameChanged:)
                                                     name:NSViewFrameDidChangeNotification
                                                   object:view];
    }
    editors()[@(editor.editorId)] = editor;
    [window orderFrontRegardless];
    publish_capture(editor);
    return editor.editorId;
}

extern "C" {

void ag_ui_run(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        install_window_at_point_hook();
        // Audio streaming must not be throttled by App Nap or timer coalescing.
        static id activity = [[NSProcessInfo processInfo]
            beginActivityWithOptions:(NSActivityUserInitiated | NSActivityLatencyCritical)
                              reason:@"OpenGridder real-time audio streaming"];
        (void)activity;
        g_loop_running.store(true);
        [NSApp run];
    }
}

bool ag_ui_loop_running(void) { return g_loop_running.load(); }

// Runs fn(ctx) on the main thread and waits. Plugins built on JUCE tie their message
// handling and timers to the thread that creates them, so instances are created and
// destroyed there (AudioGridder does the same). Inline when no loop runs (tests, CLI).
void ag_ui_main_call(void (*fn)(void*), void* ctx) {
    run_on_main(^{ fn(ctx); });
}

uint64_t ag_ui_open_au(void* unit, const char* title, int x, int y) {
    if (!unit) return 0;
    __block uint64_t result = 0;
    run_on_main(^{
        AudioUnit au = (AudioUnit)unit;
        NSView* view = au_cocoa_view(au);
        if (!view) view = au_view_controller_view(au);
        if (!view) {
            AUGenericView* generic = [[AUGenericView alloc] initWithAudioUnit:au];
            generic.showsExpertParameters = YES;
            view = generic;
        }
        if (view) result = show_editor(view, NULL, title, x, y);
    });
    return result;
}

uint64_t ag_ui_open_native(void* native, const char* title, int x, int y) {
    if (!native) return 0;
    __block uint64_t result = 0;
    run_on_main(^{
        NSView* container = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 640, 420)];
        int width = 0, height = 0;
        if (ag_native_editor_open(native, (__bridge void*)container, &width, &height) == 0) return;
        if (width > 0 && height > 0) [container setFrameSize:NSMakeSize(width, height)];
        result = show_editor(container, native, title, x, y);
    });
    return result;
}

// VST3 IPlugFrame::resizeView and CLAP host gui request_resize land here.
void ag_ui_view_resized(void* nsview, int width, int height) {
    if (!nsview || width <= 0 || height <= 0) return;
    void (^work)(void) = ^{
        for (AGEditor* editor in editors().allValues) {
            if ((__bridge void*)editor.pluginView == nsview && !editor.closed) {
                [editor.pluginView setFrameSize:NSMakeSize(width, height)];
                [editor.window setContentSize:NSMakeSize(width, height)];
                publish_capture(editor);
                return;
            }
        }
        // Not shown yet (resize during attach): size the container itself.
        [(__bridge NSView*)nsview setFrameSize:NSMakeSize(width, height)];
    };
    if ([NSThread isMainThread]) work();
    else dispatch_async(dispatch_get_main_queue(), work);
}

void ag_ui_close(uint64_t editor_id) {
    if (editor_id == 0) return;
    run_on_main(^{
        AGEditor* editor = editors()[@(editor_id)];
        if (!editor) return;
        [editor teardown];
        editor.window.delegate = nil;
        [editor.window orderOut:nil];
        [editor.window close];
        [editors() removeObjectForKey:@(editor_id)];
    });
    std::lock_guard<std::mutex> lock(g_capture_mutex);
    captures().erase(editor_id);
}

bool ag_ui_is_open(uint64_t editor_id) {
    std::lock_guard<std::mutex> lock(g_capture_mutex);
    auto it = captures().find(editor_id);
    return it != captures().end() && it->second.open;
}

bool ag_ui_size(uint64_t editor_id, int* width, int* height) {
    std::lock_guard<std::mutex> lock(g_capture_mutex);
    auto it = captures().find(editor_id);
    if (it == captures().end() || !it->second.open) return false;
    *width = it->second.width;
    *height = it->second.height;
    return true;
}

// 1: RGBA written, size in width/height. -1: buffer too small, needed size in
// width/height. 0: nothing captured.
int ag_ui_capture(uint64_t editor_id, uint8_t* buffer, size_t capacity, int* width, int* height) {
    CaptureInfo info;
    {
        std::lock_guard<std::mutex> lock(g_capture_mutex);
        auto it = captures().find(editor_id);
        if (it == captures().end() || !it->second.open) return 0;
        info = it->second;
    }
    if (CGRectIsEmpty(info.rect)) return 0;
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    CGImageRef image = CGWindowListCreateImage(info.rect, kCGWindowListOptionIncludingWindow, info.window,
                                               kCGWindowImageBoundsIgnoreFraming | kCGWindowImageNominalResolution);
#pragma clang diagnostic pop
    if (!image) return 0;
    const size_t w = CGImageGetWidth(image), h = CGImageGetHeight(image);
    *width = (int)w;
    *height = (int)h;
    if (w == 0 || h == 0) {
        CGImageRelease(image);
        return 0;
    }
    if (w * h * 4 > capacity || !buffer) {
        CGImageRelease(image);
        return -1;
    }
    CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef ctx = CGBitmapContextCreate(buffer, w, h, 8, w * 4, space,
                                             kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(space);
    if (!ctx) {
        CGImageRelease(image);
        return 0;
    }
    CGContextSetRGBFillColor(ctx, 0.11, 0.11, 0.12, 1.0);
    CGContextFillRect(ctx, CGRectMake(0, 0, w, h));
    CGContextDrawImage(ctx, CGRectMake(0, 0, w, h), image);
    CGContextRelease(ctx);
    CGImageRelease(image);
    return 1;
}

// PNG (lossless, keeps alpha) or JPEG (quality 0..1, opaque) of an RGBA buffer.
// Returns a malloc'd buffer the caller frees with ag_ui_free.
static uint8_t* encode_image(const uint8_t* rgba, int width, int height, CFStringRef type, double quality, size_t* out_len);

uint8_t* ag_ui_encode_png(const uint8_t* rgba, int width, int height, size_t* out_len) {
    return encode_image(rgba, width, height, CFSTR("public.png"), -1.0, out_len);
}

uint8_t* ag_ui_encode_jpeg(const uint8_t* rgba, int width, int height, double quality, size_t* out_len) {
    return encode_image(rgba, width, height, CFSTR("public.jpeg"), quality, out_len);
}

static uint8_t* encode_image(const uint8_t* rgba, int width, int height, CFStringRef type, double quality, size_t* out_len) {
    *out_len = 0;
    if (!rgba || width <= 0 || height <= 0) return NULL;
    CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGDataProviderRef provider = CGDataProviderCreateWithData(NULL, rgba, (size_t)width * height * 4, NULL);
    CGImageRef image = CGImageCreate(width, height, 8, 32, (size_t)width * 4, space,
                                     kCGImageAlphaLast | kCGBitmapByteOrder32Big, provider, NULL, false,
                                     kCGRenderingIntentDefault);
    CGDataProviderRelease(provider);
    CGColorSpaceRelease(space);
    if (!image) return NULL;
    CFMutableDataRef data = CFDataCreateMutable(NULL, 0);
    CGImageDestinationRef dest = CGImageDestinationCreateWithData(data, type, 1, NULL);
    uint8_t* out = NULL;
    if (dest) {
        NSDictionary* props = quality >= 0 ? @{(__bridge NSString*)kCGImageDestinationLossyCompressionQuality: @(quality)} : nil;
        CGImageDestinationAddImage(dest, image, (__bridge CFDictionaryRef)props);
        if (CGImageDestinationFinalize(dest)) {
            const CFIndex len = CFDataGetLength(data);
            out = (uint8_t*)malloc((size_t)len);
            if (out) {
                memcpy(out, CFDataGetBytePtr(data), (size_t)len);
                *out_len = (size_t)len;
            }
        }
        CFRelease(dest);
    }
    CFRelease(data);
    CGImageRelease(image);
    return out;
}

void ag_ui_free(void* p) { free(p); }

}  // extern "C"

// --- Input ----------------------------------------------------------------------------

static NSEventModifierFlags modifier_flags(bool shift, bool ctrl, bool alt, bool cmd) {
    NSEventModifierFlags flags = 0;
    if (shift) flags |= NSEventModifierFlagShift;
    if (ctrl) flags |= NSEventModifierFlagControl;
    if (alt) flags |= NSEventModifierFlagOption;
    if (cmd) flags |= NSEventModifierFlagCommand;
    return flags;
}

// Mouse types as AudioGridder sends them: 0 move, 1-3 left down/up/drag, 4-6 right,
// 7-9 other, 10 wheel. (x, y) is relative to the top-left of the captured image.
static void deliver_mouse(AGEditor* editor, uint8_t type, float x, float y, NSEventModifierFlags flags, float dx, float dy) {
    static NSTimeInterval last_down = 0;
    static NSPoint last_point;
    static NSInteger clicks = 1, event_number = 0;
    NSWindow* window = editor.window;
    if (!window || editor.closed) return;
    const CGFloat content_height = [window contentRectForFrameRect:window.frame].size.height;
    NSPoint point = NSMakePoint(x, content_height - y);  // window coordinates, bottom-left origin
    NSTimeInterval now = [NSProcessInfo processInfo].systemUptime;

    if (type == 10) {
        CGEventRef cg = CGEventCreateScrollWheelEvent(NULL, kCGScrollEventUnitPixel, 2, (int32_t)lroundf(dy), (int32_t)lroundf(dx));
        if (!cg) return;
        CGEventSetFlags(cg, (CGEventFlags)flags);
        NSPoint screen_point = [window convertPointToScreen:point];
        CGFloat primary = NSMaxY([NSScreen screens].firstObject.frame);
        CGEventSetLocation(cg, CGPointMake(screen_point.x, primary - screen_point.y));
        NSEvent* scroll = [NSEvent eventWithCGEvent:cg];
        CFRelease(cg);
        NSView* hit = [window.contentView hitTest:[window.contentView convertPoint:point fromView:nil]] ?: window.contentView;
        [hit scrollWheel:scroll];
        return;
    }
    NSEventType event_type = NSEventTypeMouseMoved;
    bool down = false, up = false, drag = false;
    switch (type) {
        case 1: event_type = NSEventTypeLeftMouseDown; down = true; break;
        case 2: event_type = NSEventTypeLeftMouseUp; up = true; break;
        case 3: event_type = NSEventTypeLeftMouseDragged; drag = true; break;
        case 4: event_type = NSEventTypeRightMouseDown; down = true; break;
        case 5: event_type = NSEventTypeRightMouseUp; up = true; break;
        case 6: event_type = NSEventTypeRightMouseDragged; drag = true; break;
        case 7: event_type = NSEventTypeOtherMouseDown; down = true; break;
        case 8: event_type = NSEventTypeOtherMouseUp; up = true; break;
        case 9: event_type = NSEventTypeOtherMouseDragged; drag = true; break;
        default: break;
    }
    if (down) {
        clicks = (now - last_down < 0.4 && hypot(point.x - last_point.x, point.y - last_point.y) < 5) ? clicks + 1 : 1;
        last_down = now;
        last_point = point;
        if (!window.isKeyWindow) [window makeKeyWindow];
    }
    NSEvent* event = [NSEvent mouseEventWithType:event_type
                                        location:point
                                   modifierFlags:flags
                                       timestamp:now
                                    windowNumber:window.windowNumber
                                         context:nil
                                     eventNumber:++event_number
                                      clickCount:(down || up || drag) ? clicks : 0
                                        pressure:(down || drag) ? 1.0f : 0.0f];
    // Posted rather than sent, so controls that track the mouse in their own event loop
    // find the following drag and up events in the queue.
    [NSApp postEvent:event atStart:NO];
}

static NSString* key_characters(uint16_t key_code, NSEventModifierFlags flags) {
    TISInputSourceRef source = TISCopyCurrentKeyboardLayoutInputSource();
    if (!source) return @"";
    CFDataRef layout = (CFDataRef)TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
    NSString* result = @"";
    if (layout) {
        UInt32 dead = 0;
        UniChar chars[4];
        UniCharCount length = 0;
        UInt32 modifiers = (flags & NSEventModifierFlagShift) ? ((shiftKey >> 8) & 0xFF) : 0;
        if (UCKeyTranslate((const UCKeyboardLayout*)CFDataGetBytePtr(layout), key_code, kUCKeyActionDown, modifiers,
                           LMGetKbdType(), kUCKeyTranslateNoDeadKeysBit, &dead, 4, &length, chars) == noErr &&
            length > 0) {
            result = [NSString stringWithCharacters:chars length:length];
        }
    }
    CFRelease(source);
    return result;
}

extern "C" {

void ag_ui_mouse(uint64_t editor_id, uint8_t type, float x, float y, bool shift, bool ctrl, bool alt, float dx, float dy) {
    dispatch_async(dispatch_get_main_queue(), ^{
        AGEditor* editor = editors()[@(editor_id)];
        if (editor) deliver_mouse(editor, type, x, y, modifier_flags(shift, ctrl, alt, false), dx, dy);
    });
}

void ag_ui_key(uint64_t editor_id, uint16_t key_code, bool shift, bool ctrl, bool alt, bool cmd) {
    dispatch_async(dispatch_get_main_queue(), ^{
        AGEditor* editor = editors()[@(editor_id)];
        if (!editor || editor.closed) return;
        NSWindow* window = editor.window;
        if (!window.isKeyWindow) [window makeKeyWindow];
        NSEventModifierFlags flags = modifier_flags(shift, ctrl, alt, cmd);
        NSString* chars = key_characters(key_code, flags);
        NSTimeInterval now = [NSProcessInfo processInfo].systemUptime;
        for (NSEventType type : {NSEventTypeKeyDown, NSEventTypeKeyUp}) {
            NSEvent* event = [NSEvent keyEventWithType:type
                                              location:NSZeroPoint
                                         modifierFlags:flags
                                             timestamp:now
                                          windowNumber:window.windowNumber
                                               context:nil
                                            characters:chars
                           charactersIgnoringModifiers:chars
                                             isARepeat:NO
                                               keyCode:key_code];
            [NSApp postEvent:event atStart:NO];
        }
    });
}

}  // extern "C"
