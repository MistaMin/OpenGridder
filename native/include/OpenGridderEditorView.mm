#import "OpenGridderEditorView.h"
#import <objc/message.h>
#include <cmath>
#include <algorithm>

static NSFont *SysFont(CGFloat size, NSFontWeight weight) {
    if ([NSFont respondsToSelector:@selector(systemFontOfSize:weight:)]) {
        return ((NSFont *(*)(id, SEL, CGFloat, CGFloat))objc_msgSend)([NSFont class], @selector(systemFontOfSize:weight:), size, weight);
    }
    if (weight >= 0.4) {
        return [NSFont boldSystemFontOfSize:size];
    }
    return [NSFont systemFontOfSize:size];
}

static NSFont *MonoFont(CGFloat size, NSFontWeight weight) {
    if ([NSFont respondsToSelector:@selector(monospacedDigitSystemFontOfSize:weight:)]) {
        return ((NSFont *(*)(id, SEL, CGFloat, CGFloat))objc_msgSend)([NSFont class], @selector(monospacedDigitSystemFontOfSize:weight:), size, weight);
    }
    return [NSFont userFixedPitchFontOfSize:size];
}

@implementation OpenGridderEditorView

@synthesize rust = _rust;
@synthesize rustProvider = _rustProvider;

- (NativePlugin *)rust {
    if (_rustProvider) {
        return _rustProvider();
    }
    return _rust;
}
@synthesize isBypassed = _isBypassed;
@synthesize activeProfile = _activeProfile;
@synthesize activeSlot = _activeSlot;
@synthesize oversamplingFactor = _oversamplingFactor;
@synthesize inputGain = _inputGain;
@synthesize wetDryMix = _wetDryMix;
@synthesize drive = _drive;
@synthesize outputGain = _outputGain;
@synthesize onParameterChanged = _onParameterChanged;
@synthesize onProfileChanged = _onProfileChanged;

- (instancetype)initWithFrame:(NSRect)frame {
    self = [super initWithFrame:frame];
    if (self) {
        _isBypassed = NO;
        _activeProfile = 1; // Standard 256
        _activeSlot = 0;
        _oversamplingFactor = 1;
        _inputGain = 0.5f;   // 0 dB
        _wetDryMix = 1.0f;   // 100%
        _drive = 0.2f;       // 2.0
        _outputGain = 0.5f;  // 0 dB
        _draggedKnob = -1;
        _statusCode = 0;
        _latencySamples = 256;
        _meterLeft = 0.65f;
        _meterRight = 0.63f;

        // Periodic display refresh for meters
        _meterTimer = [NSTimer scheduledTimerWithTimeInterval:1.0 / 30.0 target:self selector:@selector(updateMeters) userInfo:nil repeats:YES];
    }
    return self;
}

- (void)dealloc {
    [_meterTimer invalidate];
    _meterTimer = nil;
#if !__has_feature(objc_arc)
    [super dealloc];
#endif
}

- (BOOL)isFlipped {
    return NO;
}

- (void)updateStatus:(int)statusCode latencySamples:(uint32_t)latency {
    _statusCode = statusCode;
    _latencySamples = latency;
    [self setNeedsDisplay:YES];
}

- (void)updateMeters {
    // Subtle realistic meter decay / activity
    if (_isBypassed) {
        _meterLeft = std::max(0.0f, _meterLeft - 0.05f);
        _meterRight = std::max(0.0f, _meterRight - 0.05f);
    } else {
        float baseL = 0.60f + 0.08f * std::sin((float)CFAbsoluteTimeGetCurrent() * 4.0f);
        float baseR = 0.58f + 0.07f * std::cos((float)CFAbsoluteTimeGetCurrent() * 3.5f);
        _meterLeft = baseL;
        _meterRight = baseR;
    }
    [self setNeedsDisplay:YES];
}

#pragma mark - Drawing Helpers

static void DrawHardwareScrew(CGContextRef ctx, CGFloat x, CGFloat y) {
    CGContextSetRGBFillColor(ctx, 0.05, 0.05, 0.06, 0.7);
    CGContextFillEllipseInRect(ctx, CGRectMake(x - 6, y - 6, 12, 12));
    CGContextSetRGBFillColor(ctx, 0.26, 0.28, 0.31, 1.0);
    CGContextFillEllipseInRect(ctx, CGRectMake(x - 5, y - 5, 10, 10));
    CGContextSetRGBStrokeColor(ctx, 0.45, 0.50, 0.55, 0.8);
    CGContextSetLineWidth(ctx, 1.0);
    CGContextStrokeEllipseInRect(ctx, CGRectMake(x - 5, y - 5, 10, 10));
    CGContextSetRGBStrokeColor(ctx, 0.12, 0.13, 0.15, 1.0);
    CGContextSetLineWidth(ctx, 1.2);
    CGContextMoveToPoint(ctx, x - 3, y);
    CGContextAddLineToPoint(ctx, x + 3, y);
    CGContextStrokePath(ctx);
    CGContextMoveToPoint(ctx, x, y - 3);
    CGContextAddLineToPoint(ctx, x, y + 3);
    CGContextStrokePath(ctx);
}

