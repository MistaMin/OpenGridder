#import "OpenGridderEditorView.h"
#import "OpenGridderRemoteView.h"
#import "OpenGridderRemoteView.mm"
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
@synthesize isVuMeterMode = _isVuMeterMode;
@synthesize isVuCalibratedMinus18 = _isVuCalibratedMinus18;
@synthesize isMeterInputMode = _isMeterInputMode;
@synthesize rackCollapsed = _rackCollapsed;
@synthesize consoleCollapsed = _consoleCollapsed;
@synthesize meterCollapsed = _meterCollapsed;
@synthesize onParameterChanged = _onParameterChanged;
@synthesize onProfileChanged = _onProfileChanged;
@synthesize onSizeChanged = _onSizeChanged;

- (instancetype)initWithFrame:(NSRect)frame {
    self = [super initWithFrame:frame];
    if (self) {
        _isBypassed = NO;
        _activeProfile = 1; // Standard 256
        _activeSlot = 0;
        _oversamplingFactor = 1;
        _inputGain = 0.5f;   // 0 dB
        _wetDryMix = 1.0f;   // 100%
        _drive = 0.0f;       // 0.0
        _outputGain = 0.5f;  // 0 dB
        _draggedKnob = -1;
        _statusCode = 0;
        _latencySamples = 256;
        _meterLeft = 0.0f;
        _meterRight = 0.0f;
        _vuLeft = 0.0f;
        _vuRight = 0.0f;
        _peakHoldLeft = 0.0f;
        _peakHoldRight = 0.0f;
        _isVuMeterMode = YES; // Default to classic 2x-wide VU meter mode (toggleable to 1x PPM)
        _isVuCalibratedMinus18 = YES; // Default: 0 VU = +4 dBu = -18 dBFS (toggleable to 0 VU = -3 dBFS)
        _isMeterInputMode = NO; // Default: Output metering (toggleable to Input metering in both PPM & VU)
        _rackCollapsed = NO;
        _consoleCollapsed = NO;
        _meterCollapsed = NO;
        _cachedPluginMenu = nil;
        _cachedPluginCount = 0;

        [self setFrameSize:[self preferredViewSize]];

        // Periodic display refresh for meters scheduled in CommonModes so it fires during tracking
        _meterTimer = [NSTimer timerWithTimeInterval:1.0 / 30.0 target:self selector:@selector(updateMeters) userInfo:nil repeats:YES];
        [[NSRunLoop mainRunLoop] addTimer:_meterTimer forMode:NSRunLoopCommonModes];
    }
    return self;
}

- (NSSize)preferredViewSize {
    CGFloat wRack = _rackCollapsed ? 36.0 : 256.0;
    CGFloat wConsole = _consoleCollapsed ? 36.0 : 272.0;
    // When switching to VU, the Metering Window is twice as wide (288px bank / 256px inner bay vs 144px bank / 128px inner bay)
    CGFloat wMeter = _meterCollapsed ? 36.0 : (_isVuMeterMode ? 288.0 : 144.0);
    CGFloat totalW = 16.0 + wRack + 8.0 + wConsole + 8.0 + wMeter + 16.0;
    return NSMakeSize(totalW, 420.0);
}

- (void)updateLayoutSize {
    NSSize sz = [self preferredViewSize];
    [self setFrameSize:sz];
    if (self.onSizeChanged) {
        self.onSizeChanged(sz.width, sz.height);
    }
    if (self.window) {
        if (self.superview == self.window.contentView) {
            [self.window setContentSize:sz];
        } else if (self.superview) {
            [self.superview setFrameSize:sz];
        }
    }
    [self setNeedsDisplay:YES];
}

- (void)setIsVuMeterMode:(BOOL)isVuMeterMode {
    if (_isVuMeterMode != isVuMeterMode) {
        _isVuMeterMode = isVuMeterMode;
        [self updateLayoutSize];
    }
}

- (void)setRackCollapsed:(BOOL)rackCollapsed {
    if (_rackCollapsed != rackCollapsed) {
        _rackCollapsed = rackCollapsed;
        [self updateLayoutSize];
    }
}

- (void)setConsoleCollapsed:(BOOL)consoleCollapsed {
    if (_consoleCollapsed != consoleCollapsed) {
        _consoleCollapsed = consoleCollapsed;
        [self updateLayoutSize];
    }
}

- (void)setMeterCollapsed:(BOOL)meterCollapsed {
    if (_meterCollapsed != meterCollapsed) {
        _meterCollapsed = meterCollapsed;
        [self updateLayoutSize];
    }
}

- (void)viewWillMoveToWindow:(NSWindow *)newWindow {
    [super viewWillMoveToWindow:newWindow];
    if (newWindow == nil) {
        [_meterTimer invalidate];
        _meterTimer = nil;
        [[OpenGridderRemoteWindowController sharedController] hide];
    } else if (_meterTimer == nil) {
        _meterTimer = [NSTimer timerWithTimeInterval:1.0 / 30.0 target:self selector:@selector(updateMeters) userInfo:nil repeats:YES];
        [[NSRunLoop mainRunLoop] addTimer:_meterTimer forMode:NSRunLoopCommonModes];
    }
}

