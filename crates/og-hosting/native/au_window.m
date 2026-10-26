#import <Cocoa/Cocoa.h>
#import <AudioToolbox/AudioToolbox.h>
#import <AudioToolbox/AUAudioUnit.h>
#import <AudioUnit/AUCocoaUIView.h>

static NSMapTable *g_auWindows = nil;

static void og_au_init_map(void) {
    if (!g_auWindows) {
        g_auWindows = [NSMapTable strongToStrongObjectsMapTable];
    }
}

bool og_au_show_window(AudioComponentInstance unit, const char *title_str, int x, int y, bool on_top) {
    if (!unit) return false;
    og_au_init_map();

    NSString *titleCopy = (title_str && title_str[0] != '\0')
        ? [NSString stringWithUTF8String:title_str]
        : @"Audio Unit Editor";
    if (!titleCopy) {
        titleCopy = @"Audio Unit Editor";
    }

    dispatch_async(dispatch_get_main_queue(), ^{
        NSValue *key = [NSValue valueWithPointer:(const void *)unit];
        NSWindow *existing = [g_auWindows objectForKey:key];
        if (existing) {
            if (on_top) {
                [existing setLevel:NSFloatingWindowLevel];
            } else {
                [existing setLevel:NSNormalWindowLevel];
            }
            if (x > 0 || y > 0) {
                [existing setFrameTopLeftPoint:NSMakePoint(x, y)];
            }
            [existing makeKeyAndOrderFront:nil];
            [NSApp activateIgnoringOtherApps:YES];
            return;
        }

        NSView *pluginView = nil;
        UInt32 dataSize = 0;
        Boolean isWritable = false;
        OSStatus status = AudioUnitGetPropertyInfo(unit,
                                                   kAudioUnitProperty_CocoaUI,
                                                   kAudioUnitScope_Global,
                                                   0,
                                                   &dataSize,
                                                   &isWritable);
        if (status == noErr && dataSize >= sizeof(AudioUnitCocoaViewInfo)) {
            AudioUnitCocoaViewInfo *info = (AudioUnitCocoaViewInfo *)malloc(dataSize);
            if (AudioUnitGetProperty(unit,
                                     kAudioUnitProperty_CocoaUI,
                                     kAudioUnitScope_Global,
                                     0,
                                     info,
                                     &dataSize) == noErr) {
                NSString *viewClassName = (__bridge NSString *)(info->mCocoaAUViewClass[0]);
                NSURL *bundleUrl = (__bridge NSURL *)(info->mCocoaAUViewBundleLocation);
                NSBundle *viewBundle = [NSBundle bundleWithURL:bundleUrl];
                Class viewClass = [viewBundle classNamed:viewClassName];
                if (viewClass && [viewClass instancesRespondToSelector:@selector(uiViewForAudioUnit:withSize:)]) {
                    id factory = [[viewClass alloc] init];
                    pluginView = [factory uiViewForAudioUnit:unit withSize:NSMakeSize(0, 0)];
                }
                for (int i = (dataSize - sizeof(CFURLRef)) / sizeof(CFStringRef); --i >= 0;) {
                    if (info->mCocoaAUViewClass[i]) CFRelease(info->mCocoaAUViewClass[i]);
                }
                if (info->mCocoaAUViewBundleLocation) CFRelease(info->mCocoaAUViewBundleLocation);
            }
            free(info);
        }

        if (!pluginView) {
            pluginView = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 480, 320)];
            NSTextField *lbl = [NSTextField labelWithString:[NSString stringWithFormat:@"%@ (Generic View)", titleCopy]];
            lbl.frame = NSMakeRect(20, 140, 440, 40);
            lbl.alignment = NSTextAlignmentCenter;
            lbl.font = [NSFont boldSystemFontOfSize:14];
            [pluginView addSubview:lbl];
        }

        NSRect frame = pluginView.frame;
        if (frame.size.width < 100 || frame.size.height < 100) {
            frame.size = NSMakeSize(640, 420);
        }
        NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                       styleMask:(NSWindowStyleMaskTitled |
                                                                  NSWindowStyleMaskClosable |
                                                                  NSWindowStyleMaskResizable)
                                                         backing:NSBackingStoreBuffered
                                                           defer:NO];
        [window setReleasedWhenClosed:NO];
        [window setTitle:titleCopy];
        [window setContentView:pluginView];
        if (x > 0 || y > 0) {
            [window setFrameTopLeftPoint:NSMakePoint(x, y)];
        } else {
            [window center];
        }
        if (on_top) [window setLevel:NSFloatingWindowLevel];
        [g_auWindows setObject:window forKey:key];
        [window makeKeyAndOrderFront:nil];
        [NSApp activateIgnoringOtherApps:YES];
    });

    return true;
}