static void DrawGoodLookinKnob(CGContextRef ctx, CGRect bounds, float normVal, NSString *name, NSString *valStr) {
    CGFloat cx = CGRectGetMidX(bounds);
    CGFloat cy = CGRectGetMidY(bounds) + 4.0;
    CGFloat radius = bounds.size.width * 0.32;

    // Contact shadow
    CGContextSetRGBFillColor(ctx, 0.0, 0.0, 0.0, 0.55);
    CGContextFillEllipseInRect(ctx, CGRectMake(cx - radius - 3, cy - radius - 5, (radius + 3) * 2, (radius + 3) * 2));

    // Outer calibrated tick marks
    const int numTicks = 11;
    for (int i = 0; i < numTicks; ++i) {
        float t = (float)i / (numTicks - 1);
        float angle = -135.0f + t * 270.0f;
        float rad = angle * M_PI / 180.0f;
        CGFloat tickR1 = radius + 6.0;
        CGFloat tickR2 = radius + (i % 2 == 0 ? 11.0 : 8.0);
        CGFloat x1 = cx + tickR1 * std::cos(rad);
        CGFloat y1 = cy + tickR1 * std::sin(rad);
        CGFloat x2 = cx + tickR2 * std::cos(rad);
        CGFloat y2 = cy + tickR2 * std::sin(rad);

        CGContextSetRGBStrokeColor(ctx, 0.55, 0.60, 0.65, (i <= normVal * (numTicks - 1) + 0.1) ? 0.9 : 0.35);
        CGContextSetLineWidth(ctx, i % 2 == 0 ? 1.5 : 1.0);
        CGContextMoveToPoint(ctx, x1, y1);
        CGContextAddLineToPoint(ctx, x2, y2);
        CGContextStrokePath(ctx);
    }

    // Fluted metallic knob cap
    CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
    CGFloat capTop[4] = {0.32, 0.35, 0.39, 1.0};
    CGFloat capBot[4] = {0.16, 0.18, 0.20, 1.0};
    CGFloat capColors[8] = {capTop[0], capTop[1], capTop[2], capTop[3], capBot[0], capBot[1], capBot[2], capBot[3]};
    CGGradientRef capGrad = CGGradientCreateWithColorComponents(cs, capColors, NULL, 2);
    CGContextSaveGState(ctx);
    CGContextAddEllipseInRect(ctx, CGRectMake(cx - radius, cy - radius, radius * 2, radius * 2));
    CGContextClip(ctx);
    CGContextDrawLinearGradient(ctx, capGrad, CGPointMake(cx, cy + radius), CGPointMake(cx, cy - radius), 0);
    CGGradientRelease(capGrad);
    CGContextRestoreGState(ctx);

    // Bevel ring
    CGContextSetRGBStrokeColor(ctx, 0.55, 0.60, 0.65, 0.75);
    CGContextSetLineWidth(ctx, 1.2);
    CGContextStrokeEllipseInRect(ctx, CGRectMake(cx - radius, cy - radius, radius * 2, radius * 2));

    // Pointer notch
    float curAngle = (-135.0f + normVal * 270.0f) * M_PI / 180.0f;
    CGFloat ptrX1 = cx + (radius * 0.45) * std::cos(curAngle);
    CGFloat ptrY1 = cy + (radius * 0.45) * std::sin(curAngle);
    CGFloat ptrX2 = cx + (radius * 0.90) * std::cos(curAngle);
    CGFloat ptrY2 = cy + (radius * 0.90) * std::sin(curAngle);

    // Indicator glow
    CGContextSetRGBStrokeColor(ctx, 0.95, 0.75, 0.25, 1.0); // Amber needle
    CGContextSetLineWidth(ctx, 2.2);
    CGContextMoveToPoint(ctx, ptrX1, ptrY1);
    CGContextAddLineToPoint(ctx, ptrX2, ptrY2);
    CGContextStrokePath(ctx);

    // Center metallic cap
    CGFloat innerR = radius * 0.40;
    CGFloat innerTop[4] = {0.20, 0.22, 0.25, 1.0};
    CGFloat innerBot[4] = {0.10, 0.11, 0.12, 1.0};
    CGFloat innerColors[8] = {innerTop[0], innerTop[1], innerTop[2], innerTop[3], innerBot[0], innerBot[1], innerBot[2], innerBot[3]};
    CGGradientRef innerGrad = CGGradientCreateWithColorComponents(cs, innerColors, NULL, 2);
    CGContextSaveGState(ctx);
    CGContextAddEllipseInRect(ctx, CGRectMake(cx - innerR, cy - innerR, innerR * 2, innerR * 2));
    CGContextClip(ctx);
    CGContextDrawLinearGradient(ctx, innerGrad, CGPointMake(cx, cy + innerR), CGPointMake(cx, cy - innerR), 0);
    CGGradientRelease(innerGrad);
    CGContextRestoreGState(ctx);
    CGColorSpaceRelease(cs);

    // Labels
    NSDictionary *nameAttrs = @{
        NSFontAttributeName: SysFont(11, NSFontWeightSemibold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.90 alpha:1.0]
    };
    NSSize nameSize = [name sizeWithAttributes:nameAttrs];
    [name drawAtPoint:NSMakePoint(cx - nameSize.width * 0.5, bounds.origin.y + bounds.size.height - 18) withAttributes:nameAttrs];

    NSDictionary *valAttrs = @{
        NSFontAttributeName: MonoFont(10, NSFontWeightMedium),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.95 green:0.75 blue:0.25 alpha:1.0]
    };
    NSSize valSize = [valStr sizeWithAttributes:valAttrs];
    [valStr drawAtPoint:NSMakePoint(cx - valSize.width * 0.5, bounds.origin.y + 2) withAttributes:valAttrs];
}