- (void)dealloc {
    [_meterTimer invalidate];
    _meterTimer = nil;
    _cachedPluginMenu = nil;
#if !__has_feature(objc_arc)
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

- (void)updateStatus:(int)statusCode latencySamples:(uint32_t)latency {
    _statusCode = statusCode;
    _latencySamples = latency;
    [self setNeedsDisplay:YES];
}

- (void)updateMeters {
    NativePlugin *r = self.rust;
    if (r) {
        _statusCode = og_native_status(r);
        _latencySamples = og_native_latency(r);
        _activeProfile = (int)(og_native_get_param(r, 2) * 3.0f + 0.5f);
        if (_draggedKnob < 0) {
            _isBypassed = og_native_get_param(r, 0) >= 0.5f;
            _inputGain = og_native_get_param(r, 3);
            _wetDryMix = og_native_get_param(r, 4);
            _drive = og_native_get_param(r, 5);
            _outputGain = og_native_get_param(r, 6);
        }
        float rawL = 0.0f, rawR = 0.0f;
        if (_isMeterInputMode) {
            og_native_get_input_meters(r, &rawL, &rawR);
        } else {
            og_native_get_meters(r, &rawL, &rawR);
        }
        if (_isBypassed && !_isMeterInputMode) {
            _meterLeft = std::max(0.0f, _meterLeft * 0.85f);
            _meterRight = std::max(0.0f, _meterRight * 0.85f);
            _vuLeft = std::max(0.0f, _vuLeft * 0.82f);
            _vuRight = std::max(0.0f, _vuRight * 0.82f);
            _peakHoldLeft = std::max(0.0f, _peakHoldLeft * 0.94f);
            _peakHoldRight = std::max(0.0f, _peakHoldRight * 0.94f);
        } else {
            // PPM fast attack (~5ms), slow release
            _meterLeft = (rawL > _meterLeft) ? rawL : std::max(0.0f, _meterLeft * 0.88f);
            _meterRight = (rawR > _meterRight) ? rawR : std::max(0.0f, _meterRight * 0.88f);
            // Peak hold with gentle decay
            _peakHoldLeft = (rawL >= _peakHoldLeft) ? rawL : std::max(_meterLeft, _peakHoldLeft * 0.97f);
            _peakHoldRight = (rawR >= _peakHoldRight) ? rawR : std::max(_meterRight, _peakHoldRight * 0.97f);
            // True VU ~300ms integration ballistics
            _vuLeft = _vuLeft * 0.76f + rawL * 0.24f;
            _vuRight = _vuRight * 0.76f + rawR * 0.24f;
        }
    } else {
        _meterLeft = std::max(0.0f, _meterLeft * 0.85f);
        _meterRight = std::max(0.0f, _meterRight * 0.85f);
        _vuLeft = std::max(0.0f, _vuLeft * 0.82f);
        _vuRight = std::max(0.0f, _vuRight * 0.82f);
        _peakHoldLeft = std::max(0.0f, _peakHoldLeft * 0.94f);
        _peakHoldRight = std::max(0.0f, _peakHoldRight * 0.94f);
    }
    // Refresh dynamic Metering Bank, footer, and status LED
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

static void FillRoundedRect(CGContextRef ctx, CGRect rect, CGFloat radius) {
    CGMutablePathRef path = CGPathCreateMutable();
    CGPathMoveToPoint(path, NULL, CGRectGetMinX(rect) + radius, CGRectGetMinY(rect));
    CGPathAddArcToPoint(path, NULL, CGRectGetMaxX(rect), CGRectGetMinY(rect), CGRectGetMaxX(rect), CGRectGetMaxY(rect), radius);
    CGPathAddArcToPoint(path, NULL, CGRectGetMaxX(rect), CGRectGetMaxY(rect), CGRectGetMinX(rect), CGRectGetMaxY(rect), radius);
    CGPathAddArcToPoint(path, NULL, CGRectGetMinX(rect), CGRectGetMaxY(rect), CGRectGetMinX(rect), CGRectGetMinY(rect), radius);
    CGPathAddArcToPoint(path, NULL, CGRectGetMinX(rect), CGRectGetMinY(rect), CGRectGetMaxX(rect), CGRectGetMinY(rect), radius);
    CGPathCloseSubpath(path);
    CGContextAddPath(ctx, path);
    CGContextFillPath(ctx);
    CGPathRelease(path);
}

static void StrokeRoundedRect(CGContextRef ctx, CGRect rect, CGFloat radius, CGFloat lineWidth) {
    CGMutablePathRef path = CGPathCreateMutable();
    CGPathMoveToPoint(path, NULL, CGRectGetMinX(rect) + radius, CGRectGetMinY(rect));
    CGPathAddArcToPoint(path, NULL, CGRectGetMaxX(rect), CGRectGetMinY(rect), CGRectGetMaxX(rect), CGRectGetMaxY(rect), radius);
    CGPathAddArcToPoint(path, NULL, CGRectGetMaxX(rect), CGRectGetMaxY(rect), CGRectGetMinX(rect), CGRectGetMaxY(rect), radius);
    CGPathAddArcToPoint(path, NULL, CGRectGetMinX(rect), CGRectGetMaxY(rect), CGRectGetMinX(rect), CGRectGetMinY(rect), radius);
    CGPathAddArcToPoint(path, NULL, CGRectGetMinX(rect), CGRectGetMinY(rect), CGRectGetMaxX(rect), CGRectGetMinY(rect), radius);
    CGPathCloseSubpath(path);
    CGContextSetLineWidth(ctx, lineWidth);
    CGContextAddPath(ctx, path);
    CGContextStrokePath(ctx);
    CGPathRelease(path);
}

static void DrawStudioConsoleKnob(CGContextRef ctx,
                                  CGRect card,
                                  float normVal,
                                  BOOL bipolar,
                                  CGFloat accentR,
                                  CGFloat accentG,
                                  CGFloat accentB,
                                  NSString *name,
                                  NSString *valStr) {
    normVal = std::max(0.0f, std::min(1.0f, normVal));

    // 1. Recessed Anodized Channel Module Card
    CGContextSetRGBFillColor(ctx, 0.08, 0.09, 0.11, 0.95);
    FillRoundedRect(ctx, card, 6.0);
    CGContextSetRGBStrokeColor(ctx, 0.22, 0.25, 0.29, 0.85);
    StrokeRoundedRect(ctx, card, 6.0, 1.0);

    // Top Header Band inside Card
    CGFloat headerY = CGRectGetMaxY(card) - 22.0;
    CGContextSetRGBStrokeColor(ctx, 0.18, 0.20, 0.24, 0.9);
    CGContextSetLineWidth(ctx, 1.0);
    CGContextMoveToPoint(ctx, card.origin.x + 8.0, headerY);
    CGContextAddLineToPoint(ctx, CGRectGetMaxX(card) - 8.0, headerY);
    CGContextStrokePath(ctx);

    NSDictionary *nameAttrs = @{
        NSFontAttributeName: SysFont(9.5, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.82 alpha:1.0]
    };
    NSSize nameSize = [name sizeWithAttributes:nameAttrs];
    [name drawAtPoint:NSMakePoint(CGRectGetMidX(card) - nameSize.width * 0.5, headerY + 4.0) withAttributes:nameAttrs];

    // 2. Knob Geometry (well clear of header and bottom readout pill)
    CGFloat cx = CGRectGetMidX(card);
    CGFloat cy = card.origin.y + 82.0;
    CGFloat skirtR = 27.0;
    CGFloat capR = 20.5;
    CGFloat arcR = 33.5;

    // Angles in standard CG coordinates (+Y up):
    // 0.0 (7 o'clock) = 225 deg, 0.5 (12 o'clock) = 90 deg, 1.0 (5 o'clock) = -45 deg
    CGFloat startDeg = 225.0;
    CGFloat endDeg = -45.0;
    CGFloat valDeg = 225.0 - normVal * 270.0;

    // 3. Precision Outer Scale Ticks (21 ticks)
    const int numTicks = 21;
    for (int i = 0; i < numTicks; ++i) {
        float t = (float)i / (float)(numTicks - 1);
        CGFloat deg = 225.0 - t * 270.0;
        CGFloat rad = deg * M_PI / 180.0;
        BOOL isMajor = (i % 5 == 0);
        CGFloat r1 = arcR + 3.5;
        CGFloat r2 = arcR + (isMajor ? 7.5 : 5.5);
        CGFloat x1 = cx + r1 * std::cos(rad);
        CGFloat y1 = cy + r1 * std::sin(rad);
        CGFloat x2 = cx + r2 * std::cos(rad);
        CGFloat y2 = cy + r2 * std::sin(rad);

        if (isMajor) {
            CGContextSetRGBStrokeColor(ctx, 0.62, 0.67, 0.73, 0.85);
            CGContextSetLineWidth(ctx, 1.3);
        } else {
            CGContextSetRGBStrokeColor(ctx, 0.35, 0.39, 0.44, 0.65);
            CGContextSetLineWidth(ctx, 0.9);
        }
        CGContextMoveToPoint(ctx, x1, y1);
        CGContextAddLineToPoint(ctx, x2, y2);
        CGContextStrokePath(ctx);
    }

    // 4. Recessed LED Collar Track & Illuminated Value Arc
    CGContextSetLineCap(ctx, kCGLineCapRound);
    CGContextSetRGBStrokeColor(ctx, 0.04, 0.05, 0.06, 0.95);
    CGContextSetLineWidth(ctx, 3.6);
    CGContextAddArc(ctx, cx, cy, arcR, startDeg * M_PI / 180.0, endDeg * M_PI / 180.0, 1);
    CGContextStrokePath(ctx);

    CGContextSetRGBStrokeColor(ctx, accentR, accentG, accentB, 0.95);
    CGContextSetLineWidth(ctx, 3.0);
    if (bipolar) {
        CGFloat centerDeg = 90.0;
        if (std::fabs(valDeg - centerDeg) > 1.0) {
            int clockwise = (valDeg < centerDeg) ? 1 : 0;
            CGContextAddArc(ctx, cx, cy, arcR, centerDeg * M_PI / 180.0, valDeg * M_PI / 180.0, clockwise);
            CGContextStrokePath(ctx);
        }
    } else {
        if (normVal > 0.005f) {
            CGContextAddArc(ctx, cx, cy, arcR, startDeg * M_PI / 180.0, valDeg * M_PI / 180.0, 1);
            CGContextStrokePath(ctx);
        }
    }

    // 5. Drop Shadow & Knurled Outer Skirt
    CGContextSetRGBFillColor(ctx, 0.0, 0.0, 0.0, 0.60);
    CGContextFillEllipseInRect(ctx, CGRectMake(cx - skirtR - 1.5, cy - skirtR - 3.5, (skirtR + 1.5) * 2.0, (skirtR + 1.5) * 2.0));

    CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
    CGFloat skirtColors[8] = {
        0.24, 0.27, 0.31, 1.0,
        0.10, 0.11, 0.13, 1.0
    };
    CGGradientRef skirtGrad = CGGradientCreateWithColorComponents(cs, skirtColors, NULL, 2);
    CGContextSaveGState(ctx);
    CGContextAddEllipseInRect(ctx, CGRectMake(cx - skirtR, cy - skirtR, skirtR * 2.0, skirtR * 2.0));
    CGContextClip(ctx);
    CGContextDrawLinearGradient(ctx, skirtGrad, CGPointMake(cx, cy + skirtR), CGPointMake(cx, cy - skirtR), 0);
    CGGradientRelease(skirtGrad);
    CGContextRestoreGState(ctx);

    // Subtle knurl serrations around the skirt
    for (int k = 0; k < 24; ++k) {
        CGFloat kRad = (k * 15.0) * M_PI / 180.0;
        CGFloat kx1 = cx + (capR + 1.2) * std::cos(kRad);
        CGFloat ky1 = cy + (capR + 1.2) * std::sin(kRad);
        CGFloat kx2 = cx + (skirtR - 0.8) * std::cos(kRad);
        CGFloat ky2 = cy + (skirtR - 0.8) * std::sin(kRad);
        CGContextSetRGBStrokeColor(ctx, 0.05, 0.06, 0.07, 0.55);
        CGContextSetLineWidth(ctx, 1.0);
        CGContextMoveToPoint(ctx, kx1, ky1);
        CGContextAddLineToPoint(ctx, kx2, ky2);
        CGContextStrokePath(ctx);
    }

    CGContextSetRGBStrokeColor(ctx, 0.36, 0.40, 0.45, 0.80);
    CGContextSetLineWidth(ctx, 1.0);
    CGContextStrokeEllipseInRect(ctx, CGRectMake(cx - skirtR, cy - skirtR, skirtR * 2.0, skirtR * 2.0));

    // 6. Machined Top Cap
    CGFloat capColors[8] = {
        0.30, 0.33, 0.38, 1.0,
        0.15, 0.17, 0.20, 1.0
    };
    CGGradientRef capGrad = CGGradientCreateWithColorComponents(cs, capColors, NULL, 2);
    CGContextSaveGState(ctx);
    CGContextAddEllipseInRect(ctx, CGRectMake(cx - capR, cy - capR, capR * 2.0, capR * 2.0));
    CGContextClip(ctx);
    CGContextDrawLinearGradient(ctx, capGrad, CGPointMake(cx - capR * 0.4, cy + capR), CGPointMake(cx + capR * 0.4, cy - capR), 0);
    CGGradientRelease(capGrad);
    CGContextRestoreGState(ctx);
    CGColorSpaceRelease(cs);

    CGContextSetRGBStrokeColor(ctx, 0.48, 0.53, 0.58, 0.70);
    CGContextSetLineWidth(ctx, 1.0);
    CGContextStrokeEllipseInRect(ctx, CGRectMake(cx - capR, cy - capR, capR * 2.0, capR * 2.0));

    // 7. Crisp Illuminated Pointer Line & Tip Dot
    CGFloat valRad = valDeg * M_PI / 180.0;
    CGFloat px1 = cx + (capR * 0.28) * std::cos(valRad);
    CGFloat py1 = cy + (capR * 0.28) * std::sin(valRad);
    CGFloat px2 = cx + (capR - 2.2) * std::cos(valRad);
    CGFloat py2 = cy + (capR - 2.2) * std::sin(valRad);

    CGContextSetRGBStrokeColor(ctx, accentR, accentG, accentB, 1.0);
    CGContextSetLineWidth(ctx, 2.2);
    CGContextMoveToPoint(ctx, px1, py1);
    CGContextAddLineToPoint(ctx, px2, py2);
    CGContextStrokePath(ctx);

    // Center dimple
    CGContextSetRGBFillColor(ctx, 0.11, 0.12, 0.14, 1.0);
    CGContextFillEllipseInRect(ctx, CGRectMake(cx - 4.0, cy - 4.0, 8.0, 8.0));

    // 8. Recessed OLED Value Readout Pill at Bottom of Card
    CGRect pillRect = CGRectMake(card.origin.x + 14.0, card.origin.y + 8.0, card.size.width - 28.0, 20.0);
    CGContextSetRGBFillColor(ctx, 0.04, 0.05, 0.06, 1.0);
    FillRoundedRect(ctx, pillRect, 4.0);
    CGContextSetRGBStrokeColor(ctx, 0.20, 0.24, 0.28, 0.85);
    StrokeRoundedRect(ctx, pillRect, 4.0, 0.9);

    NSDictionary *valAttrs = @{
        NSFontAttributeName: MonoFont(10.0, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:accentR green:accentG blue:accentB alpha:1.0]
    };
    NSSize valSize = [valStr sizeWithAttributes:valAttrs];
    [valStr drawAtPoint:NSMakePoint(CGRectGetMidX(pillRect) - valSize.width * 0.5, pillRect.origin.y + 3.5) withAttributes:valAttrs];
}

static void DrawAnalogVuDial(CGContextRef ctx, CGRect dialRect, float vuLinear, float peakLinear, BOOL calMinus18, NSString *chLabel) {
    // Convert linear amplitude to VU (-20 .. +3 VU)
    // calMinus18 == YES: 0 VU = +4 dBu = -18 dBFS
    // calMinus18 == NO:  0 VU = -3 dBFS
    float db = (vuLinear > 0.0001f) ? (20.0f * std::log10(vuLinear)) : -60.0f;
    float vuVal = db + (calMinus18 ? 18.0f : 3.0f);
    float normVu = 0.0f;
    if (vuVal <= -20.0f) {
        normVu = 0.0f;
    } else if (vuVal <= 0.0f) {
        // Non-linear compressed low scale (-20..0 VU maps to 0.0..0.72 of scale)
        float t = (vuVal + 20.0f) / 20.0f;
        normVu = std::pow(t, 1.35f) * 0.72f;
    } else {
        // 0..+3 VU maps to 0.72..1.0 of scale
        float t = std::min(3.0f, vuVal) / 3.0f;
        normVu = 0.72f + t * 0.28f;
    }

    // Outer recessed bezel
    CGContextSetRGBFillColor(ctx, 0.05, 0.06, 0.07, 1.0);
    FillRoundedRect(ctx, dialRect, 6.0);

    CGRect faceRect = CGRectInset(dialRect, 3.0, 3.0);
    CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
    CGFloat faceColors[8] = {
        0.96, 0.90, 0.76, 1.0, // Warm vintage backlit cream top
        0.84, 0.73, 0.52, 1.0  // Warm amber bottom
    };
    CGGradientRef faceGrad = CGGradientCreateWithColorComponents(cs, faceColors, NULL, 2);
    CGContextSaveGState(ctx);
    CGMutablePathRef clipPath = CGPathCreateMutable();
    CGPathMoveToPoint(clipPath, NULL, CGRectGetMinX(faceRect) + 4.5, CGRectGetMinY(faceRect));
    CGPathAddArcToPoint(clipPath, NULL, CGRectGetMaxX(faceRect), CGRectGetMinY(faceRect), CGRectGetMaxX(faceRect), CGRectGetMaxY(faceRect), 4.5);
    CGPathAddArcToPoint(clipPath, NULL, CGRectGetMaxX(faceRect), CGRectGetMaxY(faceRect), CGRectGetMinX(faceRect), CGRectGetMaxY(faceRect), 4.5);
    CGPathAddArcToPoint(clipPath, NULL, CGRectGetMinX(faceRect), CGRectGetMaxY(faceRect), CGRectGetMinX(faceRect), CGRectGetMinY(faceRect), 4.5);
    CGPathAddArcToPoint(clipPath, NULL, CGRectGetMinX(faceRect), CGRectGetMinY(faceRect), CGRectGetMaxX(faceRect), CGRectGetMinY(faceRect), 4.5);
    CGPathCloseSubpath(clipPath);
    CGContextAddPath(ctx, clipPath);
    CGContextClip(ctx);
    CGContextDrawLinearGradient(ctx, faceGrad, CGPointMake(CGRectGetMidX(faceRect), CGRectGetMaxY(faceRect)),
                                CGPointMake(CGRectGetMidX(faceRect), CGRectGetMinY(faceRect)), 0);
    CGGradientRelease(faceGrad);

    // Pivot point below the bottom of the face — wide 2x horizontal broadcast VU dial geometry
    BOOL isWideDial = (faceRect.size.width >= 180.0);
    CGFloat pivotX = CGRectGetMidX(faceRect);
    CGFloat pivotY = CGRectGetMinY(faceRect) - (isWideDial ? 36.0 : 18.0);
    CGFloat scaleR = faceRect.size.height * (isWideDial ? 1.04 : 0.84);

    // Wide 2x VU Scale Arc: 132 deg (left, -20 VU) to 48 deg (right, +3 VU)
    CGFloat leftDeg = isWideDial ? 132.0 : 113.0;
    CGFloat rightDeg = isWideDial ? 48.0 : 67.0;
    CGFloat zeroDeg = leftDeg - 0.72 * (leftDeg - rightDeg);

    // Black normal zone arc (-20 to 0 VU)
    CGContextSetRGBStrokeColor(ctx, 0.14, 0.12, 0.10, 0.90);
    CGContextSetLineWidth(ctx, 1.5);
    CGContextAddArc(ctx, pivotX, pivotY, scaleR, leftDeg * M_PI / 180.0, zeroDeg * M_PI / 180.0, 1);
    CGContextStrokePath(ctx);

    // Red overload zone band (0 to +3 VU)
    CGContextSetRGBStrokeColor(ctx, 0.82, 0.14, 0.14, 0.95);
    CGContextSetLineWidth(ctx, 3.4);
    CGContextAddArc(ctx, pivotX, pivotY, scaleR + 1.0, zeroDeg * M_PI / 180.0, rightDeg * M_PI / 180.0, 1);
    CGContextStrokePath(ctx);

    // Scale Ticks & Labels (-20, -10, -7, -5, -3, -2, -1, 0, +1, +2, +3)
    struct VuMark { float vu; const char *label; BOOL red; };
    const VuMark marks[] = {
        {-20.0f, "-20", NO},
        {-10.0f, "-10", NO},
        {-7.0f,  "-7",  NO},
        {-5.0f,  "-5",  NO},
        {-3.0f,  "-3",  NO},
        {-2.0f,  "-2",  NO},
        {-1.0f,  "-1",  NO},
        {0.0f,   "0",   NO},
        {1.0f,   "+1",  YES},
        {2.0f,   "+2",  YES},
        {3.0f,   "+3",  YES}
    };
    for (size_t i = 0; i < sizeof(marks) / sizeof(marks[0]); ++i) {
        float mv = marks[i].vu;
        float mn = (mv <= 0.0f) ? (std::pow((mv + 20.0f) / 20.0f, 1.35f) * 0.72f) : (0.72f + (mv / 3.0f) * 0.28f);
        CGFloat mDeg = leftDeg - mn * (leftDeg - rightDeg);
        CGFloat mRad = mDeg * M_PI / 180.0;
        CGFloat x1 = pivotX + (scaleR - 3.5) * std::cos(mRad);
        CGFloat y1 = pivotY + (scaleR - 3.5) * std::sin(mRad);
        CGFloat x2 = pivotX + (scaleR + 6.0) * std::cos(mRad);
        CGFloat y2 = pivotY + (scaleR + 6.0) * std::sin(mRad);

        if (marks[i].red) {
            CGContextSetRGBStrokeColor(ctx, 0.82, 0.14, 0.14, 0.95);
        } else {
            CGContextSetRGBStrokeColor(ctx, 0.14, 0.12, 0.10, 0.90);
        }
        CGContextSetLineWidth(ctx, (mv == 0.0f) ? 1.8 : 1.1);
        CGContextMoveToPoint(ctx, x1, y1);
        CGContextAddLineToPoint(ctx, x2, y2);
        CGContextStrokePath(ctx);

        // Label marks across the wide 2x VU dial
        BOOL showLbl = isWideDial
            ? (mv == -20.0f || mv == -10.0f || mv == -7.0f || mv == -5.0f || mv == -3.0f || mv == -1.0f || mv == 0.0f || mv == 1.0f || mv == 2.0f || mv == 3.0f)
            : (mv == -20.0f || mv == -10.0f || mv == -3.0f || mv == 0.0f || mv == 3.0f);
        if (showLbl) {
            NSString *lbl = [NSString stringWithUTF8String:marks[i].label];
            NSDictionary *mAttrs = @{
                NSFontAttributeName: MonoFont(7.8, NSFontWeightBold),
                NSForegroundColorAttributeName: marks[i].red
                    ? [NSColor colorWithCalibratedRed:0.80 green:0.12 blue:0.12 alpha:1.0]
                    : [NSColor colorWithCalibratedRed:0.15 green:0.13 blue:0.10 alpha:1.0]
            };
            NSSize lSz = [lbl sizeWithAttributes:mAttrs];
            CGFloat lx = pivotX + (scaleR + 13.5) * std::cos(mRad) - lSz.width * 0.5;
            CGFloat ly = pivotY + (scaleR + 13.5) * std::sin(mRad) - lSz.height * 0.5;
            [lbl drawAtPoint:NSMakePoint(lx, ly) withAttributes:mAttrs];
        }
    }

    // Center "VU" Legend & Channel Label
    NSDictionary *vuTitleAttrs = @{
        NSFontAttributeName: SysFont(12.5, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.20 green:0.17 blue:0.13 alpha:0.78]
    };
    NSSize vuSz = [@"VU" sizeWithAttributes:vuTitleAttrs];
    [@"VU" drawAtPoint:NSMakePoint(pivotX - vuSz.width * 0.5, faceRect.origin.y + 34.0) withAttributes:vuTitleAttrs];

    if (isWideDial) {
        NSDictionary *subAttrs = @{
            NSFontAttributeName: MonoFont(7.5, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.24 green:0.20 blue:0.14 alpha:0.78]
        };
        NSString *subStr = calMinus18 ? @"0 VU = +4 dBu = -18 dBFS" : @"0 VU = -3 dBFS";
        NSSize subSz = [subStr sizeWithAttributes:subAttrs];
        [subStr drawAtPoint:NSMakePoint(pivotX - subSz.width * 0.5, faceRect.origin.y + 22.0) withAttributes:subAttrs];
    }

    NSDictionary *chAttrs = @{
        NSFontAttributeName: MonoFont(9.0, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.25 green:0.20 blue:0.14 alpha:0.92]
    };
    [chLabel drawAtPoint:NSMakePoint(faceRect.origin.x + 8.0, faceRect.origin.y + 6.0) withAttributes:chAttrs];

    // Peak LED in top-right corner of VU dial
    BOOL peakOn = (peakLinear >= 0.707f);
    CGRect pkRect = CGRectMake(CGRectGetMaxX(faceRect) - 16.0, CGRectGetMaxY(faceRect) - 14.0, 8.0, 8.0);
    CGContextSetRGBFillColor(ctx, peakOn ? 0.95 : 0.35, peakOn ? 0.15 : 0.22, peakOn ? 0.15 : 0.18, 1.0);
    CGContextFillEllipseInRect(ctx, pkRect);
    CGContextSetRGBStrokeColor(ctx, 0.15, 0.12, 0.10, 0.8);
    CGContextSetLineWidth(ctx, 0.8);
    CGContextStrokeEllipseInRect(ctx, pkRect);
    if (isWideDial) {
        NSDictionary *pkAttrs = @{
            NSFontAttributeName: MonoFont(7.5, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.35 green:0.18 blue:0.14 alpha:0.85]
        };
        [@"PEAK" drawAtPoint:NSMakePoint(CGRectGetMinX(pkRect) - 24.0, CGRectGetMinY(pkRect) - 1.5) withAttributes:pkAttrs];
    }

    // Ballistic Needle + Shadow
    CGFloat needleDeg = leftDeg - normVu * (leftDeg - rightDeg);
    CGFloat needleRad = needleDeg * M_PI / 180.0;
    CGFloat tipX = pivotX + (scaleR + 6.0) * std::cos(needleRad);
    CGFloat tipY = pivotY + (scaleR + 6.0) * std::sin(needleRad);

    // Needle shadow
    CGContextSetRGBStrokeColor(ctx, 0.0, 0.0, 0.0, 0.25);
    CGContextSetLineWidth(ctx, 1.8);
    CGContextMoveToPoint(ctx, pivotX + 1.5, pivotY - 1.0);
    CGContextAddLineToPoint(ctx, tipX + 1.5, tipY - 1.0);
    CGContextStrokePath(ctx);

    // Needle body
    CGContextSetRGBStrokeColor(ctx, 0.10, 0.09, 0.08, 0.96);
    CGContextSetLineWidth(ctx, 1.5);
    CGContextMoveToPoint(ctx, pivotX, pivotY);
    CGContextAddLineToPoint(ctx, tipX, tipY);
    CGContextStrokePath(ctx);

    // Pivot shroud at bottom edge
    CGContextSetRGBFillColor(ctx, 0.12, 0.13, 0.15, 1.0);
    CGFloat shroudR = isWideDial ? 22.0 : 14.0;
    CGContextFillEllipseInRect(ctx, CGRectMake(pivotX - shroudR, CGRectGetMinY(faceRect) - shroudR * 1.35, shroudR * 2.0, shroudR * 2.0));

    // Top glass highlight
    CGFloat glassColors[8] = {
        1.0, 1.0, 1.0, 0.18,
        1.0, 1.0, 1.0, 0.0
    };
    CGGradientRef glassGrad = CGGradientCreateWithColorComponents(cs, glassColors, NULL, 2);
    CGContextDrawLinearGradient(ctx, glassGrad,
                                CGPointMake(pivotX, CGRectGetMaxY(faceRect)),
                                CGPointMake(pivotX, CGRectGetMaxY(faceRect) - 28.0), 0);
    CGGradientRelease(glassGrad);
    CGColorSpaceRelease(cs);

    CGContextRestoreGState(ctx);
    CGPathRelease(clipPath);

    // Inner bezel stroke
    CGContextSetRGBStrokeColor(ctx, 0.28, 0.31, 0.35, 0.9);
    StrokeRoundedRect(ctx, dialRect, 6.0, 1.0);
}

static void DrawBankFoldButton(CGContextRef ctx, CGRect btnRect, NSString *label) {
    CGContextSetRGBFillColor(ctx, 0.15, 0.17, 0.20, 1.0);
    FillRoundedRect(ctx, btnRect, 3.5);
    CGContextSetRGBStrokeColor(ctx, 0.38, 0.43, 0.49, 0.85);
    StrokeRoundedRect(ctx, btnRect, 3.5, 1.0);

    NSDictionary *attrs = @{
        NSFontAttributeName: MonoFont(8.0, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.85 alpha:1.0]
    };
    NSSize sz = [label sizeWithAttributes:attrs];
    [label drawAtPoint:NSMakePoint(CGRectGetMidX(btnRect) - sz.width * 0.5, btnRect.origin.y + (btnRect.size.height - sz.height) * 0.5)
        withAttributes:attrs];
}

static void DrawCollapsedBankStrip(CGContextRef ctx, CGRect stripRect, NSString *verticalTitle, BOOL showLed, BOOL ledGreen) {
    CGContextSetRGBFillColor(ctx, 0.10, 0.11, 0.13, 0.95);
    FillRoundedRect(ctx, stripRect, 5.0);
    CGContextSetRGBStrokeColor(ctx, 0.30, 0.34, 0.39, 0.80);
    StrokeRoundedRect(ctx, stripRect, 5.0, 1.0);

    // Top expand button [▸]
    CGRect expBtn = CGRectMake(stripRect.origin.x + 5.0, CGRectGetMaxY(stripRect) - 26.0, stripRect.size.width - 10.0, 18.0);
    DrawBankFoldButton(ctx, expBtn, @"▸");

    if (showLed) {
        CGFloat lx = CGRectGetMidX(stripRect);
        CGFloat ly = CGRectGetMaxY(stripRect) - 42.0;
        CGContextSetRGBFillColor(ctx, ledGreen ? 0.25 : 0.90, ledGreen ? 0.90 : 0.28, 0.30, 1.0);
        CGContextFillEllipseInRect(ctx, CGRectMake(lx - 4.0, ly - 4.0, 8.0, 8.0));
    }

    // Draw stacked vertical letters down the strip
    NSDictionary *vAttrs = @{
        NSFontAttributeName: MonoFont(9.5, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.74 alpha:1.0]
    };
    NSUInteger len = [verticalTitle length];
    CGFloat startY = CGRectGetMidY(stripRect) + (len * 13.0) * 0.5 - 10.0;
    for (NSUInteger i = 0; i < len; ++i) {
        NSString *ch = [verticalTitle substringWithRange:NSMakeRange(i, 1)];
        NSSize sz = [ch sizeWithAttributes:vAttrs];
        [ch drawAtPoint:NSMakePoint(CGRectGetMidX(stripRect) - sz.width * 0.5, startY - i * 13.0) withAttributes:vAttrs];
    }
}

static void DrawMasterMeteringBay(CGContextRef ctx,
                                  CGRect bayRect,
                                  BOOL isVuMode,
                                  BOOL calMinus18,
                                  BOOL isInputMode,
                                  float ppmL,
                                  float ppmR,
                                  float vuL,
                                  float vuR,
                                  float holdL,
                                  float holdR) {
    // Recessed Metering Bay Card (128px wide in PPM mode, 256px [2x] wide in VU mode)
    CGContextSetRGBFillColor(ctx, 0.08, 0.09, 0.11, 0.95);
    FillRoundedRect(ctx, bayRect, 6.0);
    CGContextSetRGBStrokeColor(ctx, 0.22, 0.25, 0.29, 0.85);
    StrokeRoundedRect(ctx, bayRect, 6.0, 1.0);

    // Shared Header Controls (right-aligned so they fit identically in 128px PPM and 256px VU modes):
    // [IN | OUT*] (46px) + [VU* | PPM] (50px) + [◂] (17px)
    CGRect foldBtn = CGRectMake(CGRectGetMaxX(bayRect) - 21.0, CGRectGetMaxY(bayRect) - 22.0, 17.0, 16.0);
    DrawBankFoldButton(ctx, foldBtn, @"◂");

    CGRect modeBtn = CGRectMake(CGRectGetMinX(foldBtn) - 53.0, CGRectGetMaxY(bayRect) - 22.0, 50.0, 16.0);
    CGContextSetRGBFillColor(ctx, 0.14, 0.17, 0.21, 1.0);
    FillRoundedRect(ctx, modeBtn, 3.5);
    CGContextSetRGBStrokeColor(ctx, 0.95, 0.75, 0.25, 0.85);
    StrokeRoundedRect(ctx, modeBtn, 3.5, 1.0);

    NSString *modeTxt = isVuMode ? @"VU* | PPM" : @"VU | PPM*";
    NSDictionary *modeAttrs = @{
        NSFontAttributeName: MonoFont(7.5, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.95 green:0.78 blue:0.30 alpha:1.0]
    };
    NSSize mSz = [modeTxt sizeWithAttributes:modeAttrs];
    [modeTxt drawAtPoint:NSMakePoint(CGRectGetMidX(modeBtn) - mSz.width * 0.5, modeBtn.origin.y + 2.5) withAttributes:modeAttrs];

    // Input / Output Source Toggle Pill: [IN* | OUT] vs [IN | OUT*]
    CGRect ioBtn = CGRectMake(CGRectGetMinX(modeBtn) - 49.0, CGRectGetMaxY(bayRect) - 22.0, 46.0, 16.0);
    CGContextSetRGBFillColor(ctx, 0.12, 0.17, 0.16, 1.0);
    FillRoundedRect(ctx, ioBtn, 3.5);
    CGContextSetRGBStrokeColor(ctx, 0.28, 0.86, 0.62, 0.85);
    StrokeRoundedRect(ctx, ioBtn, 3.5, 1.0);

    NSString *ioTxt = isInputMode ? @"IN* | OUT" : @"IN | OUT*";
    NSDictionary *ioAttrs = @{
        NSFontAttributeName: MonoFont(7.4, NSFontWeightBold),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.35 green:0.92 blue:0.68 alpha:1.0]
    };
    NSSize ioSz = [ioTxt sizeWithAttributes:ioAttrs];
    [ioTxt drawAtPoint:NSMakePoint(CGRectGetMidX(ioBtn) - ioSz.width * 0.5, ioBtn.origin.y + 2.5) withAttributes:ioAttrs];

    if (isVuMode) {
        NSDictionary *hdrAttrs = @{
            NSFontAttributeName: SysFont(9.0, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.82 alpha:1.0]
        };
        [@"VU (2X)" drawAtPoint:NSMakePoint(bayRect.origin.x + 8.0, CGRectGetMaxY(bayRect) - 20.0) withAttributes:hdrAttrs];

        // VU Calibration Toggle Pill: [-18* | -3 dB] (0 VU = +4 dBu = -18 dBFS vs 0 VU = -3 dBFS)
        CGRect calBtn = CGRectMake(CGRectGetMinX(ioBtn) - 65.0, CGRectGetMaxY(bayRect) - 22.0, 62.0, 16.0);
        CGContextSetRGBFillColor(ctx, 0.13, 0.16, 0.20, 1.0);
        FillRoundedRect(ctx, calBtn, 3.5);
        CGContextSetRGBStrokeColor(ctx, 0.40, 0.78, 0.96, 0.85);
        StrokeRoundedRect(ctx, calBtn, 3.5, 1.0);

        NSString *calTxt = calMinus18 ? @"-18* | -3 dB" : @"-18 | -3* dB";
        NSDictionary *calAttrs = @{
            NSFontAttributeName: MonoFont(7.4, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.45 green:0.82 blue:0.98 alpha:1.0]
        };
        NSSize cSz = [calTxt sizeWithAttributes:calAttrs];
        [calTxt drawAtPoint:NSMakePoint(CGRectGetMidX(calBtn) - cSz.width * 0.5, calBtn.origin.y + 2.5) withAttributes:calAttrs];

        // Dual Stacked 2x-Wide Classic Analog VU Meters (IN/OUT CH L & CH R)
        CGFloat dialW = bayRect.size.width - 16.0;
        CGFloat dialH = (bayRect.size.height - 46.0) * 0.5;
        CGRect topDial = CGRectMake(bayRect.origin.x + 8.0, bayRect.origin.y + 14.0 + dialH, dialW, dialH - 4.0);
        CGRect botDial = CGRectMake(bayRect.origin.x + 8.0, bayRect.origin.y + 8.0, dialW, dialH - 4.0);

        DrawAnalogVuDial(ctx, topDial, vuL, ppmL, calMinus18, isInputMode ? @"IN L" : @"OUT L");
        DrawAnalogVuDial(ctx, botDial, vuR, ppmR, calMinus18, isInputMode ? @"IN R" : @"OUT R");
    } else {
        // High-Resolution Digital Stereo PPM Bargraphs with dBFS Scale & Peak Hold
        CGFloat meterTop = CGRectGetMaxY(bayRect) - 32.0;
        CGFloat meterBot = bayRect.origin.y + 26.0;
        CGFloat meterH = meterTop - meterBot;
        CGFloat barW = 20.0;
        CGFloat leftBarX = bayRect.origin.x + 16.0;
        CGFloat rightBarX = CGRectGetMaxX(bayRect) - 16.0 - barW;

        auto levelToNorm = [](float lin) -> float {
            if (lin <= 0.001f) return 0.0f;
            float db = 20.0f * std::log10(lin);
            return std::max(0.0f, std::min(1.0f, (db + 48.0f) / 48.0f));
        };

        float normL = levelToNorm(ppmL);
        float normR = levelToNorm(ppmR);
        float normHoldL = levelToNorm(holdL);
        float normHoldR = levelToNorm(holdR);

        auto drawPpmColumn = [&](CGFloat bx, float normVal, float holdVal) {
            CGRect well = CGRectMake(bx, meterBot, barW, meterH);
            CGContextSetRGBFillColor(ctx, 0.04, 0.05, 0.06, 1.0);
            FillRoundedRect(ctx, well, 3.0);
            CGContextSetRGBStrokeColor(ctx, 0.22, 0.25, 0.30, 0.85);
            StrokeRoundedRect(ctx, well, 3.0, 0.9);

            const int segments = 32;
            int lit = (int)std::round(normVal * segments);
            int holdSeg = (int)std::round(holdVal * (segments - 1));
            CGFloat segStep = (meterH - 6.0) / (CGFloat)segments;
            for (int s = 0; s < segments; ++s) {
                CGFloat sy = meterBot + 3.0 + s * segStep;
                CGRect sRect = CGRectMake(bx + 2.5, sy, barW - 5.0, segStep - 1.4);
                BOOL isLit = (s < lit) || (s == holdSeg && holdVal > 0.02f);
                if (s >= segments - 3) {
                    CGContextSetRGBFillColor(ctx, 0.98, 0.22, 0.22, isLit ? 1.0 : 0.16);
                } else if (s >= segments - 8) {
                    CGContextSetRGBFillColor(ctx, 0.96, 0.76, 0.22, isLit ? 1.0 : 0.16);
                } else {
                    CGContextSetRGBFillColor(ctx, 0.22, 0.86, 0.48, isLit ? 1.0 : 0.16);
                }
                CGContextFillRect(ctx, sRect);
            }
        };

        drawPpmColumn(leftBarX, normL, normHoldL);
        drawPpmColumn(rightBarX, normR, normHoldR);

        // Center dB Scale Markings between L and R bars
        const int dbMarks[] = {0, -6, -12, -18, -24, -36, -48};
        NSDictionary *dbAttrs = @{
            NSFontAttributeName: MonoFont(7.5, NSFontWeightMedium),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.62 alpha:1.0]
        };
        for (size_t i = 0; i < sizeof(dbMarks) / sizeof(dbMarks[0]); ++i) {
            float t = (float)(dbMarks[i] + 48) / 48.0f;
            CGFloat y = meterBot + 3.0 + t * (meterH - 6.0);
            NSString *lbl = [NSString stringWithFormat:@"%d", -dbMarks[i]];
            NSSize sz = [lbl sizeWithAttributes:dbAttrs];
            [lbl drawAtPoint:NSMakePoint(CGRectGetMidX(bayRect) - sz.width * 0.5, y - sz.height * 0.5) withAttributes:dbAttrs];
        }

        // Bottom L / R Labels
        NSDictionary *lrAttrs = @{
            NSFontAttributeName: MonoFont(8.5, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.78 alpha:1.0]
        };
        [@"L" drawAtPoint:NSMakePoint(leftBarX + 6.5, bayRect.origin.y + 7.0) withAttributes:lrAttrs];
        [@"R" drawAtPoint:NSMakePoint(rightBarX + 6.5, bayRect.origin.y + 7.0) withAttributes:lrAttrs];
        NSString *ppmSrcLbl = isInputMode ? @"IN PPM" : @"OUT PPM";
        NSSize pSz = [ppmSrcLbl sizeWithAttributes:dbAttrs];
        [ppmSrcLbl drawAtPoint:NSMakePoint(CGRectGetMidX(bayRect) - pSz.width * 0.5, bayRect.origin.y + 7.0) withAttributes:dbAttrs];
    }
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

    // Compute dynamic 3-Bank geometry
    CGFloat rackW = _rackCollapsed ? 36.0 : 256.0;
    CGFloat consoleW = _consoleCollapsed ? 36.0 : 272.0;
    CGFloat meterW = _meterCollapsed ? 36.0 : (_isVuMeterMode ? 288.0 : 144.0);

    CGRect rackBank = CGRectMake(16.0, 38.0, rackW, bounds.size.height - 62.0);
    CGRect consoleBank = CGRectMake(CGRectGetMaxX(rackBank) + 8.0, 38.0, consoleW, bounds.size.height - 62.0);
    CGRect meterBank = CGRectMake(CGRectGetMaxX(consoleBank) + 8.0, 38.0, meterW, bounds.size.height - 62.0);

    BOOL connected = (_statusCode >= 2);

    // =========================================================================
    // BANK 1: OPENGRIDDER PLUGIN SELECTION RACK
    // =========================================================================
    if (_rackCollapsed) {
        DrawCollapsedBankStrip(ctx, rackBank, @"PLUGINS", YES, connected);
    } else {
        CGContextSetRGBFillColor(ctx, 0.11, 0.12, 0.14, 0.92);
        FillRoundedRect(ctx, rackBank, 6.0);
        CGContextSetRGBStrokeColor(ctx, 0.28, 0.31, 0.35, 0.65);
        StrokeRoundedRect(ctx, rackBank, 6.0, 1.0);

        CGFloat rx = rackBank.origin.x + 14.0;

        // Bank 1 Header + [A/B] + LED + [◂] Fold Button
        NSDictionary *brandAttrs = @{
            NSFontAttributeName: SysFont(13.5, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.96 alpha:1.0]
        };
        [@"OPENGRIDDER" drawAtPoint:NSMakePoint(rx, bounds.size.height - 68) withAttributes:brandAttrs];

        // A/B Comparison Badge [A/B]
        BOOL isB = self.rust ? og_native_is_b(self.rust) : NO;
        CGRect abRect = CGRectMake(rx + 122, bounds.size.height - 69, 36, 17);
        CGContextSetRGBFillColor(ctx, isB ? 0.30 : 0.16, isB ? 0.24 : 0.22, isB ? 0.12 : 0.28, 1.0);
        CGContextFillRect(ctx, abRect);
        CGContextSetRGBStrokeColor(ctx, isB ? 0.95 : 0.45, isB ? 0.75 : 0.65, isB ? 0.25 : 0.85, 0.9);
        CGContextSetLineWidth(ctx, 1.0);
        CGContextStrokeRect(ctx, abRect);
        NSString *abTxt = isB ? @"A | B*" : @"A* | B";
        NSDictionary *abAttrs = @{
            NSFontAttributeName: MonoFont(8, NSFontWeightBold),
            NSForegroundColorAttributeName: isB ? [NSColor colorWithCalibratedRed:0.95 green:0.75 blue:0.25 alpha:1.0] : [NSColor whiteColor]
        };
        NSSize abSz = [abTxt sizeWithAttributes:abAttrs];
        [abTxt drawAtPoint:NSMakePoint(abRect.origin.x + (36 - abSz.width) * 0.5, abRect.origin.y + 2.5) withAttributes:abAttrs];

        // Connection LED Dot
        CGFloat ledX = rx + 174, ledY = bounds.size.height - 60.5;
        CGContextSetRGBFillColor(ctx, connected ? 0.3 : 0.9, connected ? 0.9 : 0.3, 0.3, 1.0);
        CGContextFillEllipseInRect(ctx, CGRectMake(ledX - 4, ledY - 4, 8, 8));
        CGContextSetRGBStrokeColor(ctx, connected ? 0.5 : 1.0, 1.0, 0.5, 0.6);
        CGContextSetLineWidth(ctx, 1.6);
        CGContextStrokeEllipseInRect(ctx, CGRectMake(ledX - 5.5, ledY - 5.5, 11, 11));

        // Bank 1 Clap-Close Button [◂]
        CGRect rackFoldBtn = CGRectMake(CGRectGetMaxX(rackBank) - 32.0, bounds.size.height - 69.0, 22.0, 17.0);
        DrawBankFoldButton(ctx, rackFoldBtn, @"◂");

        // Host status label (clickable server selector)
        char srvBuf[128] = "127.0.0.1:55056";
        if (self.rust) {
            og_native_get_active_server(self.rust, srvBuf, sizeof(srvBuf));
        }
        NSString *srvStr = [NSString stringWithFormat:@"Server: %s ▾", srvBuf];
        NSDictionary *hostAttrs = @{
            NSFontAttributeName: MonoFont(10, NSFontWeightMedium),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.95 green:0.75 blue:0.25 alpha:1.0]
        };
        [srvStr drawAtPoint:NSMakePoint(rx, bounds.size.height - 88) withAttributes:hostAttrs];

        // Profile Section Label
        NSDictionary *sectionAttrs = @{
            NSFontAttributeName: SysFont(10, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.65 alpha:1.0]
        };
        [@"LATENCY PROFILE (PDC)" drawAtPoint:NSMakePoint(rx, bounds.size.height - 114) withAttributes:sectionAttrs];

        // 4 Latency Profile Buttons
        NSArray *profileLabels = @[@"LOW (128)", @"STD (256)", @"HIGH (512)", @"STUDIO (1024)"];
        for (int i = 0; i < 4; ++i) {
            CGFloat bx = rx + (i % 2) * 114;
            CGFloat by = bounds.size.height - 146 - (i / 2) * 28;
            CGRect bRect = CGRectMake(bx, by, 108, 24);

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
            [txt drawAtPoint:NSMakePoint(bx + (108 - sz.width) * 0.5, by + 5) withAttributes:btnAttrs];
        }

        // Plugin Slots Header
        [@"REMOTE PLUGIN SLOTS" drawAtPoint:NSMakePoint(rx, bounds.size.height - 198) withAttributes:sectionAttrs];

        // Dynamic Plugin Slots with Per-Plugin Bypass, Presets, Reorder, Oversampling, and Unload
        uint32_t numLoaded = self.rust ? og_native_num_loaded_plugins(self.rust) : 0;
        CGFloat slotH = 26;
        CGFloat slotSpacing = 4;
        CGFloat baseY = bounds.size.height - 230;
        CGFloat slotW = 222.0;

        for (uint32_t i = 0; i < numLoaded; ++i) {
            CGFloat sy = baseY - i * (slotH + slotSpacing);
            CGRect slotRect = CGRectMake(rx, sy, slotW, slotH);
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
            CGRect bypBadge = CGRectMake(slotRect.origin.x + 3, sy + 3, 18, 20);
            CGContextSetRGBFillColor(ctx, bypassed ? 0.85 : 0.12, bypassed ? 0.45 : 0.14, bypassed ? 0.15 : 0.16, 1.0);
            CGContextFillRect(ctx, bypBadge);
            CGContextSetRGBStrokeColor(ctx, bypassed ? 1.0 : 0.4, bypassed ? 0.7 : 0.4, bypassed ? 0.3 : 0.4, 1.0);
            CGContextStrokeRect(ctx, bypBadge);

            NSDictionary *bBadgeAttrs = @{
                NSFontAttributeName: SysFont(9, NSFontWeightBold),
                NSForegroundColorAttributeName: bypassed ? [NSColor blackColor] : [NSColor colorWithCalibratedWhite:0.6 alpha:1.0]
            };
            NSSize bSz = [@"B" sizeWithAttributes:bBadgeAttrs];
            [@"B" drawAtPoint:NSMakePoint(bypBadge.origin.x + (18 - bSz.width) * 0.5, bypBadge.origin.y + 4) withAttributes:bBadgeAttrs];

            // 2. Plugin Title
            NSDictionary *slotAttrs = @{
                NSFontAttributeName: SysFont(10, NSFontWeightMedium),
                NSForegroundColorAttributeName: bypassed ? [NSColor colorWithCalibratedWhite:0.5 alpha:1.0] : [NSColor whiteColor]
            };
            if (slotTitle.length > 13) {
                slotTitle = [NSString stringWithFormat:@"%@…", [slotTitle substringToIndex:12]];
            }
            [slotTitle drawAtPoint:NSMakePoint(rx + 24, sy + 6) withAttributes:slotAttrs];

            // 3. Preset Badge [P]
            CGRect preBadge = CGRectMake(CGRectGetMaxX(slotRect) - 86, sy + 3, 16, 20);
            CGContextSetRGBFillColor(ctx, 0.13, 0.16, 0.20, 1.0);
            CGContextFillRect(ctx, preBadge);
            CGContextSetRGBStrokeColor(ctx, 0.35, 0.55, 0.75, 0.8);
            CGContextStrokeRect(ctx, preBadge);
            NSDictionary *pAttrs = @{
                NSFontAttributeName: SysFont(8, NSFontWeightBold),
                NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.55 green:0.80 blue:0.98 alpha:1.0]
            };
            NSSize pSz = [@"P" sizeWithAttributes:pAttrs];
            [@"P" drawAtPoint:NSMakePoint(preBadge.origin.x + (16 - pSz.width) * 0.5, preBadge.origin.y + 4) withAttributes:pAttrs];

            // 4. Reorder Badge [↕]
            CGRect ordBadge = CGRectMake(CGRectGetMaxX(slotRect) - 68, sy + 3, 16, 20);
            CGContextSetRGBFillColor(ctx, 0.14, 0.15, 0.17, 1.0);
            CGContextFillRect(ctx, ordBadge);
            CGContextSetRGBStrokeColor(ctx, 0.45, 0.48, 0.52, 0.8);
            CGContextStrokeRect(ctx, ordBadge);
            NSSize oSz = [@"↕" sizeWithAttributes:pAttrs];
            [@"↕" drawAtPoint:NSMakePoint(ordBadge.origin.x + (16 - oSz.width) * 0.5, ordBadge.origin.y + 4) withAttributes:pAttrs];

            // 5. Per-Plugin Oversampling Badge (1x / 2x / 4x)
            CGRect osBadge = CGRectMake(CGRectGetMaxX(slotRect) - 50, sy + 3, 24, 20);
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

            // 6. Unload / Delete Button [✕]
            CGRect delBadge = CGRectMake(CGRectGetMaxX(slotRect) - 23, sy + 3, 20, 20);
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
        if (addSy >= 88) {
            CGRect addRect = CGRectMake(rx, addSy, slotW, slotH);
            CGContextSetRGBFillColor(ctx, 0.12, 0.14, 0.16, 0.8);
            CGContextFillRect(ctx, addRect);
            CGContextSetRGBStrokeColor(ctx, 0.35, 0.45, 0.55, 0.8);
            CGFloat dash[2] = {4.0, 3.0};
            CGContextSetLineDash(ctx, 0, dash, 2);
            CGContextStrokeRect(ctx, addRect);
            CGContextSetLineDash(ctx, 0, NULL, 0);

            NSDictionary *addAttrs = @{
                NSFontAttributeName: SysFont(10, NSFontWeightBold),
                NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:0.45 green:0.75 blue:0.95 alpha:1.0]
            };
            NSString *addTxt = @"+ Add Plugin...";
            NSSize addSz = [addTxt sizeWithAttributes:addAttrs];
            [addTxt drawAtPoint:NSMakePoint(addRect.origin.x + (slotW - addSz.width) * 0.5, addSy + 6) withAttributes:addAttrs];
        }

        // Master Bypass Toggle
        CGRect bypRect = CGRectMake(rx, 48, slotW, 30);
        CGContextSetRGBFillColor(ctx, _isBypassed ? 0.35 : 0.18, _isBypassed ? 0.15 : 0.28, _isBypassed ? 0.15 : 0.20, 1.0);
        CGContextFillRect(ctx, bypRect);
        CGContextSetRGBStrokeColor(ctx, _isBypassed ? 0.85 : 0.45, _isBypassed ? 0.35 : 0.75, _isBypassed ? 0.35 : 0.45, 1.0);
        CGContextSetLineWidth(ctx, 1.2);
        CGContextStrokeRect(ctx, bypRect);

        NSString *bypText = _isBypassed ? @"MASTER BYPASS: ENGAGED" : @"MASTER: ACTIVE (PROCESSING)";
        NSDictionary *bypAttrs = @{
            NSFontAttributeName: SysFont(10.5, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor whiteColor]
        };
        NSSize bypSz = [bypText sizeWithAttributes:bypAttrs];
        [bypText drawAtPoint:NSMakePoint(bypRect.origin.x + (slotW - bypSz.width) * 0.5, bypRect.origin.y + 7) withAttributes:bypAttrs];
    }

    // =========================================================================
    // BANK 2: CONSOLE INPUT / WET-DRY / DRIVE-SAT / OUTPUT TRIM
    // =========================================================================
    if (_consoleCollapsed) {
        DrawCollapsedBankStrip(ctx, consoleBank, @"CONSOLE", NO, NO);
    } else {
        CGContextSetRGBFillColor(ctx, 0.11, 0.12, 0.14, 0.92);
        FillRoundedRect(ctx, consoleBank, 6.0);
        CGContextSetRGBStrokeColor(ctx, 0.28, 0.31, 0.35, 0.65);
        StrokeRoundedRect(ctx, consoleBank, 6.0, 1.0);

        // Bank 2 Header + [◂] Clap-Close Button (above the 2x2 knob cards)
        NSDictionary *cHdrAttrs = @{
            NSFontAttributeName: SysFont(10.0, NSFontWeightBold),
            NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.78 alpha:1.0]
        };
        [@"CONSOLE I/O & DRIVE" drawAtPoint:NSMakePoint(consoleBank.origin.x + 12.0, CGRectGetMaxY(consoleBank) - 22.0) withAttributes:cHdrAttrs];

        CGRect consoleFoldBtn = CGRectMake(CGRectGetMaxX(consoleBank) - 32.0, CGRectGetMaxY(consoleBank) - 24.0, 22.0, 17.0);
        DrawBankFoldButton(ctx, consoleFoldBtn, @"◂");

        // 4 Precision Console Channel Module Cards in 2x2 Grid inside consoleBank
        CGFloat cx0 = consoleBank.origin.x + 10.0;
        CGFloat cx1 = consoleBank.origin.x + 140.0;
        CGRect card0 = CGRectMake(cx0, 206, 122, 158); // Top-Left: INPUT GAIN
        CGRect card1 = CGRectMake(cx1, 206, 122, 158); // Top-Right: WET / DRY
        CGRect card2 = CGRectMake(cx0, 44,  122, 156); // Bottom-Left: SAT DRIVE
        CGRect card3 = CGRectMake(cx1, 44,  122, 156); // Bottom-Right: OUTPUT TRIM

        float inDb = (_inputGain - 0.5f) * 48.0f;
        NSString *inStr = [NSString stringWithFormat:@"%+.1f dB", inDb];
        DrawStudioConsoleKnob(ctx, card0, _inputGain, YES, 0.35, 0.78, 0.98, @"INPUT GAIN", inStr);

        NSString *mixStr = [NSString stringWithFormat:@"%.0f %%", _wetDryMix * 100.0f];
        DrawStudioConsoleKnob(ctx, card1, _wetDryMix, NO, 0.28, 0.88, 0.62, @"WET / DRY MIX", mixStr);

        // Pre-gain is 1x..10x (0..+20 dB), exactly what the DSP applies before the tanh shaper
        NSString *drvStr = [NSString stringWithFormat:@"+%.1f dB", 20.0f * std::log10(1.0f + _drive * 9.0f)];
        DrawStudioConsoleKnob(ctx, card2, _drive, NO, 0.96, 0.62, 0.22, @"DRIVE / SAT", drvStr);

        // Drive character [CLN | Tb | Tr] and anti-alias quality [ADAA | 2x | 4x] pills
        {
            int drvChar = self.rust ? (int)std::lround(og_native_get_param(self.rust, 23) * 2.0f) : 0;
            int drvQual = self.rust ? (int)std::lround(og_native_get_param(self.rust, 24) * 2.0f) : 0;
            NSArray *charNames = @[@"CLEAN", @"Tb TUBE", @"Tr XFMR"];
            NSArray *qualNames = @[@"ADAA", @"2x OS", @"4x OS"];
            CGRect charPill = CGRectMake(card2.origin.x + 8.0, card2.origin.y + 32.0, 50.0, 14.0);
            CGRect qualPill = CGRectMake(CGRectGetMaxX(card2) - 58.0, card2.origin.y + 32.0, 50.0, 14.0);
            struct { CGRect r; NSString *t; CGFloat cr, cg, cb; } pills[2] = {
                { charPill, charNames[std::max(0, std::min(2, drvChar))], 0.96, 0.62, 0.22 },
                { qualPill, qualNames[std::max(0, std::min(2, drvQual))], 0.45, 0.82, 0.98 }
            };
            for (int i = 0; i < 2; ++i) {
                CGContextSetRGBFillColor(ctx, 0.13, 0.15, 0.18, 1.0);
                FillRoundedRect(ctx, pills[i].r, 3.0);
                CGContextSetRGBStrokeColor(ctx, pills[i].cr, pills[i].cg, pills[i].cb, 0.8);
                StrokeRoundedRect(ctx, pills[i].r, 3.0, 1.0);
                NSDictionary *pa = @{
                    NSFontAttributeName: MonoFont(7.6, NSFontWeightBold),
                    NSForegroundColorAttributeName: [NSColor colorWithCalibratedRed:pills[i].cr green:pills[i].cg blue:pills[i].cb alpha:1.0]
                };
                NSSize psz = [pills[i].t sizeWithAttributes:pa];
                [pills[i].t drawAtPoint:NSMakePoint(CGRectGetMidX(pills[i].r) - psz.width * 0.5, pills[i].r.origin.y + 2.5) withAttributes:pa];
            }
        }

        float outDb = (_outputGain - 0.5f) * 48.0f;
        NSString *outStr = [NSString stringWithFormat:@"%+.1f dB", outDb];
        DrawStudioConsoleKnob(ctx, card3, _outputGain, YES, 0.95, 0.78, 0.28, @"OUTPUT TRIM", outStr);
    }

    // =========================================================================
    // BANK 3: MASTER METERING BAY (1x PPM / 2x-WIDE ANALOG VU)
    // =========================================================================
    float vuL = (_vuLeft > 0.0001f || self.rust != nullptr) ? _vuLeft : _meterLeft;
    float vuR = (_vuRight > 0.0001f || self.rust != nullptr) ? _vuRight : _meterRight;
    float holdL = (_peakHoldLeft > 0.0001f || self.rust != nullptr) ? _peakHoldLeft : _meterLeft;
    float holdR = (_peakHoldRight > 0.0001f || self.rust != nullptr) ? _peakHoldRight : _meterRight;

    if (_meterCollapsed) {
        BOOL sigActive = ( _meterLeft > 0.01f || _meterRight > 0.01f );
        DrawCollapsedBankStrip(ctx, meterBank, @"METER", YES, sigActive);
    } else {
        CGContextSetRGBFillColor(ctx, 0.11, 0.12, 0.14, 0.92);
        FillRoundedRect(ctx, meterBank, 6.0);
        CGContextSetRGBStrokeColor(ctx, 0.28, 0.31, 0.35, 0.65);
        StrokeRoundedRect(ctx, meterBank, 6.0, 1.0);

        // Inner Metering Bay is 128px wide in PPM mode and 256px (twice as wide!) in VU mode
        CGFloat innerW = _isVuMeterMode ? 256.0 : 128.0;
        CGFloat innerX = meterBank.origin.x + (meterBank.size.width - innerW) * 0.5;
        CGRect meterBay = CGRectMake(innerX, 44.0, innerW, bounds.size.height - 74.0);
        DrawMasterMeteringBay(ctx, meterBay, _isVuMeterMode, _isVuCalibratedMinus18, _isMeterInputMode, _meterLeft, _meterRight, vuL, vuR, holdL, holdR);
    }

    // Footer Status Text
    NSDictionary *footerAttrs = @{
        NSFontAttributeName: MonoFont(10, NSFontWeightRegular),
        NSForegroundColorAttributeName: [NSColor colorWithCalibratedWhite:0.55 alpha:0.8]
    };
    NSString *footer = (bounds.size.width < 420.0)
        ? [NSString stringWithFormat:@"PDC: %u smp", _latencySamples]
        : [NSString stringWithFormat:@"PDC Latency: %u smp (%.2f ms) • GoodLookinDri Hardware UI Engine",
           _latencySamples, (double)_latencySamples / 44.1];
    [footer drawAtPoint:NSMakePoint(24, 14) withAttributes:footerAttrs];
}