void og_au_hide_window(AudioComponentInstance unit) {
    if (!unit || !g_auWindows) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        NSValue *key = [NSValue valueWithPointer:(const void *)unit];
        NSWindow *window = [g_auWindows objectForKey:key];
        if (window) {
            [window orderOut:nil];
        }
    });
}

void og_run_main_loop(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        [NSApp run];
    }
}

void og_stop_main_loop(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        [NSApp stop:nil];
        NSEvent *event = [NSEvent otherEventWithType:NSEventTypeApplicationDefined
                                            location:NSZeroPoint
                                       modifierFlags:0
                                           timestamp:0
                                        windowNumber:0
                                             context:nil
                                             subtype:0
                                               data1:0
                                               data2:0];
        [NSApp postEvent:event atStart:YES];
    });
}

void og_post_mouse_event(uint8_t ev_type, float x, float y, bool shift, bool ctrl, bool alt, float delta_x, float delta_y) {
    CGPoint loc = CGPointMake(x, y);
    CGEventFlags flags = 0;
    if (shift) flags |= kCGEventFlagMaskShift;
    if (ctrl) flags |= kCGEventFlagMaskControl;
    if (alt) flags |= kCGEventFlagMaskAlternate;

    if (ev_type == 10) { // Wheel
        CGEventRef ev = nil;
        if (delta_x != 0) {
            ev = CGEventCreateScrollWheelEvent(NULL, kCGScrollEventUnitPixel, 2, (int)lroundf(delta_y), (int)lroundf(delta_x));
        } else {
            ev = CGEventCreateScrollWheelEvent(NULL, kCGScrollEventUnitPixel, 1, (int)lroundf(delta_y));
        }
        if (ev) {
            CGEventPost(kCGSessionEventTap, ev);
            CFRelease(ev);
        }
        return;
    }

    CGEventType type = kCGEventMouseMoved;
    CGMouseButton btn = kCGMouseButtonLeft;
    switch (ev_type) {
        case 1: type = kCGEventLeftMouseDown; btn = kCGMouseButtonLeft; break;
        case 2: type = kCGEventLeftMouseUp; btn = kCGMouseButtonLeft; break;
        case 3: type = kCGEventLeftMouseDragged; btn = kCGMouseButtonLeft; break;
        case 4: type = kCGEventRightMouseDown; btn = kCGMouseButtonRight; break;
        case 5: type = kCGEventRightMouseUp; btn = kCGMouseButtonRight; break;
        case 6: type = kCGEventRightMouseDragged; btn = kCGMouseButtonRight; break;
        case 7: type = kCGEventOtherMouseDown; btn = kCGMouseButtonCenter; break;
        case 8: type = kCGEventOtherMouseUp; btn = kCGMouseButtonCenter; break;
        case 9: type = kCGEventOtherMouseDragged; btn = kCGMouseButtonCenter; break;
        default: type = kCGEventMouseMoved; btn = kCGMouseButtonLeft; break;
    }

    CGEventRef ev = CGEventCreateMouseEvent(NULL, type, loc, btn);
    if (ev) {
        CGEventSetFlags(ev, flags | CGEventGetFlags(ev));
        CGEventPost(kCGSessionEventTap, ev);
        CFRelease(ev);
    }
}

void og_post_key_event(uint16_t key_code, bool is_down) {
    CGEventRef ev = CGEventCreateKeyboardEvent(NULL, (CGKeyCode)key_code, is_down);
    if (ev) {
        CGEventPost(kCGSessionEventTap, ev);
        CFRelease(ev);
    }
}