#pragma mark - Main Drawing

- (void)drawRect:(NSRect)dirtyRect {
    [super drawRect:dirtyRect];
    NSGraphicsContext *nsCtx = [NSGraphicsContext currentContext];
    CGContextRef ctx = NULL;
    if ([nsCtx respondsToSelector:@selector(CGContext)]) {
        ctx = ((CGContextRef(*)(id, SEL))objc_msgSend)(nsCtx, @selector(CGContext));
    } else {
        ctx = (CGContextRef)[nsCtx graphicsPort];
    }
    if (!ctx) return;
    CGRect bounds = NSRectToCGRect(self.bounds);

    // 1. Dark brushed chassis background
    CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
    CGFloat topColor[4] = {0.14, 0.16, 0.18, 1.0};
    CGFloat botColor[4] = {0.08, 0.09, 0.10, 1.0};
    CGFloat colors[8] = {topColor[0], topColor[1], topColor[2], topColor[3], botColor[0], botColor[1], botColor[2], botColor[3]};
    CGGradientRef grad = CGGradientCreateWithColorComponents(cs, colors, NULL, 2);
    CGContextDrawLinearGradient(ctx, grad, CGPointMake(0, bounds.size.height), CGPointMake(0, 0), 0);
    CGGradientRelease(grad);
    CGColorSpaceRelease(cs);

    // Bevel outer frame
    CGContextSetRGBStrokeColor(ctx, 0.32, 0.36, 0.40, 0.6);
    CGContextSetLineWidth(ctx, 1.5);
    CGContextStrokeRect(ctx, CGRectInset(bounds, 1.5, 1.5));

    // Corner rack screws
    DrawHardwareScrew(ctx, 14, 14);
    DrawHardwareScrew(ctx, bounds.size.width - 14, 14);
    DrawHardwareScrew(ctx, 14, bounds.size.height - 14);
    DrawHardwareScrew(ctx, bounds.size.width - 14, bounds.size.height - 14);

    // 2. Left Panel: Rack & Navigation Controls (x = 24..260)
    CGRect leftPanel = CGRectMake(24, 40, 240, bounds.size.height - 64);
    CGContextSetRGBFillColor(ctx, 0.11, 0.12, 0.14, 0.9);
    CGContextFillRect(ctx, leftPanel);
    CGContextSetRGBStrokeColor(ctx, 0.28, 0.31, 0.35, 0.6);
    CGContextSetLineWidth(ctx, 1.0);
    CGContextStrokeRect(ctx, leftPanel);

    // Left Panel Header
    NSDictionary *brandAttrs = @{
        NSFontAttributeName: SysFont(15, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.96 alpha:1.0]
    };
    [@"OPENGRIDDER" drawAtPoint:NSMakePoint(40, bounds.size.height - 70) withAttributes:brandAttrs];

    // Connection LED Dot
    BOOL connected = (_statusCode >= 0);
    CGFloat ledX = 230, ledY = bounds.size.height - 62;
    CGContextSetRGBFillColor(ctx, connected ? 0.3 : 0.9, connected ? 0.9 : 0.3, 0.3, 1.0);
    CGContextFillEllipseInRect(ctx, CGRectMake(ledX - 4, ledY - 4, 8, 8));
    CGContextSetRGBStrokeColor(ctx, connected ? 0.5 : 1.0, 1.0, 0.5, 0.6);
    CGContextSetLineWidth(ctx, 2.0);
    CGContextStrokeEllipseInRect(ctx, CGRectMake(ledX - 6, ledY - 6, 12, 12));

    // Host status label
    NSDictionary *hostAttrs = @{
        NSFontAttributeName: MonoFont(10, NSFontWeightMedium),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.95 green:0.75 blue:0.25 alpha:1.0]
    };
    [@"Server: 127.0.0.1:55056" drawAtPoint:NSMakePoint(40, bounds.size.height - 88) withAttributes:hostAttrs];

    // Profile Section Label
    NSDictionary *sectionAttrs = @{
        NSFontAttributeName: SysFont(10, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.65 alpha:1.0]
    };
    [@"LATENCY PROFILE (PDC)" drawAtPoint:NSMakePoint(40, bounds.size.height - 114) withAttributes:sectionAttrs];

    // 4 Latency Profile Buttons
    NSArray *profileLabels = @[@"LOW (128)", @"STD (256)", @"HIGH (512)", @"STUDIO (1024)"];
    for (int i = 0; i < 4; ++i) {
        CGFloat bx = 40 + (i % 2) * 105;
        CGFloat by = bounds.size.height - 146 - (i / 2) * 28;
        CGRect bRect = CGRectMake(bx, by, 100, 24);

        BOOL active = (_activeProfile == i);
        CGContextSetRGBFillColor(ctx, active ? 0.25 : 0.15, active ? 0.32 : 0.17, active ? 0.38 : 0.19, 1.0);
        CGContextFillRect(ctx, bRect);
        CGContextSetRGBStrokeColor(ctx, active ? 0.95 : 0.35, active ? 0.75 : 0.38, active ? 0.25 : 0.42, 1.0);
        CGContextSetLineWidth(ctx, active ? 1.5 : 1.0);
        CGContextStrokeRect(ctx, bRect);

        NSDictionary *btnAttrs = @{
            NSFontAttributeName: MonoFont(9, active ? NSFontWeightBold : NSFontWeightRegular),
            NSForegroundColorAttributeName: active ? [NSColor whiteColor] : [NSColor colorWithCalibratedWhite:0.75 alpha:1.0]
        };
        NSString *txt = profileLabels[i];
        NSSize sz = [txt sizeWithAttributes:btnAttrs];
        [txt drawAtPoint:NSMakePoint(bx + (100 - sz.width) * 0.5, by + 5) withAttributes:btnAttrs];
    }

    // Plugin Slots Header
    [@"REMOTE PLUGIN SLOTS" drawAtPoint:NSMakePoint(40, bounds.size.height - 198) withAttributes:sectionAttrs];

    // Dynamic Plugin Slots with Per-Plugin Bypass, Oversampling, and Unload
    uint32_t numLoaded = self.rust ? og_native_num_loaded_plugins(self.rust) : 0;
    CGFloat slotH = 26;
    CGFloat slotSpacing = 4;
    CGFloat baseY = bounds.size.height - 230;

    for (uint32_t i = 0; i < numLoaded; ++i) {
        CGFloat sy = baseY - i * (slotH + slotSpacing);
        CGRect slotRect = CGRectMake(40, sy, 208, slotH);
        BOOL selected = (_activeSlot == i);

        CGContextSetRGBFillColor(ctx, selected ? 0.20 : 0.14, selected ? 0.25 : 0.16, selected ? 0.28 : 0.17, 1.0);
        CGContextFillRect(ctx, slotRect);
        CGContextSetRGBStrokeColor(ctx, selected ? 0.45 : 0.28, selected ? 0.65 : 0.30, selected ? 0.85 : 0.34, 1.0);
        CGContextSetLineWidth(ctx, 1.0);
        CGContextStrokeRect(ctx, slotRect);

        char nameBuf[256] = {0};
        bool bypassed = false;
        uint32_t osFactor = 1;
        og_native_get_loaded_plugin(self.rust, i, nameBuf, sizeof(nameBuf), &bypassed, &osFactor);
        NSString *slotTitle = [NSString stringWithUTF8String:nameBuf];
        if (slotTitle.length == 0) slotTitle = [NSString stringWithFormat:@"Plugin %u", i];

        // 1. Per-Plugin Bypass Button [B]
        CGRect bypBadge = CGRectMake(slotRect.origin.x + 3, sy + 3, 20, 20);
        CGContextSetRGBFillColor(ctx, bypassed ? 0.85 : 0.12, bypassed ? 0.45 : 0.14, bypassed ? 0.15 : 0.16, 1.0);
        CGContextFillRect(ctx, bypBadge);
        CGContextSetRGBStrokeColor(ctx, bypassed ? 1.0 : 0.4, bypassed ? 0.7 : 0.4, bypassed ? 0.3 : 0.4, 1.0);
        CGContextStrokeRect(ctx, bypBadge);

        NSDictionary *bBadgeAttrs = @{
            NSFontAttributeName: SysFont(9, NSFontWeightBold),
            NSForegroundColorAttributeName: bypassed ? [NSColor blackColor] : [NSColor colorWithCalibratedWhite:0.6 alpha:1.0]
        };
        NSSize bSz = [@"B" sizeWithAttributes:bBadgeAttrs];
        [@"B" drawAtPoint:NSMakePoint(bypBadge.origin.x + (20 - bSz.width) * 0.5, bypBadge.origin.y + 4) withAttributes:bBadgeAttrs];

        // 2. Plugin Title
        NSDictionary *slotAttrs = @{
            NSFontAttributeName: SysFont(10, NSFontWeightMedium),
            NSForegroundColorAttributeName: bypassed ? [NSColor colorWithCalibratedWhite:0.5 alpha:1.0] : [NSColor whiteColor]
        };
        // Truncate title if long
        if (slotTitle.length > 15) {
            slotTitle = [NSString stringWithFormat:@"%@…", [slotTitle substringToIndex:14]];
        }
        [slotTitle drawAtPoint:NSMakePoint(67, sy + 6) withAttributes:slotAttrs];

        // 3. Per-Plugin Oversampling Badge (1x / 2x / 4x)
        CGRect osBadge = CGRectMake(slotRect.origin.x + slotRect.size.width - 50, sy + 3, 24, 20);
        CGContextSetRGBFillColor(ctx, 0.12, 0.14, 0.16, 1.0);
        CGContextFillRect(ctx, osBadge);
        CGContextSetRGBStrokeColor(ctx, (osFactor > 1) ? 0.95 : 0.5, (osFactor > 1) ? 0.75 : 0.5, (osFactor > 1) ? 0.25 : 0.5, 0.8);
        CGContextSetLineWidth(ctx, 1.0);
        CGContextStrokeRect(ctx, osBadge);

        NSString *osText = [NSString stringWithFormat:@"%ux", osFactor];
        NSDictionary *osAttrs = @{
            NSFontAttributeName: MonoFont(8, NSFontWeightBold),
            NSForegroundColorAttributeName: (osFactor > 1) ? [NSColor colorWithCalibratedRed:0.95 green:0.75 blue:0.25 alpha:1.0] : [NSColor colorWithCalibratedWhite:0.65 alpha:1.0]
        };
        NSSize osSz = [osText sizeWithAttributes:osAttrs];
        [osText drawAtPoint:NSMakePoint(osBadge.origin.x + (24 - osSz.width) * 0.5, osBadge.origin.y + 4) withAttributes:osAttrs];

        // 4. Unload / Delete Button [✕]
        CGRect delBadge = CGRectMake(slotRect.origin.x + slotRect.size.width - 23, sy + 3, 20, 20);
        CGContextSetRGBFillColor(ctx, 0.18, 0.12, 0.12, 1.0);
        CGContextFillRect(ctx, delBadge);
        CGContextSetRGBStrokeColor(ctx, 0.6, 0.25, 0.25, 0.8);
        CGContextStrokeRect(ctx, delBadge);

        NSDictionary *delAttrs = @{
            NSFontAttributeName: SysFont(9, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.85 green:0.4 blue:0.4 alpha:1.0]
        };
        NSSize dSz = [@"✕" sizeWithAttributes:delAttrs];
        [@"✕" drawAtPoint:NSMakePoint(delBadge.origin.x + (20 - dSz.width) * 0.5, delBadge.origin.y + 4) withAttributes:delAttrs];
    }

    // Add Plugin Button [+]
    CGFloat addSy = baseY - numLoaded * (slotH + slotSpacing);
    if (addSy >= 90) {
        CGRect addRect = CGRectMake(40, addSy, 208, slotH);
        CGContextSetRGBFillColor(ctx, 0.12, 0.14, 0.16, 0.8);
        CGContextFillRect(ctx, addRect);
        CGContextSetRGBStrokeColor(ctx, 0.35, 0.45, 0.55, 0.8);
        CGFloat dash[2] = {4.0, 3.0};
        CGContextSetLineDash(ctx, 0, dash, 2);
        CGContextStrokeRect(ctx, addRect);
        CGContextSetLineDash(ctx, 0, NULL, 0); // Reset dash

        NSDictionary *addAttrs = @{
            NSFontAttributeName: SysFont(10, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.45 green:0.75 blue:0.95 alpha:1.0]
        };
        NSString *addTxt = @"+ Add Plugin...";
        NSSize addSz = [addTxt sizeWithAttributes:addAttrs];
        [addTxt drawAtPoint:NSMakePoint(addRect.origin.x + (208 - addSz.width) * 0.5, addSy + 6) withAttributes:addAttrs];
    }

    // Master Bypass Toggle
    CGRect bypRect = CGRectMake(40, 52, 208, 30);
    CGContextSetRGBFillColor(ctx, _isBypassed ? 0.35 : 0.18, _isBypassed ? 0.15 : 0.28, _isBypassed ? 0.15 : 0.20, 1.0);
    CGContextFillRect(ctx, bypRect);
    CGContextSetRGBStrokeColor(ctx, _isBypassed ? 0.85 : 0.45, _isBypassed ? 0.35 : 0.75, _isBypassed ? 0.35 : 0.45, 1.0);
    CGContextSetLineWidth(ctx, 1.2);
    CGContextStrokeRect(ctx, bypRect);

    NSString *bypText = _isBypassed ? @"MASTER BYPASS: ENGAGED" : @"MASTER: ACTIVE (PROCESSING)";
    NSDictionary *bypAttrs = @{
        NSFontAttributeName: SysFont(11, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor whiteColor]
    };
    NSSize bypSz = [bypText sizeWithAttributes:bypAttrs];
    [bypText drawAtPoint:NSMakePoint(bypRect.origin.x + (208 - bypSz.width) * 0.5, bypRect.origin.y + 7) withAttributes:bypAttrs];

    // 3. Right Panel: Hardware Console Strip (x = 280..696)
    CGRect rightPanel = CGRectMake(280, 40, bounds.size.width - 304, bounds.size.height - 64);
    CGContextSetRGBFillColor(ctx, 0.10, 0.11, 0.13, 0.9);
    CGContextFillRect(ctx, rightPanel);
    CGContextSetRGBStrokeColor(ctx, 0.30, 0.33, 0.37, 0.6);
    CGContextSetLineWidth(ctx, 1.0);
    CGContextStrokeRect(ctx, rightPanel);

    // 4 GoodLookin Rotary Knobs in 2x2 Grid
    CGFloat knobW = (rightPanel.size.width - 90) * 0.5;
    CGFloat knobH = (rightPanel.size.height - 40) * 0.45;

    // Knob 0: Input Gain
    float inDb = (_inputGain - 0.5f) * 48.0f;
    NSString *inStr = [NSString stringWithFormat:@"%+.1f dB", inDb];
    DrawGoodLookinKnob(ctx, CGRectMake(rightPanel.origin.x + 20, rightPanel.origin.y + knobH + 15, knobW, knobH),
                       _inputGain, @"INPUT GAIN", inStr);

    // Knob 1: Wet/Dry Mix
    NSString *mixStr = [NSString stringWithFormat:@"%.0f %%", _wetDryMix * 100.0f];
    DrawGoodLookinKnob(ctx, CGRectMake(rightPanel.origin.x + knobW + 30, rightPanel.origin.y + knobH + 15, knobW, knobH),
                       _wetDryMix, @"WET / DRY MIX", mixStr);

    // Knob 2: Saturation Drive
    NSString *drvStr = [NSString stringWithFormat:@"%.1f", _drive * 10.0f];
    DrawGoodLookinKnob(ctx, CGRectMake(rightPanel.origin.x + 20, rightPanel.origin.y + 10, knobW, knobH),
                       _drive, @"DRIVE / SAT", drvStr);

    // Knob 3: Output Trim
    float outDb = (_outputGain - 0.5f) * 48.0f;
    NSString *outStr = [NSString stringWithFormat:@"%+.1f dB", outDb];
    DrawGoodLookinKnob(ctx, CGRectMake(rightPanel.origin.x + knobW + 30, rightPanel.origin.y + 10, knobW, knobH),
                       _outputGain, @"OUTPUT TRIM", outStr);

    // Dual Stereo VU Meters (x = right edge of right panel)
    CGFloat meterX = rightPanel.origin.x + rightPanel.size.width - 34;
    CGFloat meterY = rightPanel.origin.y + 18;
    CGFloat meterH = rightPanel.size.height - 36;

    void (^drawMeterBar)(CGFloat, float) = ^(CGFloat mx, float level) {
        CGRect mFrame = CGRectMake(mx, meterY, 10, meterH);
        CGContextSetRGBFillColor(ctx, 0.05, 0.06, 0.07, 1.0);
        CGContextFillRect(ctx, mFrame);
        CGContextSetRGBStrokeColor(ctx, 0.35, 0.38, 0.42, 0.8);
        CGContextSetLineWidth(ctx, 1.0);
        CGContextStrokeRect(ctx, mFrame);

        // LED segments (Green, Yellow, Red)
        int segments = 24;
        int lit = (int)(level * segments);
        for (int s = 0; s < segments; ++s) {
            CGFloat sy = meterY + 2 + s * (meterH / segments);
            CGRect sRect = CGRectMake(mx + 2, sy, 6, (meterH / segments) - 1.5);
            if (s < lit) {
                if (s > segments - 4) {
                    CGContextSetRGBFillColor(ctx, 1.0, 0.25, 0.25, 1.0); // Clip Red
                } else if (s > segments - 9) {
                    CGContextSetRGBFillColor(ctx, 0.95, 0.75, 0.25, 1.0); // Amber
                } else {
                    CGContextSetRGBFillColor(ctx, 0.30, 0.85, 0.40, 1.0); // Green
                }
            } else {
                CGContextSetRGBFillColor(ctx, 0.15, 0.18, 0.20, 0.3); // Unlit
            }
            CGContextFillRect(ctx, sRect);
        }
    };

    drawMeterBar(meterX - 14, _meterLeft);
    drawMeterBar(meterX, _meterRight);

    // Footer Status Text
    NSDictionary *footerAttrs = @{
        NSFontAttributeName: MonoFont(10, NSFontWeightRegular),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.55 alpha:0.8]
    };
    NSString *footer = [NSString stringWithFormat:@"PDC Latency: %u smp (%.2f ms) • GoodLookinDri Hardware UI Engine",
                        _latencySamples, (double)_latencySamples / 44.1];
    [footer drawAtPoint:NSMakePoint(28, 14) withAttributes:footerAttrs];
}