#pragma mark - Mouse Interaction

- (void)mouseDown:(NSEvent *)event {
    if (self.window) {
        [self.window makeFirstResponder:self];
    }
    NSPoint p = [self convertPoint:event.locationInWindow fromView:nil];
    CGPoint cp = NSPointToCGPoint(p);
    CGRect bounds = NSRectToCGRect(self.bounds);

    CGFloat rackW = _rackCollapsed ? 36.0 : 256.0;
    CGFloat consoleW = _consoleCollapsed ? 36.0 : 272.0;
    CGFloat meterW = _meterCollapsed ? 36.0 : (_isVuMeterMode ? 288.0 : 144.0);

    CGRect rackBank = CGRectMake(16.0, 38.0, rackW, bounds.size.height - 62.0);
    CGRect consoleBank = CGRectMake(CGRectGetMaxX(rackBank) + 8.0, 38.0, consoleW, bounds.size.height - 62.0);
    CGRect meterBank = CGRectMake(CGRectGetMaxX(consoleBank) + 8.0, 38.0, meterW, bounds.size.height - 62.0);

    // =========================================================================
    // 1. Check Bank 1 (Rack / Plugin Selection) Collapse / Interactions
    // =========================================================================
    if (_rackCollapsed) {
        if (CGRectContainsPoint(rackBank, cp)) {
            [self setRackCollapsed:NO];
            return;
        }
    } else {
        CGRect rackFoldBtn = CGRectMake(CGRectGetMaxX(rackBank) - 32.0, bounds.size.height - 69.0, 22.0, 17.0);
        if (CGRectContainsPoint(rackFoldBtn, cp)) {
            [self setRackCollapsed:YES];
            return;
        }

        CGFloat rx = rackBank.origin.x + 14.0;
        CGFloat slotW = 222.0;

        // Check A/B Comparison Badge [A/B]
        CGRect abRect = CGRectMake(rx + 122, bounds.size.height - 69, 36, 17);
        if (CGRectContainsPoint(abRect, cp)) {
            if (self.rust) og_native_toggle_ab(self.rust);
            [self setNeedsDisplay:YES];
            return;
        }

        // Check Server Selector Row
        CGRect srvRect = CGRectMake(rx, bounds.size.height - 92, 210, 18);
        if (CGRectContainsPoint(srvRect, cp)) {
            [self showServerMenuAtPoint:p];
            return;
        }

        // Check Latency Profile Buttons
        for (int i = 0; i < 4; ++i) {
            CGFloat bx = rx + (i % 2) * 114;
            CGFloat by = bounds.size.height - 146 - (i / 2) * 28;
            CGRect bRect = CGRectMake(bx, by, 108, 24);
            if (CGRectContainsPoint(bRect, cp)) {
                _activeProfile = i;
                uint32_t target = (i == 0) ? 128 : (i == 1) ? 256 : (i == 2) ? 512 : 1024;
                _latencySamples = target;
                if (self.rust) {
                    og_native_set_param(self.rust, 2, (float)i / 3.0f);
                    og_native_set_transport_latency(self.rust, target);
                    og_native_accept_latency(self.rust, og_native_latency(self.rust));
                    _latencySamples = og_native_latency(self.rust);
                }
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
            CGRect slotRect = CGRectMake(rx, sy, slotW, slotH);

            CGRect bypBadge = CGRectMake(slotRect.origin.x + 3, sy + 3, 18, 20);
            if (CGRectContainsPoint(bypBadge, cp)) {
                char nameBuf[256] = {0};
                bool bypassed = false;
                uint32_t osFactor = 1;
                og_native_get_loaded_plugin(self.rust, i, nameBuf, sizeof(nameBuf), &bypassed, &osFactor);
                og_native_set_plugin_bypassed(self.rust, i, !bypassed);
                [self setNeedsDisplay:YES];
                return;
            }

            CGRect preBadge = CGRectMake(CGRectGetMaxX(slotRect) - 86, sy + 3, 16, 20);
            if (CGRectContainsPoint(preBadge, cp)) {
                [self showPresetMenuForSlot:i atPoint:p];
                return;
            }

            CGRect ordBadge = CGRectMake(CGRectGetMaxX(slotRect) - 68, sy + 3, 16, 20);
            if (CGRectContainsPoint(ordBadge, cp)) {
                if (numLoaded > 1) {
                    uint32_t targetIdx = (i + 1) % numLoaded;
                    og_native_exchange_plugins(self.rust, i, targetIdx);
                    [self setNeedsDisplay:YES];
                }
                return;
            }

            CGRect osBadge = CGRectMake(CGRectGetMaxX(slotRect) - 50, sy + 3, 24, 20);
            if (CGRectContainsPoint(osBadge, cp)) {
                char nameBuf[256] = {0};
                bool bypassed = false;
                uint32_t osFactor = 1;
                og_native_get_loaded_plugin(self.rust, i, nameBuf, sizeof(nameBuf), &bypassed, &osFactor);
                uint32_t nextOs = (osFactor == 1) ? 2 : (osFactor == 2) ? 4 : 1;
                og_native_set_plugin_oversampling(self.rust, i, nextOs);
                [self setNeedsDisplay:YES];
                return;
            }

            CGRect delBadge = CGRectMake(CGRectGetMaxX(slotRect) - 23, sy + 3, 20, 20);
            if (CGRectContainsPoint(delBadge, cp)) {
                og_native_unload_plugin(self.rust, i);
                [[OpenGridderRemoteWindowController sharedController] hide];
                [self setNeedsDisplay:YES];
                return;
            }

            if (CGRectContainsPoint(slotRect, cp)) {
                _activeSlot = i;
                NSWindow *win = self.window;
                NSRect winFrame = win ? [win frame] : NSZeroRect;
                int32_t posX = (int32_t)(winFrame.origin.x + winFrame.size.width + 10.0);
                int32_t posY = (int32_t)(winFrame.origin.y + winFrame.size.height);
                og_native_edit_plugin(self.rust, i, posX, posY);

                bool isLocal = og_native_is_server_local_mode(self.rust);
                if (!isLocal) {
                    char slotName[256] = {0};
                    bool slotByp = false;
                    uint32_t slotOs = 1;
                    og_native_get_loaded_plugin(self.rust, i, slotName, sizeof(slotName), &slotByp, &slotOs);
                    NSString *title = [NSString stringWithUTF8String:slotName];
                    [[OpenGridderRemoteWindowController sharedController] showForPlugin:self.rust slotIndex:i title:title];
                } else {
                    [[OpenGridderRemoteWindowController sharedController] hide];
                }
                [self setNeedsDisplay:YES];
                return;
            }
        }

        CGFloat addSy = baseY - numLoaded * (slotH + slotSpacing);
        if (addSy >= 88) {
            CGRect addRect = CGRectMake(rx, addSy, slotW, slotH);
            if (CGRectContainsPoint(addRect, cp)) {
                [self showPluginMenuAtPoint:p];
                return;
            }
        }

        CGRect bypRect = CGRectMake(rx, 48, slotW, 30);
        if (CGRectContainsPoint(bypRect, cp)) {
            _isBypassed = !_isBypassed;
            if (self.rust) og_native_bypass(self.rust, _isBypassed);
            if (self.onParameterChanged) self.onParameterChanged(0, _isBypassed ? 1.0f : 0.0f);
            [self setNeedsDisplay:YES];
            return;
        }
    }

    // =========================================================================
    // 2. Check Bank 2 (Console Input/WetDry/DriveSat/Output) Collapse & Knobs
    // =========================================================================
    if (_consoleCollapsed) {
        if (CGRectContainsPoint(consoleBank, cp)) {
            [self setConsoleCollapsed:NO];
            return;
        }
    } else {
        CGRect consoleFoldBtn = CGRectMake(CGRectGetMaxX(consoleBank) - 32.0, CGRectGetMaxY(consoleBank) - 24.0, 22.0, 17.0);
        if (CGRectContainsPoint(consoleFoldBtn, cp)) {
            [self setConsoleCollapsed:YES];
            return;
        }

        CGFloat cx0 = consoleBank.origin.x + 10.0;
        CGFloat cx1 = consoleBank.origin.x + 140.0;
        CGRect r0 = CGRectMake(cx0, 206, 122, 158);
        CGRect r1 = CGRectMake(cx1, 206, 122, 158);
        CGRect r2 = CGRectMake(cx0, 44,  122, 156);
        CGRect r3 = CGRectMake(cx1, 44,  122, 156);

        if (CGRectContainsPoint(r0, cp)) {
            if (event.clickCount >= 2) {
                _inputGain = 0.5f;
                if (self.rust) og_native_set_param(self.rust, 3, _inputGain);
                if (self.onParameterChanged) self.onParameterChanged(3, _inputGain);
                [self setNeedsDisplay:YES];
                return;
            }
            _draggedKnob = 0; _dragStartPoint = p; _dragStartValue = _inputGain; return;
        }
        if (CGRectContainsPoint(r1, cp)) {
            if (event.clickCount >= 2) {
                _wetDryMix = 1.0f;
                if (self.rust) og_native_set_param(self.rust, 4, _wetDryMix);
                if (self.onParameterChanged) self.onParameterChanged(4, _wetDryMix);
                [self setNeedsDisplay:YES];
                return;
            }
            _draggedKnob = 1; _dragStartPoint = p; _dragStartValue = _wetDryMix; return;
        }
        if (CGRectContainsPoint(r2, cp) && self.rust) {
            CGRect charPill = CGRectMake(r2.origin.x + 8.0, r2.origin.y + 32.0, 50.0, 14.0);
            CGRect qualPill = CGRectMake(CGRectGetMaxX(r2) - 58.0, r2.origin.y + 32.0, 50.0, 14.0);
            if (CGRectContainsPoint(charPill, cp)) {
                int c = ((int)std::lround(og_native_get_param(self.rust, 23) * 2.0f) + 1) % 3;
                og_native_set_param(self.rust, 23, c / 2.0f);
                [self setNeedsDisplay:YES];
                return;
            }
            if (CGRectContainsPoint(qualPill, cp)) {
                int q = ((int)std::lround(og_native_get_param(self.rust, 24) * 2.0f) + 1) % 3;
                og_native_set_param(self.rust, 24, q / 2.0f);
                [self setNeedsDisplay:YES];
                return;
            }
        }
        if (CGRectContainsPoint(r2, cp)) {
            if (event.clickCount >= 2) {
                _drive = 0.0f;
                if (self.rust) og_native_set_param(self.rust, 5, _drive);
                if (self.onParameterChanged) self.onParameterChanged(5, _drive);
                [self setNeedsDisplay:YES];
                return;
            }
            _draggedKnob = 2; _dragStartPoint = p; _dragStartValue = _drive; return;
        }
        if (CGRectContainsPoint(r3, cp)) {
            if (event.clickCount >= 2) {
                _outputGain = 0.5f;
                if (self.rust) og_native_set_param(self.rust, 6, _outputGain);
                if (self.onParameterChanged) self.onParameterChanged(6, _outputGain);
                [self setNeedsDisplay:YES];
                return;
            }
            _draggedKnob = 3; _dragStartPoint = p; _dragStartValue = _outputGain; return;
        }
    }

    // =========================================================================
    // 3. Check Bank 3 (Master Metering Bay) Collapse & VU/PPM 2x Width Toggle
    // =========================================================================
    if (_meterCollapsed) {
        if (CGRectContainsPoint(meterBank, cp)) {
            [self setMeterCollapsed:NO];
            return;
        }
    } else {
        CGFloat innerW = _isVuMeterMode ? 256.0 : 128.0;
        CGFloat innerX = meterBank.origin.x + (meterBank.size.width - innerW) * 0.5;
        CGRect meterBay = CGRectMake(innerX, 44.0, innerW, bounds.size.height - 74.0);
        CGRect meterFoldBtn = CGRectMake(CGRectGetMaxX(meterBay) - 21.0, CGRectGetMaxY(meterBay) - 22.0, 17.0, 16.0);
        if (CGRectContainsPoint(meterFoldBtn, cp)) {
            [self setMeterCollapsed:YES];
            return;
        }
        CGRect modeBtn = CGRectMake(CGRectGetMinX(meterFoldBtn) - 53.0, CGRectGetMaxY(meterBay) - 22.0, 50.0, 16.0);
        if (CGRectContainsPoint(modeBtn, cp)) {
            [self setIsVuMeterMode:!_isVuMeterMode];
            return;
        }
        CGRect ioBtn = CGRectMake(CGRectGetMinX(modeBtn) - 49.0, CGRectGetMaxY(meterBay) - 22.0, 46.0, 16.0);
        if (CGRectContainsPoint(ioBtn, cp)) {
            _isMeterInputMode = !_isMeterInputMode;
            [self setNeedsDisplay:YES];
            return;
        }
        if (_isVuMeterMode) {
            CGRect calBtn = CGRectMake(CGRectGetMinX(ioBtn) - 65.0, CGRectGetMaxY(meterBay) - 22.0, 62.0, 16.0);
            if (CGRectContainsPoint(calBtn, cp)) {
                _isVuCalibratedMinus18 = !_isVuCalibratedMinus18;
                [self setNeedsDisplay:YES];
                return;
            }
        }
        if (CGRectContainsPoint(meterBay, cp) || CGRectContainsPoint(meterBank, cp)) {
            [self setIsVuMeterMode:!_isVuMeterMode];
            return;
        }
    }
}

- (void)mouseDragged:(NSEvent *)event {
    if (_draggedKnob < 0) return;
    NSPoint p = [self convertPoint:event.locationInWindow fromView:nil];
    CGFloat dy = p.y - _dragStartPoint.y;
    float delta = (float)dy / 150.0f;
    float newVal = std::max(0.0f, std::min(1.0f, _dragStartValue + delta));

    uint32_t paramId = 3 + (uint32_t)_draggedKnob;
    if (_draggedKnob == 0) {
        _inputGain = newVal;
    } else if (_draggedKnob == 1) {
        _wetDryMix = newVal;
    } else if (_draggedKnob == 2) {
        _drive = newVal;
    } else if (_draggedKnob == 3) {
        _outputGain = newVal;
    }

    if (self.rust) {
        og_native_set_param(self.rust, paramId, newVal);
    }
    if (self.onParameterChanged) {
        self.onParameterChanged(paramId, newVal);
    }

    [self setNeedsDisplay:YES];
}

- (void)mouseUp:(NSEvent *)event {
    _draggedKnob = -1;
}

- (void)showServerMenuAtPoint:(NSPoint)pt {
    NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Servers"];
    [menu setAutoenablesItems:NO];
    if (self.rust) {
        uint32_t count = og_native_num_discovered_servers(self.rust);
        char activeBuf[128] = {0};
        og_native_get_active_server(self.rust, activeBuf, sizeof(activeBuf));
        NSString *activeStr = [NSString stringWithUTF8String:activeBuf];
        for (uint32_t i = 0; i < count; ++i) {
            char buf[128] = {0};
            if (og_native_get_discovered_server(self.rust, i, buf, sizeof(buf))) {
                NSString *srv = [NSString stringWithUTF8String:buf];
                NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:srv action:@selector(onSelectServer:) keyEquivalent:@""];
                [item setTarget:self];
                [item setRepresentedObject:srv];
                if ([srv isEqualToString:activeStr]) {
                    [item setState:1];
                }
                [menu addItem:item];
            }
        }
    }
    [menu popUpMenuPositioningItem:nil atLocation:pt inView:self];
}

