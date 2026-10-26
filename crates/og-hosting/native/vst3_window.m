// Cocoa window host for VST3 plugin editors (IPlugView with kPlatformTypeNSView).
#import <Cocoa/Cocoa.h>

extern int og_vst3_view_open(void *handle, int *width, int *height, void (*resize)(void *, int, int), void *user);
extern int og_vst3_view_attach(void *handle, void *nsview);
extern void og_vst3_view_close(void *handle);

static NSMutableDictionary<NSValue *, NSWindow *> *g_vstWindows;

static void og_vst3_resize(void *user, int w, int h) {
    NSWindow *window = (__bridge NSWindow *)user;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (w > 0 && h > 0) [window setContentSize:NSMakeSize(w, h)];
    });
}

bool og_vst3_show_window(void *handle, const char *title_str, int x, int y, bool on_top) {
    if (!handle) return false;
    if (!g_vstWindows) g_vstWindows = [NSMutableDictionary dictionary];
    NSString *title = title_str ? [NSString stringWithUTF8String:title_str] : @"VST3 Editor";
    if (!title) title = @"VST3 Editor";
    dispatch_async(dispatch_get_main_queue(), ^{
        NSValue *key = [NSValue valueWithPointer:handle];
        NSWindow *window = g_vstWindows[key];
        if (window) {
            [window setLevel:on_top ? NSFloatingWindowLevel : NSNormalWindowLevel];
            if (x > 0 || y > 0) [window setFrameTopLeftPoint:NSMakePoint(x, y)];
            [window makeKeyAndOrderFront:nil];
            [window orderFrontRegardless];
            [NSApp activateIgnoringOtherApps:YES];
            return;
        }
        window = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 640, 360)
                                             styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable)
                                               backing:NSBackingStoreBuffered
                                                 defer:NO];
        [window setReleasedWhenClosed:NO];
        [window setTitle:title];
        int w = 0, h = 0;
        if (og_vst3_view_open(handle, &w, &h, og_vst3_resize, (__bridge void *)window) != 0 || w < 1 || h < 1) {
            NSLog(@"[OpenGridder] VST3 plugin '%@' has no usable editor view", title);
            return;
        }
        NSView *container = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, w, h)];
        [window setContentView:container];
        [window setContentSize:NSMakeSize(w, h)];
        if (og_vst3_view_attach(handle, (__bridge void *)container) != 0) {
            NSLog(@"[OpenGridder] VST3 editor attach failed for '%@'", title);
            og_vst3_view_close(handle);
            return;
        }
        if (x > 0 || y > 0) [window setFrameTopLeftPoint:NSMakePoint(x, y)]; else [window center];
        if (on_top) [window setLevel:NSFloatingWindowLevel];
        g_vstWindows[key] = window;
        [window makeKeyAndOrderFront:nil];
        [window orderFrontRegardless];
        [NSApp activateIgnoringOtherApps:YES];
    });
    return true;
}

void og_vst3_hide_window(void *handle) {
    if (!handle || !g_vstWindows) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        [g_vstWindows[[NSValue valueWithPointer:handle]] orderOut:nil];
    });
}

// Must run before the plugin instance is destroyed: the view references the controller.
void og_vst3_destroy_window(void *handle) {
    if (!handle || !g_vstWindows) return;
    void (^work)(void) = ^{
        NSValue *key = [NSValue valueWithPointer:handle];
        NSWindow *window = g_vstWindows[key];
        if (window) {
            [window orderOut:nil];
            og_vst3_view_close(handle);
            [window setContentView:[[NSView alloc] initWithFrame:NSZeroRect]];
            [window close];
            [g_vstWindows removeObjectForKey:key];
        }
    };
    if ([NSThread isMainThread]) { work(); return; }
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    dispatch_async(dispatch_get_main_queue(), ^{ work(); dispatch_semaphore_signal(done); });
    dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
}