#pragma mark - Mouse Interaction

- (void)mouseDown:(NSEvent *)event {
    NSPoint p = [self convertPoint:event.locationInWindow fromView:nil];
    CGRect bounds = NSRectToCGRect(self.bounds);
    CGRect rightPanel = CGRectMake(280, 40, bounds.size.width - 304, bounds.size.height - 64);
    CGFloat knobW = (rightPanel.size.width - 90) * 0.5;
    CGFloat knobH = (rightPanel.size.height - 40) * 0.45;

    // Check Knobs
    CGRect r0 = CGRectMake(rightPanel.origin.x + 20, rightPanel.origin.y + knobH + 15, knobW, knobH);
    CGRect r1 = CGRectMake(rightPanel.origin.x + knobW + 30, rightPanel.origin.y + knobH + 15, knobW, knobH);
    CGRect r2 = CGRectMake(rightPanel.origin.x + 20, rightPanel.origin.y + 10, knobW, knobH);
    CGRect r3 = CGRectMake(rightPanel.origin.x + knobW + 30, rightPanel.origin.y + 10, knobW, knobH);

    if (CGRectContainsPoint(r0, NSPointToCGPoint(p))) {
        _draggedKnob = 0; _dragStartPoint = p; _dragStartValue = _inputGain; return;
    }
    if (CGRectContainsPoint(r1, NSPointToCGPoint(p))) {
        _draggedKnob = 1; _dragStartPoint = p; _dragStartValue = _wetDryMix; return;
    }
    if (CGRectContainsPoint(r2, NSPointToCGPoint(p))) {
        _draggedKnob = 2; _dragStartPoint = p; _dragStartValue = _drive; return;
    }
    if (CGRectContainsPoint(r3, NSPointToCGPoint(p))) {
        _draggedKnob = 3; _dragStartPoint = p; _dragStartValue = _outputGain; return;
    }

    // Check Latency Profile Buttons
    for (int i = 0; i < 4; ++i) {
        CGFloat bx = 40 + (i % 2) * 105;
        CGFloat by = bounds.size.height - 146 - (i / 2) * 28;
        CGRect bRect = CGRectMake(bx, by, 100, 24);
        if (CGRectContainsPoint(bRect, NSPointToCGPoint(p))) {
            _activeProfile = i;
            if (self.onProfileChanged) self.onProfileChanged(i);
            [self setNeedsDisplay:YES];
            return;
        }
    }

    // Dynamic Plugin Slots
    uint32_t numLoaded = self.rust ? og_native_num_loaded_plugins(self.rust) : 0;
    CGFloat slotH = 26;
    CGFloat slotSpacing = 4;
    CGFloat baseY = bounds.size.height - 230;

    for (uint32_t i = 0; i < numLoaded; ++i) {
        CGFloat sy = baseY - i * (slotH + slotSpacing);
        CGRect slotRect = CGRectMake(40, sy, 208, slotH);

        // 1. Bypass badge [B]
        CGRect bypBadge = CGRectMake(slotRect.origin.x + 3, sy + 3, 20, 20);
        if (CGRectContainsPoint(bypBadge, NSPointToCGPoint(p))) {
            char nameBuf[256] = {0};
            bool bypassed = false;
            uint32_t osFactor = 1;
            og_native_get_loaded_plugin(self.rust, i, nameBuf, sizeof(nameBuf), &bypassed, &osFactor);
            og_native_set_plugin_bypassed(self.rust, i, !bypassed);
            [self setNeedsDisplay:YES];
            return;
        }

        // 2. Oversampling badge [1x / 2x / 4x]
        CGRect osBadge = CGRectMake(slotRect.origin.x + slotRect.size.width - 50, sy + 3, 24, 20);
        if (CGRectContainsPoint(osBadge, NSPointToCGPoint(p))) {
            char nameBuf[256] = {0};
            bool bypassed = false;
            uint32_t osFactor = 1;
            og_native_get_loaded_plugin(self.rust, i, nameBuf, sizeof(nameBuf), &bypassed, &osFactor);
            uint32_t nextOs = (osFactor == 1) ? 2 : (osFactor == 2) ? 4 : 1;
            og_native_set_plugin_oversampling(self.rust, i, nextOs);
            [self setNeedsDisplay:YES];
            return;
        }

        // 3. Delete badge [✕]
        CGRect delBadge = CGRectMake(slotRect.origin.x + slotRect.size.width - 23, sy + 3, 20, 20);
        if (CGRectContainsPoint(delBadge, NSPointToCGPoint(p))) {
            og_native_unload_plugin(self.rust, i);
            [self setNeedsDisplay:YES];
            return;
        }

        // 4. Slot body click -> open remote plugin window
        if (CGRectContainsPoint(slotRect, NSPointToCGPoint(p))) {
            _activeSlot = i;
            NSWindow *win = self.window;
            NSPoint winPoint = [self convertPoint:p toView:nil];
            NSRect screenRect = [win convertRectToScreen:NSMakeRect(winPoint.x, winPoint.y, 1, 1)];
            og_native_edit_plugin(self.rust, i, (int32_t)screenRect.origin.x, (int32_t)screenRect.origin.y);
            [self setNeedsDisplay:YES];
            return;
        }
    }

    // Add Plugin Button [+]
    CGFloat addSy = baseY - numLoaded * (slotH + slotSpacing);
    if (addSy >= 90) {
        CGRect addRect = CGRectMake(40, addSy, 208, slotH);
        if (CGRectContainsPoint(addRect, NSPointToCGPoint(p))) {
            [self showPluginMenuAtPoint:p];
            return;
        }
    }

    // Check Bypass Toggle
    CGRect bypRect = CGRectMake(40, 52, 208, 30);
    if (CGRectContainsPoint(bypRect, NSPointToCGPoint(p))) {
        _isBypassed = !_isBypassed;
        if (self.rust) og_native_bypass(self.rust, _isBypassed);
        if (self.onParameterChanged) self.onParameterChanged(0, _isBypassed ? 1.0f : 0.0f);
        [self setNeedsDisplay:YES];
        return;
    }
}