- (void)onSelectServer:(NSMenuItem *)sender {
    if (!self.rust) return;
    NSString *srv = sender.representedObject;
    if (srv.length > 0) {
        og_native_set_server(self.rust, [srv UTF8String]);
        _cachedPluginMenu = nil;
        _cachedPluginCount = 0;
        [self setNeedsDisplay:YES];
    }
}

- (void)showPresetMenuForSlot:(uint32_t)slotIdx atPoint:(NSPoint)pt {
    NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Presets"];
    [menu setAutoenablesItems:NO];
    uint32_t numPresets = self.rust ? og_native_num_presets(self.rust, slotIdx) : 0;
    if (numPresets == 0) {
        NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:@"Default" action:nil keyEquivalent:@""];
        [item setEnabled:NO];
        [menu addItem:item];
    } else {
        for (uint32_t p = 0; p < numPresets; ++p) {
            char pBuf[256] = {0};
            if (og_native_get_preset_name(self.rust, slotIdx, p, pBuf, sizeof(pBuf))) {
                NSString *pName = [NSString stringWithUTF8String:pBuf];
                if (pName.length == 0) pName = [NSString stringWithFormat:@"Preset %u", p + 1];
                NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:pName action:@selector(onSelectPreset:) keyEquivalent:@""];
                [item setTarget:self];
                [item setTag:(NSInteger)((slotIdx << 16) | (p & 0xFFFF))];
                [menu addItem:item];
            }
        }
    }
    [menu popUpMenuPositioningItem:nil atLocation:pt inView:self];
}

- (void)onSelectPreset:(NSMenuItem *)sender {
    if (!self.rust) return;
    uint32_t packed = (uint32_t)sender.tag;
    uint32_t slotIdx = packed >> 16;
    uint32_t presetIdx = packed & 0xFFFF;
    og_native_set_preset(self.rust, slotIdx, presetIdx);
    [self setNeedsDisplay:YES];
}

- (void)showPluginMenuAtPoint:(NSPoint)pt {
    if (!self.rust) {
        NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Available Plugins"];
        [menu setAutoenablesItems:NO];
        NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:@"Connecting to server (127.0.0.1:55056)..." action:nil keyEquivalent:@""];
        [item setEnabled:NO];
        [menu addItem:item];
        [menu popUpMenuPositioningItem:nil atLocation:pt inView:self];
        return;
    }
    uint32_t numAvail = og_native_num_available_plugins(self.rust);

    if (numAvail == 0) {
        NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Available Plugins"];
        [menu setAutoenablesItems:NO];
        NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:@"No plugins available on server" action:nil keyEquivalent:@""];
        [item setEnabled:NO];
        [menu addItem:item];
        [menu popUpMenuPositioningItem:nil atLocation:pt inView:self];
        return;
    }

    if (!_cachedPluginMenu || _cachedPluginCount != numAvail) {
        _cachedPluginMenu = [[NSMenu alloc] initWithTitle:@"Available Plugins"];
        [_cachedPluginMenu setAutoenablesItems:NO];

        NSMenuItem *searchItem = [[NSMenuItem alloc] initWithTitle:[NSString stringWithFormat:@"Search %u Plugins…", numAvail]
                                                            action:@selector(onSearchPlugins:)
                                                     keyEquivalent:@"f"];
        [searchItem setTarget:self];
        [searchItem setEnabled:YES];
        [_cachedPluginMenu addItem:searchItem];
        [_cachedPluginMenu addItem:[NSMenuItem separatorItem]];

        // Format submenus: "AudioUnit", "VST3", "VST"
        NSMutableDictionary *formatMenus = [NSMutableDictionary dictionary];
        // Nested (Format -> Company) submenus
        NSMutableDictionary *companyMenus = [NSMutableDictionary dictionary];
        // Nested (Format -> Company -> Category) submenus
        NSMutableDictionary *categoryMenus = [NSMutableDictionary dictionary];

        NSArray *formatOrder = @[@"AudioUnit", @"VST3", @"CLAP", @"LV2", @"VST"];
        for (NSString *fmt in formatOrder) {
            NSMenu *fmtMenu = [[NSMenu alloc] initWithTitle:fmt];
            [fmtMenu setAutoenablesItems:NO];
            NSMenuItem *fmtItem = [[NSMenuItem alloc] initWithTitle:fmt action:nil keyEquivalent:@""];
            [fmtItem setSubmenu:fmtMenu];
            [_cachedPluginMenu addItem:fmtItem];
            formatMenus[fmt] = fmtMenu;
        }

        for (uint32_t i = 0; i < numAvail; ++i) {
            char nameBuf[256] = {0};
            char idBuf[256] = {0};
            char typeBuf[64] = {0};
            char compBuf[128] = {0};
            char catBuf[128] = {0};

            og_native_get_available_plugin_ext(self.rust, i,
                                               nameBuf, sizeof(nameBuf),
                                               idBuf, sizeof(idBuf),
                                               typeBuf, sizeof(typeBuf),
                                               compBuf, sizeof(compBuf),
                                               catBuf, sizeof(catBuf));

            NSString *name = [NSString stringWithUTF8String:nameBuf];
            NSString *fmt = [NSString stringWithUTF8String:typeBuf];
            NSString *comp = [NSString stringWithUTF8String:compBuf];
            NSString *cat = [NSString stringWithUTF8String:catBuf];

            if (fmt.length == 0) fmt = @"AudioUnit";
            if (comp.length == 0) comp = @"General";
            if (cat.length == 0) cat = @"Effect";

            NSMenu *targetFmtMenu = formatMenus[fmt];
            if (!targetFmtMenu) {
                targetFmtMenu = [[NSMenu alloc] initWithTitle:fmt];
                [targetFmtMenu setAutoenablesItems:NO];
                NSMenuItem *fmtItem = [[NSMenuItem alloc] initWithTitle:fmt action:nil keyEquivalent:@""];
                [fmtItem setEnabled:YES];
                [fmtItem setSubmenu:targetFmtMenu];
                [_cachedPluginMenu addItem:fmtItem];
                formatMenus[fmt] = targetFmtMenu;
            }

            NSString *fmtCompKey = [NSString stringWithFormat:@"%@::%@", fmt, comp];
            NSMenu *targetCompMenu = companyMenus[fmtCompKey];
            if (!targetCompMenu) {
                targetCompMenu = [[NSMenu alloc] initWithTitle:comp];
                [targetCompMenu setAutoenablesItems:NO];
                NSMenuItem *compItem = [[NSMenuItem alloc] initWithTitle:comp action:nil keyEquivalent:@""];
                [compItem setEnabled:YES];
                [compItem setSubmenu:targetCompMenu];
                [targetFmtMenu addItem:compItem];
                companyMenus[fmtCompKey] = targetCompMenu;
            }

            NSMenu *finalMenu = targetCompMenu;
            if (cat.length > 0 && ![cat isEqualToString:@"Effect"] && ![cat isEqualToString:@"General"]) {
                NSString *fmtCompCatKey = [NSString stringWithFormat:@"%@::%@::%@", fmt, comp, cat];
                NSMenu *targetCatMenu = categoryMenus[fmtCompCatKey];
                if (!targetCatMenu) {
                    targetCatMenu = [[NSMenu alloc] initWithTitle:cat];
                    [targetCatMenu setAutoenablesItems:NO];
                    NSMenuItem *catItem = [[NSMenuItem alloc] initWithTitle:cat action:nil keyEquivalent:@""];
                    [catItem setEnabled:YES];
                    [catItem setSubmenu:targetCatMenu];
                    [targetCompMenu addItem:catItem];
                    categoryMenus[fmtCompCatKey] = targetCatMenu;
                }
                finalMenu = targetCatMenu;
            }

            NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:name action:@selector(onSelectPlugin:) keyEquivalent:@""];
            [item setTarget:self];
            [item setTag:(NSInteger)i];
            [item setEnabled:YES];
            [finalMenu addItem:item];
        }

        // Clean up empty format menus if no plugins exist for that format
        for (NSString *fmt in formatOrder) {
            NSMenu *fmtMenu = formatMenus[fmt];
            if (fmtMenu.numberOfItems == 0) {
                NSInteger idx = [_cachedPluginMenu indexOfItemWithSubmenu:fmtMenu];
                if (idx >= 0) {
                    [_cachedPluginMenu removeItemAtIndex:idx];
                }
            }
        }

        _cachedPluginCount = numAvail;
    }

    [_cachedPluginMenu popUpMenuPositioningItem:nil atLocation:pt inView:self];
}