- (void)mouseDragged:(NSEvent *)event {
    if (_draggedKnob < 0) return;
    NSPoint p = [self convertPoint:event.locationInWindow fromView:nil];
    CGFloat dy = p.y - _dragStartPoint.y;
    float delta = (float)dy / 150.0f;
    float newVal = std::max(0.0f, std::min(1.0f, _dragStartValue + delta));

    if (_draggedKnob == 0) {
        _inputGain = newVal;
    } else if (_draggedKnob == 1) {
        _wetDryMix = newVal;
    } else if (_draggedKnob == 2) {
        _drive = newVal;
    } else if (_draggedKnob == 3) {
        _outputGain = newVal;
    }

    [self setNeedsDisplay:YES];
}

- (void)mouseUp:(NSEvent *)event {
    _draggedKnob = -1;
}

- (void)showPluginMenuAtPoint:(NSPoint)pt {
    NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Available Plugins"];
    [menu setAutoenablesItems:NO];

    if (!self.rust) {
        NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:@"Connecting to server (127.0.0.1:55056)..." action:nil keyEquivalent:@""];
        [item setEnabled:NO];
        [menu addItem:item];
        [menu popUpMenuPositioningItem:nil atLocation:pt inView:self];
        return;
    }
    uint32_t numAvail = og_native_num_available_plugins(self.rust);

    if (numAvail == 0) {
        NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:@"No plugins available on server" action:nil keyEquivalent:@""];
        [item setEnabled:NO];
        [menu addItem:item];
    } else {
        NSMutableDictionary *categoryMenus = [NSMutableDictionary dictionary];

        for (uint32_t i = 0; i < numAvail; ++i) {
            char nameBuf[256] = {0};
            char idBuf[256] = {0};
            char catBuf[256] = {0};
            og_native_get_available_plugin(self.rust, i, nameBuf, sizeof(nameBuf), idBuf, sizeof(idBuf), catBuf, sizeof(catBuf));

            NSString *name = [NSString stringWithUTF8String:nameBuf];
            NSString *cat = [NSString stringWithUTF8String:catBuf];
            if (cat.length == 0) {
                cat = @"General";
            }

            NSMenu *targetMenu = categoryMenus[cat];
            if (!targetMenu) {
                NSMenuItem *catItem = [[NSMenuItem alloc] initWithTitle:cat action:nil keyEquivalent:@""];
                targetMenu = [[NSMenu alloc] initWithTitle:cat];
                [catItem setSubmenu:targetMenu];
                [menu addItem:catItem];
                categoryMenus[cat] = targetMenu;
            }

            NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:name action:@selector(onSelectPlugin:) keyEquivalent:@""];
            [item setTarget:self];
            [item setTag:(NSInteger)i];
            [targetMenu addItem:item];
        }
    }

    [menu popUpMenuPositioningItem:nil atLocation:pt inView:self];
}

- (void)onSelectPlugin:(NSMenuItem *)sender {
    if (!self.rust) return;
    uint32_t idx = (uint32_t)sender.tag;
    og_native_load_plugin(self.rust, idx);
    [self setNeedsDisplay:YES];
}

@end