- (BOOL)validateMenuItem:(NSMenuItem *)menuItem {
    return YES;
}

- (void)onSelectPlugin:(NSMenuItem *)sender {
    [self addPluginAtIndex:(uint32_t)sender.tag title:sender.title];
}

- (void)onSearchPlugins:(id)sender {
    [[OpenGridderPluginSearch sharedSearch] showForEditor:self];
}

// Load catalog entry `idx` into a new slot (from the menu or the search panel).
- (void)addPluginAtIndex:(uint32_t)idx title:(NSString *)pluginTitle {
    if (!self.rust) return;
    NSWindow *win = self.window;
    NSRect winFrame = win ? [win frame] : NSZeroRect;
    int32_t posX = (int32_t)(winFrame.origin.x + winFrame.size.width + 10.0);
    int32_t posY = (int32_t)(winFrame.origin.y + winFrame.size.height);
    uint32_t newSlot = og_native_num_loaded_plugins(self.rust);
    og_native_load_plugin_at(self.rust, idx, posX, posY);
    _activeSlot = (int)newSlot;

    bool isLocal = og_native_is_server_local_mode(self.rust);
    if (!isLocal) {
        NSString *title = pluginTitle ?: @"Remote Plugin";
        [[OpenGridderRemoteWindowController sharedController] showForPlugin:self.rust slotIndex:newSlot title:title];
    } else {
        [[OpenGridderRemoteWindowController sharedController] hide];
    }
    [self setNeedsDisplay:YES];
}

- (void)rightMouseDown:(NSEvent *)event {
    if (!self.rust) return;
    NSPoint p = [self convertPoint:event.locationInWindow fromView:nil];
    CGRect bounds = NSRectToCGRect(self.bounds);
    uint32_t numLoaded = og_native_num_loaded_plugins(self.rust);
    CGFloat slotH = 26;
    CGFloat slotSpacing = 4;
    CGFloat baseY = bounds.size.height - 230;

    for (uint32_t i = 0; i < numLoaded; ++i) {
        CGFloat sy = baseY - i * (slotH + slotSpacing);
        CGRect slotRect = CGRectMake(40, sy, 208, slotH);
        if (CGRectContainsPoint(slotRect, NSPointToCGPoint(p))) {
            NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Slot Context"];
            [menu setAutoenablesItems:NO];

            // Parameters submenu
            NSMenuItem *paramRoot = [[NSMenuItem alloc] initWithTitle:@"Parameters" action:nil keyEquivalent:@""];
            NSMenu *paramMenu = [[NSMenu alloc] initWithTitle:@"Parameters"];
            uint32_t numParams = og_native_num_plugin_params(self.rust, i);
            uint32_t maxShown = std::min(numParams, 64u);
            if (maxShown == 0) {
                NSMenuItem *noneItem = [[NSMenuItem alloc] initWithTitle:@"No parameters exposed" action:nil keyEquivalent:@""];
                [noneItem setEnabled:NO];
                [paramMenu addItem:noneItem];
            } else {
                for (uint32_t prm = 0; prm < maxShown; ++prm) {
                    char pName[128] = {0};
                    float defVal = 0.0f, curVal = 0.0f;
                    if (og_native_get_plugin_param_info(self.rust, i, prm, pName, sizeof(pName), &defVal, &curVal)) {
                        NSString *title = [NSString stringWithFormat:@"%s (%.2f)", pName, curVal];
                        NSMenuItem *pItem = [[NSMenuItem alloc] initWithTitle:title action:nil keyEquivalent:@""];
                        NSMenu *valMenu = [[NSMenu alloc] initWithTitle:title];
                        for (int step = 0; step <= 4; ++step) {
                            float v = (float)step * 0.25f;
                            NSString *vStr = [NSString stringWithFormat:@"%.0f%%", v * 100.0f];
                            NSMenuItem *vItem = [[NSMenuItem alloc] initWithTitle:vStr action:@selector(onSetRemoteParam:) keyEquivalent:@""];
                            [vItem setTarget:self];
                            [vItem setTag:(NSInteger)(((i & 0xFF) << 24) | ((prm & 0xFFFF) << 8) | (step & 0xFF))];
                            [valMenu addItem:vItem];
                        }
                        [pItem setSubmenu:valMenu];
                        [paramMenu addItem:pItem];
                    }
                }
            }
            [paramRoot setSubmenu:paramMenu];
            [menu addItem:paramRoot];
            [menu popUpMenuPositioningItem:nil atLocation:p inView:self];
            return;
        }
    }
}

- (void)onSetRemoteParam:(NSMenuItem *)sender {
    if (!self.rust) return;
    uint32_t packed = (uint32_t)sender.tag;
    uint32_t slotIdx = (packed >> 24) & 0xFF;
    uint32_t paramIdx = (packed >> 8) & 0xFFFF;
    uint32_t step = packed & 0xFF;
    float val = std::max(0.0f, std::min(1.0f, (float)step * 0.25f));
    og_native_set_plugin_param(self.rust, slotIdx, paramIdx, val);
    [self setNeedsDisplay:YES];
}

@end


#pragma mark - Plugin Search

// A floating panel to find a plugin by typing (AudioGridder's plugin search window).
// A separate window, not a menu, so the DAW does not swallow the keystrokes.
@interface OpenGridderPluginSearchEntry : NSObject
@property (nonatomic) uint32_t index;
@property (nonatomic, copy) NSString *name, *company, *format, *category, *haystack;
@end
@implementation OpenGridderPluginSearchEntry
@end

@implementation OpenGridderPluginSearch {
    __weak OpenGridderEditorView *_editor;
    NSSearchField *_field;
    NSTableView *_table;
    NSTextField *_countLabel;
    NSArray<OpenGridderPluginSearchEntry *> *_all;
    NSArray<OpenGridderPluginSearchEntry *> *_shown;
}

+ (instancetype)sharedSearch {
    static OpenGridderPluginSearch *shared = nil;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ shared = [[OpenGridderPluginSearch alloc] init]; });
    return shared;
}

- (instancetype)init {
    NSPanel *panel = [[NSPanel alloc] initWithContentRect:NSMakeRect(0, 0, 560, 420)
                                                styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                                                           NSWindowStyleMaskResizable | NSWindowStyleMaskUtilityWindow)
                                                  backing:NSBackingStoreBuffered
                                                    defer:NO];
    panel.title = @"Add Plugin";
    panel.level = NSFloatingWindowLevel;
    panel.releasedWhenClosed = NO;
    panel.hidesOnDeactivate = NO;
    panel.becomesKeyOnlyIfNeeded = NO;
    panel.minSize = NSMakeSize(360, 220);
    if ((self = [super initWithWindow:panel])) {
        NSView *content = panel.contentView;
        _field = [[NSSearchField alloc] initWithFrame:NSMakeRect(10, 384, 540, 26)];
        _field.autoresizingMask = NSViewWidthSizable | NSViewMinYMargin;
        _field.placeholderString = @"Search name, company, format…";
        _field.delegate = self;
        _field.sendsSearchStringImmediately = YES;
        [content addSubview:_field];

        NSScrollView *scroll = [[NSScrollView alloc] initWithFrame:NSMakeRect(10, 30, 540, 346)];
        scroll.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        scroll.hasVerticalScroller = YES;
        scroll.borderType = NSBezelBorder;
        _table = [[NSTableView alloc] initWithFrame:scroll.bounds];
        NSArray *columns = @[@[@"name", @"Plugin", @250], @[@"company", @"Company", @170], @[@"format", @"Format", @80]];
        for (NSArray *spec in columns) {
            NSTableColumn *column = [[NSTableColumn alloc] initWithIdentifier:spec[0]];
            column.title = spec[1];
            column.width = [spec[2] doubleValue];
            [_table addTableColumn:column];
        }
        _table.dataSource = self;
        _table.delegate = self;
        _table.usesAlternatingRowBackgroundColors = YES;
        _table.allowsEmptySelection = NO;
        _table.target = self;
        _table.doubleAction = @selector(addSelected:);
        scroll.documentView = _table;
        [content addSubview:scroll];

        _countLabel = [NSTextField labelWithString:@""];
        _countLabel.frame = NSMakeRect(10, 6, 540, 18);
        _countLabel.autoresizingMask = NSViewWidthSizable | NSViewMaxYMargin;
        _countLabel.textColor = [NSColor secondaryLabelColor];
        _countLabel.font = [NSFont systemFontOfSize:11];
        [content addSubview:_countLabel];
    }
    return self;
}

- (void)showForEditor:(OpenGridderEditorView *)editor {
    _editor = editor;
    NSMutableArray *entries = [NSMutableArray array];
    NativePlugin *rust = editor.rust;
    const uint32_t count = rust ? og_native_num_available_plugins(rust) : 0;
    for (uint32_t i = 0; i < count; ++i) {
        char name[256] = {0}, idBuf[1024] = {0}, type[64] = {0}, company[128] = {0}, category[128] = {0};
        if (!og_native_get_available_plugin_ext(rust, i, name, sizeof name, idBuf, sizeof idBuf, type, sizeof type,
                                                company, sizeof company, category, sizeof category)) {
            continue;
        }
        OpenGridderPluginSearchEntry *entry = [[OpenGridderPluginSearchEntry alloc] init];
        entry.index = i;
        entry.name = [NSString stringWithUTF8String:name] ?: @"";
        entry.company = [NSString stringWithUTF8String:company] ?: @"";
        entry.format = [NSString stringWithUTF8String:type] ?: @"";
        entry.category = [NSString stringWithUTF8String:category] ?: @"";
        entry.haystack = [[NSString stringWithFormat:@"%@ %@ %@ %@", entry.name, entry.company, entry.format, entry.category] lowercaseString];
        [entries addObject:entry];
    }
    [entries sortUsingComparator:^NSComparisonResult(OpenGridderPluginSearchEntry *a, OpenGridderPluginSearchEntry *b) {
        return [a.name localizedCaseInsensitiveCompare:b.name];
    }];
    _all = entries;
    [self applyFilter];

    NSWindow *host = editor.window;
    if (host) {
        NSRect frame = host.frame;
        [self.window setFrameTopLeftPoint:NSMakePoint(NSMinX(frame) + 40, NSMaxY(frame) - 40)];
    } else {
        [self.window center];
    }
    [self.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
    [self.window makeFirstResponder:_field];
    [_field selectText:nil];
}

// Every typed word must appear in the name, company, format or category.
// Names that start with the first word come first.
- (void)applyFilter {
    NSString *query = [_field.stringValue.lowercaseString stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
    NSArray *terms = [query componentsSeparatedByCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
    terms = [terms filteredArrayUsingPredicate:[NSPredicate predicateWithFormat:@"length > 0"]];
    if (terms.count == 0) {
        _shown = _all;
    } else {
        NSMutableArray *starts = [NSMutableArray array], *others = [NSMutableArray array];
        for (OpenGridderPluginSearchEntry *entry in _all) {
            BOOL match = YES;
            for (NSString *term in terms) {
                if ([entry.haystack rangeOfString:term].location == NSNotFound) { match = NO; break; }
            }
            if (!match) continue;
            if ([entry.name.lowercaseString hasPrefix:terms.firstObject]) [starts addObject:entry];
            else [others addObject:entry];
        }
        [starts addObjectsFromArray:others];
        _shown = starts;
    }
    [_table reloadData];
    if (_shown.count > 0) {
        [_table selectRowIndexes:[NSIndexSet indexSetWithIndex:0] byExtendingSelection:NO];
        [_table scrollRowToVisible:0];
    }
    _countLabel.stringValue = [NSString stringWithFormat:@"%lu of %lu plugins — Return adds the selected one",
                               (unsigned long)_shown.count, (unsigned long)_all.count];
}

- (void)controlTextDidChange:(NSNotification *)note {
    [self applyFilter];
}

// Arrow keys move the selection while typing; Return adds; Esc closes.
- (BOOL)control:(NSControl *)control textView:(NSTextView *)textView doCommandBySelector:(SEL)command {
    NSInteger row = _table.selectedRow;
    if (command == @selector(moveDown:)) {
        if (row + 1 < (NSInteger)_shown.count) {
            [_table selectRowIndexes:[NSIndexSet indexSetWithIndex:row + 1] byExtendingSelection:NO];
            [_table scrollRowToVisible:row + 1];
        }
        return YES;
    }
    if (command == @selector(moveUp:)) {
        if (row > 0) {
            [_table selectRowIndexes:[NSIndexSet indexSetWithIndex:row - 1] byExtendingSelection:NO];
            [_table scrollRowToVisible:row - 1];
        }
        return YES;
    }
    if (command == @selector(insertNewline:)) {
        [self addSelected:nil];
        return YES;
    }
    if (command == @selector(cancelOperation:)) {
        if (_field.stringValue.length > 0) {
            _field.stringValue = @"";
            [self applyFilter];
        } else {
            [self.window orderOut:nil];
        }
        return YES;
    }
    return NO;
}

- (void)addSelected:(id)sender {
    NSInteger row = _table.selectedRow;
    if (row < 0 || row >= (NSInteger)_shown.count) return;
    OpenGridderPluginSearchEntry *entry = _shown[(NSUInteger)row];
    OpenGridderEditorView *editor = _editor;
    [self.window orderOut:nil];
    if (editor) {
        [editor addPluginAtIndex:entry.index title:entry.name];
    }
}

- (NSInteger)numberOfRowsInTableView:(NSTableView *)tableView {
    return (NSInteger)_shown.count;
}

- (id)tableView:(NSTableView *)tableView objectValueForTableColumn:(NSTableColumn *)column row:(NSInteger)row {
    if (row < 0 || row >= (NSInteger)_shown.count) return nil;
    OpenGridderPluginSearchEntry *entry = _shown[(NSUInteger)row];
    if ([column.identifier isEqualToString:@"company"]) return entry.company;
    if ([column.identifier isEqualToString:@"format"]) return entry.format;
    return entry.name;
}

@end
