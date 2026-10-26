// OpenGridder Server native macOS Application with GoodLookinDri Hardware UI
// Pure AppKit / Cocoa implementation - Zero-Overhead Pure-Native.
#import <Cocoa/Cocoa.h>
#import <ApplicationServices/ApplicationServices.h>
#import <signal.h>
#import <unistd.h>

#pragma mark - GoodLookin Hardware Drawing Helper

static void DrawGoodLookinPlate(CGContextRef ctx, CGRect bounds) {
    // 1. Brushed dark metallic gradient
    CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
    CGFloat topColor[4] = {0.15f, 0.17f, 0.20f, 1.0f};
    CGFloat botColor[4] = {0.09f, 0.10f, 0.12f, 1.0f};
    CGFloat colors[8] = {
        topColor[0], topColor[1], topColor[2], topColor[3],
        botColor[0], botColor[1], botColor[2], botColor[3]
    };
    CGGradientRef grad = CGGradientCreateWithColorComponents(cs, colors, NULL, 2);
    CGContextDrawLinearGradient(ctx, grad, CGPointMake(0, bounds.size.height), CGPointMake(0, 0), 0);
    CGGradientRelease(grad);
    CGColorSpaceRelease(cs);

    // 2. Beveled border
    CGContextSetRGBStrokeColor(ctx, 0.35f, 0.39f, 0.44f, 0.6f);
    CGContextSetLineWidth(ctx, 1.5f);
    CGContextStrokeRect(ctx, CGRectInset(bounds, 1.5, 1.5));

    // 3. Hardware hex screws in 4 corners
    void (^drawScrew)(CGFloat, CGFloat) = ^(CGFloat x, CGFloat y) {
        // Shadow
        CGContextSetRGBFillColor(ctx, 0.05f, 0.05f, 0.06f, 0.6f);
        CGContextFillEllipseInRect(ctx, CGRectMake(x - 7, y - 7, 14, 14));
        // Screw head
        CGContextSetRGBFillColor(ctx, 0.24f, 0.26f, 0.29f, 1.0f);
        CGContextFillEllipseInRect(ctx, CGRectMake(x - 6, y - 6, 12, 12));
        // Outer ring
        CGContextSetRGBStrokeColor(ctx, 0.45f, 0.50f, 0.55f, 0.9f);
        CGContextSetLineWidth(ctx, 1.0f);
        CGContextStrokeEllipseInRect(ctx, CGRectMake(x - 6, y - 6, 12, 12));
        // Hex slot
        CGContextSetRGBStrokeColor(ctx, 0.10f, 0.11f, 0.13f, 1.0f);
        CGContextSetLineWidth(ctx, 1.5f);
        CGContextMoveToPoint(ctx, x - 3.5, y);
        CGContextAddLineToPoint(ctx, x + 3.5, y);
        CGContextStrokePath(ctx);
        CGContextMoveToPoint(ctx, x, y - 3.5);
        CGContextAddLineToPoint(ctx, x, y + 3.5);
        CGContextStrokePath(ctx);
    };

    drawScrew(16, 16);
    drawScrew(bounds.size.width - 16, 16);
    drawScrew(16, bounds.size.height - 16);
    drawScrew(bounds.size.width - 16, bounds.size.height - 16);
}

#pragma mark - Custom Hardware View

@interface GoodLookinPlateView : NSView
@end

@implementation GoodLookinPlateView
- (void)drawRect:(NSRect)dirtyRect {
    [super drawRect:dirtyRect];
    CGContextRef ctx = [[NSGraphicsContext currentContext] CGContext];
    DrawGoodLookinPlate(ctx, NSRectToCGRect(self.bounds));
}
@end

#pragma mark - Splash Window

@interface SplashWindowController : NSWindowController
@property (strong, nonatomic) NSTextField *titleLabel;
@property (strong, nonatomic) NSTextField *subtitleLabel;
@property (strong, nonatomic) NSTextField *versionLabel;
@property (strong, nonatomic) NSTextField *statusLabel;
@property (strong, nonatomic) NSProgressIndicator *progressBar;
@property (strong, nonatomic) NSButton *cancelButton;
@property (copy, nonatomic) void (^onCancel)(void);
- (void)setProgress:(double)fraction text:(NSString *)text;
- (void)setCompletedWithTotal:(NSInteger)total port:(NSInteger)port;
@end

@implementation SplashWindowController

- (instancetype)init {
    NSRect frame = NSMakeRect(0, 0, 720, 380);
    NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                   styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskFullSizeContentView
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    [window setTitlebarAppearsTransparent:YES];
    [window setTitleVisibility:NSWindowTitleHidden];
    [window center];
    [window setOpaque:YES];
    [window setHasShadow:YES];
    [window setLevel:NSFloatingWindowLevel];

    self = [super initWithWindow:window];
    if (self) {
        GoodLookinPlateView *plate = [[GoodLookinPlateView alloc] initWithFrame:frame];
        plate.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [window setContentView:plate];

        // Prominent Logo (145x145 with high-DPI scaling)
        NSImageView *logoView = [[NSImageView alloc] initWithFrame:NSMakeRect(35, 195, 145, 145)];
        [logoView setImageScaling:NSImageScaleProportionallyUpOrDown];
        [logoView setImageAlignment:NSImageAlignCenter];
        NSImage *icon = [NSImage imageNamed:@"OpenGridderServer"];
        if (!icon) {
            NSString *iconPath = [[NSBundle mainBundle] pathForResource:@"OpenGridderServer" ofType:@"icns"];
            if (iconPath) icon = [[NSImage alloc] initWithContentsOfFile:iconPath];
        }
        logoView.image = icon;
        [plate addSubview:logoView];

        // Title
        self.titleLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(195, 285, 480, 40)];
        self.titleLabel.editable = NO;
        self.titleLabel.bordered = NO;
        self.titleLabel.backgroundColor = [NSColor clearColor];
        self.titleLabel.font = [NSFont systemFontOfSize:28 weight:NSFontWeightBold];
        self.titleLabel.textColor = [NSColor colorWithCalibratedRed:0.95 green:0.95 blue:0.97 alpha:1.0];
        self.titleLabel.stringValue = @"OPENGRIDDER SERVER";
        [plate addSubview:self.titleLabel];

        // Subtitle
        self.subtitleLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(195, 255, 480, 26)];
        self.subtitleLabel.editable = NO;
        self.subtitleLabel.bordered = NO;
        self.subtitleLabel.backgroundColor = [NSColor clearColor];
        self.subtitleLabel.font = [NSFont systemFontOfSize:14 weight:NSFontWeightMedium];
        self.subtitleLabel.textColor = [NSColor colorWithCalibratedRed:0.95 green:0.75 blue:0.25 alpha:1.0]; // GoodLookin Amber
        self.subtitleLabel.stringValue = @"Zero-Overhead Precision Audio Grid • GoodLookin UI Engine";
        [plate addSubview:self.subtitleLabel];

        // Version & Arch
        self.versionLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(195, 228, 480, 22)];
        self.versionLabel.editable = NO;
        self.versionLabel.bordered = NO;
        self.versionLabel.backgroundColor = [NSColor clearColor];
        self.versionLabel.font = [NSFont systemFontOfSize:12 weight:NSFontWeightRegular];
        self.versionLabel.textColor = [NSColor colorWithCalibratedRed:0.60 green:0.65 blue:0.70 alpha:0.8];
        self.versionLabel.stringValue = @"v0.3.2 • macOS Universal (Apple Silicon & Intel)";
        [plate addSubview:self.versionLabel];

        // Progress bar
        self.progressBar = [[NSProgressIndicator alloc] initWithFrame:NSMakeRect(45, 140, 630, 16)];
        self.progressBar.indeterminate = NO;
        self.progressBar.minValue = 0.0;
        self.progressBar.maxValue = 1.0;
        self.progressBar.doubleValue = 0.0;
        [plate addSubview:self.progressBar];

        // Status message
        self.statusLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(45, 95, 510, 28)];
        self.statusLabel.editable = NO;
        self.statusLabel.bordered = NO;
        self.statusLabel.backgroundColor = [NSColor clearColor];
        self.statusLabel.font = [NSFont monospacedSystemFontOfSize:12 weight:NSFontWeightRegular];
        self.statusLabel.textColor = [NSColor colorWithCalibratedRed:0.85 green:0.88 blue:0.92 alpha:1.0];
        self.statusLabel.stringValue = @"Initializing plugin discovery...";
        [plate addSubview:self.statusLabel];

        // Skip / Cancel button
        self.cancelButton = [[NSButton alloc] initWithFrame:NSMakeRect(575, 92, 100, 32)];
        self.cancelButton.bezelStyle = NSBezelStyleRounded;
        self.cancelButton.title = @"Skip Scan";
        self.cancelButton.target = self;
        self.cancelButton.action = @selector(cancelClicked:);
        [plate addSubview:self.cancelButton];

        // Footer note
        NSTextField *footer = [[NSTextField alloc] initWithFrame:NSMakeRect(45, 26, 630, 18)];
        footer.editable = NO;
        footer.bordered = NO;
        footer.backgroundColor = [NSColor clearColor];
        footer.font = [NSFont systemFontOfSize:11 weight:NSFontWeightLight];
        footer.textColor = [NSColor colorWithCalibratedRed:0.45 green:0.48 blue:0.52 alpha:0.8];
        footer.stringValue = @"OpenGrid Network Architecture • GoodLookin Hardware UI by Marcos Deida";
        [plate addSubview:footer];
    }
    return self;
}

- (void)setProgress:(double)fraction text:(NSString *)text {
    dispatch_async(dispatch_get_main_queue(), ^{
        self.progressBar.doubleValue = fraction;
        self.statusLabel.stringValue = text ?: @"";
    });
}

- (void)setCompletedWithTotal:(NSInteger)total port:(NSInteger)port {
    dispatch_async(dispatch_get_main_queue(), ^{
        self.progressBar.doubleValue = 1.0;
        self.statusLabel.textColor = [NSColor colorWithCalibratedRed:0.35 green:0.90 blue:0.45 alpha:1.0]; // Bright Green
        self.statusLabel.stringValue = [NSString stringWithFormat:@"Ready: %ld plugins active on port %ld", (long)total, (long)port];
        self.cancelButton.hidden = YES;

        // Auto close after 1.2s
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.2 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            [NSAnimationContext runAnimationGroup:^(NSAnimationContext *context) {
                context.duration = 0.4;
                self.window.animator.alphaValue = 0.0;
            } completionHandler:^{
                [self.window orderOut:nil];
                self.window.alphaValue = 1.0;
            }];
        });
    });
}

- (void)cancelClicked:(id)sender {
    if (self.onCancel) {
        self.onCancel();
    }
    [self.window orderOut:nil];
}

@end

#pragma mark - About Window Controller

@interface AboutWindowController : NSWindowController
@property (strong, nonatomic) NSTextView *textView;
@property (strong, nonatomic) NSScrollView *scrollView;
@property (strong, nonatomic) NSTimer *scrollTimer;
@property (assign, nonatomic) BOOL autoScrollEnabled;
@property (strong, nonatomic) NSButton *scrollToggleButton;
@property (assign, nonatomic) NSTimeInterval lastUserInteraction;
- (void)loadContent;
- (void)startScrollTimer;
- (void)stopScrollTimer;
@end

@implementation AboutWindowController

- (instancetype)init {
    NSRect frame = NSMakeRect(0, 0, 720, 640);
    NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                   styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    window.title = @"About OpenGridder Server";
    [window center];
    self = [super initWithWindow:window];
    if (self) {
        _autoScrollEnabled = YES;
        _lastUserInteraction = 0;

        GoodLookinPlateView *plate = [[GoodLookinPlateView alloc] initWithFrame:frame];
        plate.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [window setContentView:plate];

        // Header matching Splash Screen with Prominent 135x135 Logo
        NSImageView *logoView = [[NSImageView alloc] initWithFrame:NSMakeRect(35, 475, 135, 135)];
        [logoView setImageScaling:NSImageScaleProportionallyUpOrDown];
        [logoView setImageAlignment:NSImageAlignCenter];
        NSImage *icon = [NSImage imageNamed:@"OpenGridderServer"];
        if (!icon) {
            NSString *iconPath = [[NSBundle mainBundle] pathForResource:@"OpenGridderServer" ofType:@"icns"];
            if (iconPath) icon = [[NSImage alloc] initWithContentsOfFile:iconPath];
        }
        logoView.image = icon;
        [plate addSubview:logoView];

        NSTextField *titleLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(185, 560, 490, 36)];
        titleLabel.editable = NO;
        titleLabel.bordered = NO;
        titleLabel.backgroundColor = [NSColor clearColor];
        titleLabel.font = [NSFont systemFontOfSize:26 weight:NSFontWeightBold];
        titleLabel.textColor = [NSColor colorWithCalibratedRed:0.95 green:0.95 blue:0.97 alpha:1.0];
        titleLabel.stringValue = @"OPENGRIDDER SERVER";
        [plate addSubview:titleLabel];

        NSTextField *subtitleLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(185, 532, 490, 24)];
        subtitleLabel.editable = NO;
        subtitleLabel.bordered = NO;
        subtitleLabel.backgroundColor = [NSColor clearColor];
        subtitleLabel.font = [NSFont systemFontOfSize:13 weight:NSFontWeightMedium];
        subtitleLabel.textColor = [NSColor colorWithCalibratedRed:0.95 green:0.75 blue:0.25 alpha:1.0]; // GoodLookin Amber
        subtitleLabel.stringValue = @"Zero-Overhead Precision Audio Grid • GoodLookin UI Engine";
        [plate addSubview:subtitleLabel];

        NSTextField *versionLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(185, 508, 490, 20)];
        versionLabel.editable = NO;
        versionLabel.bordered = NO;
        versionLabel.backgroundColor = [NSColor clearColor];
        versionLabel.font = [NSFont systemFontOfSize:11 weight:NSFontWeightRegular];
        versionLabel.textColor = [NSColor colorWithCalibratedRed:0.60 green:0.65 blue:0.70 alpha:0.8];
        versionLabel.stringValue = @"v0.3.2 • macOS Universal (Apple Silicon & Intel)";
        [plate addSubview:versionLabel];

        NSTextField *tagline = [[NSTextField alloc] initWithFrame:NSMakeRect(185, 486, 490, 18)];
        tagline.editable = NO;
        tagline.bordered = NO;
        tagline.backgroundColor = [NSColor clearColor];
        tagline.font = [NSFont systemFontOfSize:10 weight:NSFontWeightLight];
        tagline.textColor = [NSColor colorWithCalibratedRed:0.45 green:0.75 blue:0.85 alpha:0.8];
        tagline.stringValue = @"Motorola DSP56321 4-Stream TDM Engine • Focusrite Control|24 & Avid C|24";
        [plate addSubview:tagline];

        // Recessed Container for Scrolling Text
        NSRect scrollFrame = NSMakeRect(45, 75, 630, 390);
        self.scrollView = [[NSScrollView alloc] initWithFrame:scrollFrame];
        self.scrollView.borderType = NSBezelBorder;
        self.scrollView.hasVerticalScroller = YES;
        self.scrollView.hasHorizontalScroller = NO;
        self.scrollView.autohidesScrollers = NO;

        self.textView = [[NSTextView alloc] initWithFrame:self.scrollView.bounds];
        self.textView.editable = NO;
        self.textView.selectable = YES;
        self.textView.backgroundColor = [NSColor colorWithCalibratedRed:0.08 green:0.09 blue:0.11 alpha:1.0];
        self.textView.textColor = [NSColor colorWithCalibratedRed:0.88 green:0.90 blue:0.92 alpha:1.0];
        self.textView.font = [NSFont monospacedSystemFontOfSize:11 weight:NSFontWeightRegular];
        self.textView.textContainerInset = NSMakeSize(16, 16);

        // Populate text content with README & CREDITS
        [self loadContent];

        self.scrollView.documentView = self.textView;
        [plate addSubview:self.scrollView];

        // Bottom Controls
        self.scrollToggleButton = [[NSButton alloc] initWithFrame:NSMakeRect(45, 25, 140, 30)];
        self.scrollToggleButton.bezelStyle = NSBezelStyleRounded;
        self.scrollToggleButton.title = @"Auto-Scroll: ON";
        self.scrollToggleButton.target = self;
        self.scrollToggleButton.action = @selector(toggleAutoScroll:);
        [plate addSubview:self.scrollToggleButton];

        NSButton *repoBtn = [[NSButton alloc] initWithFrame:NSMakeRect(195, 25, 160, 30)];
        repoBtn.bezelStyle = NSBezelStyleRounded;
        repoBtn.title = @"GitHub Repository";
        repoBtn.target = self;
        repoBtn.action = @selector(openGitHub:);
        [plate addSubview:repoBtn];

        NSButton *closeBtn = [[NSButton alloc] initWithFrame:NSMakeRect(575, 25, 100, 30)];
        closeBtn.bezelStyle = NSBezelStyleRounded;
        closeBtn.title = @"Close";
        closeBtn.target = self;
        closeBtn.action = @selector(closeClicked:);
        [plate addSubview:closeBtn];

        // Start Auto-Scroll timer
        [self startScrollTimer];
    }
    return self;
}

- (void)loadContent {
    NSString *rawText = 
@"================================================================================\n"
@"                 OPENGRIDDER — ZERO-OVERHEAD PRECISION AUDIO GRID              \n"
@"                         Version 0.3.2 • Multi-Format                         \n"
@"================================================================================\n\n"
@"[ ARCHITECTURE & OVERVIEW ]\n"
@"OpenGridder is an ultra-high-performance, zero-overhead, pure-native audio streaming\n"
@"grid and distributed DSP processing engine. It seamlessly bridges studio DAWs\n"
@"with remote and local DSP resources with sub-millisecond precision, lock-free\n"
@"ring buffering, and hardware-inspired analog console ergonomics.\n\n"
@"[ KEY SYSTEM FEATURES ]\n"
@"• Multi-Stream DSP Core Sharing (Motorola DSP56321 TDM Engine):\n"
@"  Expanded assembly firmware executes 4 concurrent stereo streams per DSP core\n"
@"  (8 audio channels per core, up to 72 stereo instances per 18-core Pro Tools HD\n"
@"  Accel card) using internal 24-bit 64k RAM ring buffering.\n\n"
@"• Hardware Console Integration (Focusrite Control|24 & Avid C|24):\n"
@"  Native 24-channel rotary encoder and motorized fader mapping via custom 'FrTL'\n"
@"  Pro Tools XML Page Tables, plus ICON D-Control/D-Command ('HgTL'), ProControl\n"
@"  ('PcTL'), and Command|8/HUI ('MkTL') tactile console control.\n\n"
@"• Zero-Overhead Pure-Native Architecture:\n"
@"  Pure Rust audio streaming core paired with native C/C++/Objective-C bindings\n"
@"  directly to Steinberg VST3, Apple AudioUnit, and Avid AAX SDKs with zero\n"
@"  runtime bloat and zero intermediate wrapper overhead.\n\n"
@"• GoodLookin Hardware Console UI:\n"
@"  3-bank collapsible brushed anodized aluminum rack, 2x-wide precision analog\n"
@"  ballistic VU meters (864x420), dual VU calibration (-18 dBFS European broadcast\n"
@"  and -3 dBFS headroom test), and real-time [IN | OUT] source toggles.\n\n"
@"• Residual ADAA & Polyphase Oversampling:\n"
@"  HybridEQ-matched non-linear drive saturation with anti-derivative anti-aliasing\n"
@"  (ADAA) and selectable 1x, 2x, and 4x linear-phase polyphase oversampling.\n\n"
@"• Universal Format Support:\n"
@"  AAX Native, AudioUnit v2 (Component Manager 'thng' & Cocoa UI), AudioUnit v3\n"
@"  (macOS App Extension), VST 3.7 Universal, VST 2.4, CLAP, and LV2.\n\n"
@"================================================================================\n"
@"                            CREDITS & ATTRIBUTION                              \n"
@"================================================================================\n\n"
@"• Marcos Deida (@MistaMin)\n"
@"  Lead Architect & Developer: Core network streaming architecture, distributed\n"
@"  DSP processing engine, multi-stream Motorola DSP56321 assembly firmware, Pro Tools\n"
@"  AAX and hardware console integrations (Control|24 & C|24), and creator of\n"
@"  GoodLookinUI, used for hardware-inspired analog console panels, precision\n"
@"  ballistic VU meters, and rotary controls.\n\n"
@"• OpenGrid Precision Audio Network\n"
@"  Ultra-low latency network audio transport, lock-free ring buffering, remote\n"
@"  screen compression, and dynamic delay compensation across modern macOS,\n"
@"  Windows, and vintage studio systems.\n\n"
@"================================================================================\n"
@"                   (c) 2026 Marcos Deida • studio.opengrid                     \n"
@"================================================================================\n";

    self.textView.string = rawText;
}

- (void)startScrollTimer {
    [self stopScrollTimer];
    self.scrollTimer = [NSTimer scheduledTimerWithTimeInterval:1.0 / 30.0
                                                        target:self
                                                      selector:@selector(scrollTick:)
                                                      userInfo:nil
                                                       repeats:YES];
}

- (void)stopScrollTimer {
    if (self.scrollTimer) {
        [self.scrollTimer invalidate];
        self.scrollTimer = nil;
    }
}

- (void)scrollTick:(NSTimer *)timer {
    if (!self.autoScrollEnabled) return;
    if ([[NSDate date] timeIntervalSince1970] - self.lastUserInteraction < 2.5) return;

    NSClipView *clip = self.scrollView.contentView;
    NSPoint current = clip.bounds.origin;
    CGFloat maxY = self.textView.bounds.size.height - clip.bounds.size.height;
    if (maxY <= 0) return;

    if (current.y >= maxY - 2.0) {
        current.y = 0;
        self.lastUserInteraction = [[NSDate date] timeIntervalSince1970] + 1.5;
    } else {
        current.y += 0.6;
    }
    [clip scrollToPoint:current];
    [self.scrollView reflectScrolledClipView:clip];
}

- (void)toggleAutoScroll:(id)sender {
    self.autoScrollEnabled = !self.autoScrollEnabled;
    self.scrollToggleButton.title = self.autoScrollEnabled ? @"Auto-Scroll: ON" : @"Auto-Scroll: OFF";
}

- (void)openGitHub:(id)sender {
    [[NSWorkspace sharedWorkspace] openURL:[NSURL URLWithString:@"https://github.com/MistaMin/OpenGridder-Rust"]];
}

- (void)closeClicked:(id)sender {
    [self stopScrollTimer];
    [self.window orderOut:nil];
}

- (void)windowWillClose:(NSNotification *)notification {
    [self stopScrollTimer];
}

@end

#pragma mark - Plugin Manager Window Controller

@interface PluginListWindowController : NSWindowController <NSTableViewDataSource, NSTableViewDelegate, NSSearchFieldDelegate>
@property (strong, nonatomic) NSArray<NSDictionary *> *allPlugins;
@property (strong, nonatomic) NSArray<NSDictionary *> *filteredPlugins;
@property (strong, nonatomic) NSTableView *tableView;
@property (strong, nonatomic) NSSearchField *searchField;
@property (strong, nonatomic) NSSegmentedControl *formatFilter;
@property (strong, nonatomic) NSTextField *countLabel;
@property (copy, nonatomic) void (^onRescanRequested)(void);
- (void)updatePlugins:(NSArray<NSDictionary *> *)plugins;
@end

@implementation PluginListWindowController

- (instancetype)init {
    NSRect frame = NSMakeRect(0, 0, 840, 560);
    NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                   styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    window.title = @"OpenGridder — Plugin Manager";
    window.minSize = NSMakeSize(680, 400);
    [window center];

    self = [super initWithWindow:window];
    if (self) {
        _allPlugins = @[];
        _filteredPlugins = @[];

        GoodLookinPlateView *content = [[GoodLookinPlateView alloc] initWithFrame:frame];
        content.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [window setContentView:content];

        // Search Field
        self.searchField = [[NSSearchField alloc] initWithFrame:NSMakeRect(30, frame.size.height - 60, 320, 28)];
        self.searchField.placeholderString = @"Filter by plugin name, vendor, format...";
        self.searchField.delegate = self;
        self.searchField.autoresizingMask = NSViewMinYMargin | NSViewMaxXMargin;
        [content addSubview:self.searchField];

        // Format Filter Segmented Control
        self.formatFilter = [NSSegmentedControl segmentedControlWithLabels:@[@"All", @"AudioUnit", @"VST3", @"VST2"]
                                                              trackingMode:NSSegmentSwitchTrackingSelectOne
                                                                    target:self
                                                                    action:@selector(filterChanged:)];
        self.formatFilter.selectedSegment = 0;
        self.formatFilter.frame = NSMakeRect(370, frame.size.height - 60, 260, 28);
        self.formatFilter.autoresizingMask = NSViewMinYMargin | NSViewMaxXMargin;
        [content addSubview:self.formatFilter];

        // Rescan Button
        NSButton *rescanBtn = [[NSButton alloc] initWithFrame:NSMakeRect(frame.size.width - 160, frame.size.height - 60, 130, 28)];
        rescanBtn.bezelStyle = NSBezelStyleRounded;
        rescanBtn.title = @"Rescan All";
        rescanBtn.target = self;
        rescanBtn.action = @selector(rescanClicked:);
        rescanBtn.autoresizingMask = NSViewMinYMargin | NSViewMinXMargin;
        [content addSubview:rescanBtn];

        // Table View ScrollView
        NSScrollView *scrollView = [[NSScrollView alloc] initWithFrame:NSMakeRect(30, 65, frame.size.width - 60, frame.size.height - 135)];
        scrollView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        scrollView.hasVerticalScroller = YES;
        scrollView.hasHorizontalScroller = NO;
        scrollView.borderType = NSBezelBorder;

        self.tableView = [[NSTableView alloc] initWithFrame:scrollView.bounds];
        self.tableView.dataSource = self;
        self.tableView.delegate = self;
        self.tableView.usesAlternatingRowBackgroundColors = YES;
        self.tableView.rowHeight = 24;

        NSTableColumn *colName = [[NSTableColumn alloc] initWithIdentifier:@"name"];
        colName.title = @"Plugin Name";
        colName.width = 240;
        [self.tableView addTableColumn:colName];

        NSTableColumn *colFmt = [[NSTableColumn alloc] initWithIdentifier:@"format"];
        colFmt.title = @"Format";
        colFmt.width = 110;
        [self.tableView addTableColumn:colFmt];

        NSTableColumn *colVendor = [[NSTableColumn alloc] initWithIdentifier:@"company"];
        colVendor.title = @"Manufacturer";
        colVendor.width = 180;
        [self.tableView addTableColumn:colVendor];

        NSTableColumn *colCat = [[NSTableColumn alloc] initWithIdentifier:@"category"];
        colCat.title = @"Category";
        colCat.width = 130;
        [self.tableView addTableColumn:colCat];

        NSTableColumn *colStatus = [[NSTableColumn alloc] initWithIdentifier:@"status"];
        colStatus.title = @"Status";
        colStatus.width = 90;
        [self.tableView addTableColumn:colStatus];

        scrollView.documentView = self.tableView;
        [content addSubview:scrollView];

        // Footer Count
        self.countLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(30, 24, 500, 22)];
        self.countLabel.editable = NO;
        self.countLabel.bordered = NO;
        self.countLabel.backgroundColor = [NSColor clearColor];
        self.countLabel.font = [NSFont systemFontOfSize:12 weight:NSFontWeightMedium];
        self.countLabel.textColor = [NSColor colorWithCalibratedRed:0.80 green:0.85 blue:0.90 alpha:0.9];
        self.countLabel.stringValue = @"0 plugins discovered";
        self.countLabel.autoresizingMask = NSViewMaxYMargin | NSViewWidthSizable;
        [content addSubview:self.countLabel];

        // Close Button
        NSButton *closeBtn = [[NSButton alloc] initWithFrame:NSMakeRect(frame.size.width - 130, 20, 100, 28)];
        closeBtn.bezelStyle = NSBezelStyleRounded;
        closeBtn.title = @"Close";
        closeBtn.target = self;
        closeBtn.action = @selector(closeClicked:);
        closeBtn.autoresizingMask = NSViewMaxYMargin | NSViewMinXMargin;
        [content addSubview:closeBtn];
    }
    return self;
}

- (void)updatePlugins:(NSArray<NSDictionary *> *)plugins {
    _allPlugins = plugins ?: @[];
    [self applyFilter];
}

- (void)applyFilter {
    NSString *search = [self.searchField.stringValue lowercaseString];
    NSInteger seg = self.formatFilter.selectedSegment;

    NSMutableArray *filtered = [NSMutableArray array];
    NSInteger auCount = 0;
    NSInteger vst3Count = 0;
    NSInteger vst2Count = 0;

    for (NSDictionary *p in self.allPlugins) {
        NSString *name = p[@"name"] ?: @"";
        NSString *vendor = p[@"company"] ?: @"";
        NSString *fmt = p[@"type"] ?: @"AudioUnit";
        NSString *cat = p[@"category"] ?: @"";

        if ([fmt containsString:@"AudioUnit"] || [fmt containsString:@"AU"]) auCount++;
        else if ([fmt containsString:@"VST3"]) vst3Count++;
        else if ([fmt containsString:@"VST2"] || [fmt isEqualToString:@"VST"]) vst2Count++;

        if (seg == 1 && ![fmt containsString:@"AudioUnit"] && ![fmt containsString:@"AU"]) continue;
        if (seg == 2 && ![fmt containsString:@"VST3"]) continue;
        if (seg == 3 && ([fmt containsString:@"VST3"] || (![fmt containsString:@"VST2"] && ![fmt isEqualToString:@"VST"]))) continue;

        if (search.length > 0) {
            NSString *full = [NSString stringWithFormat:@"%@ %@ %@ %@", name, vendor, fmt, cat].lowercaseString;
            if (![full containsString:search]) continue;
        }
        [filtered addObject:p];
    }
    self.filteredPlugins = filtered;
    [self.tableView reloadData];

    self.countLabel.stringValue = [NSString stringWithFormat:@"Showing %ld of %ld plugins (%ld AudioUnits, %ld VST3, %ld VST2)",
                                  (long)self.filteredPlugins.count, (long)self.allPlugins.count, (long)auCount, (long)vst3Count, (long)vst2Count];
}

- (void)controlTextDidChange:(NSNotification *)obj {
    [self applyFilter];
}

- (void)filterChanged:(id)sender {
    [self applyFilter];
}

- (void)rescanClicked:(id)sender {
    if (self.onRescanRequested) {
        self.onRescanRequested();
    }
}

- (void)closeClicked:(id)sender {
    [self.window orderOut:nil];
}

#pragma mark - Table View Data Source & Delegate

- (NSInteger)numberOfRowsInTableView:(NSTableView *)tableView {
    return self.filteredPlugins.count;
}

- (NSView *)tableView:(NSTableView *)tableView viewForTableColumn:(NSTableColumn *)tableColumn row:(NSInteger)row {
    if (row < 0 || row >= self.filteredPlugins.count) return nil;
    NSDictionary *p = self.filteredPlugins[row];

    NSString *identifier = tableColumn.identifier;
    NSTextField *cell = [tableView makeViewWithIdentifier:identifier owner:self];
    if (!cell) {
        cell = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 0, tableColumn.width, 22)];
        cell.identifier = identifier;
        cell.bordered = NO;
        cell.editable = NO;
        cell.backgroundColor = [NSColor clearColor];
    }

    if ([identifier isEqualToString:@"name"]) {
        cell.font = [NSFont systemFontOfSize:12 weight:NSFontWeightSemibold];
        cell.textColor = [NSColor whiteColor];
        cell.stringValue = p[@"name"] ?: @"Unknown";
    } else if ([identifier isEqualToString:@"format"]) {
        cell.font = [NSFont systemFontOfSize:11 weight:NSFontWeightMedium];
        NSString *fmt = p[@"type"] ?: @"AudioUnit";
        if ([fmt containsString:@"VST3"]) {
            cell.textColor = [NSColor colorWithCalibratedRed:0.35 green:0.80 blue:1.0 alpha:1.0]; // Cyan
            cell.stringValue = @"VST3";
        } else if ([fmt containsString:@"VST2"] || [fmt isEqualToString:@"VST"]) {
            cell.textColor = [NSColor colorWithCalibratedRed:0.95 green:0.55 blue:0.95 alpha:1.0]; // Magenta
            cell.stringValue = @"VST2";
        } else {
            cell.textColor = [NSColor colorWithCalibratedRed:1.0 green:0.80 blue:0.35 alpha:1.0]; // Amber
            cell.stringValue = @"AudioUnit";
        }
    } else if ([identifier isEqualToString:@"company"]) {
        cell.font = [NSFont systemFontOfSize:11 weight:NSFontWeightRegular];
        cell.textColor = [NSColor colorWithCalibratedWhite:0.85 alpha:1.0];
        cell.stringValue = p[@"company"] ?: @"-";
    } else if ([identifier isEqualToString:@"category"]) {
        cell.font = [NSFont systemFontOfSize:11 weight:NSFontWeightRegular];
        cell.textColor = [NSColor colorWithCalibratedWhite:0.75 alpha:1.0];
        cell.stringValue = p[@"category"] ?: @"Effect";
    } else if ([identifier isEqualToString:@"status"]) {
        cell.font = [NSFont systemFontOfSize:11 weight:NSFontWeightBold];
        cell.textColor = [NSColor colorWithCalibratedRed:0.35 green:0.90 blue:0.45 alpha:1.0];
        cell.stringValue = @"● Active";
    }
    return cell;
}

@end

#pragma mark - Server Settings Window Controller

@interface SettingsWindowController : NSWindowController
@property (strong, nonatomic) NSPopUpButton *captureModePopup;
@property (strong, nonatomic) NSButton *diffDetectionCheck;
@property (strong, nonatomic) NSButton *alwaysOnTopCheck;
@property (strong, nonatomic) NSButton *auCheck;
@property (strong, nonatomic) NSButton *vst3Check;
@property (strong, nonatomic) NSButton *vst2Check;
@property (strong, nonatomic) NSButton *lv2Check;
@property (strong, nonatomic) NSTextField *serverNameField;
@property (strong, nonatomic) NSTextField *serverIdField;
@property (strong, nonatomic) NSButton *scanAtStartupCheck;
@property (strong, nonatomic) NSString *configPath;
@property (weak, nonatomic) id delegate;
- (void)loadCurrentSettings;
- (void)saveAndApply;
@end

@implementation SettingsWindowController

- (instancetype)init {
    NSRect frame = NSMakeRect(0, 0, 540, 500);
    NSWindow *win = [[NSWindow alloc] initWithContentRect:frame
                                                styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable)
                                                  backing:NSBackingStoreBuffered
                                                    defer:NO];
    win.title = @"OpenGridder Server Settings";
    win.appearance = [NSAppearance appearanceNamed:NSAppearanceNameVibrantDark];

    self = [super initWithWindow:win];
    if (self) {
        NSString *home = NSHomeDirectory();
        self.configPath = [home stringByAppendingPathComponent:@".audiogridder/audiogridderserver.cfg"];
        [self buildUI];
    }
    return self;
}

- (void)buildUI {
    NSView *content = self.window.contentView;
    content.wantsLayer = YES;

    CGFloat y = 450.0;

    // Header Title
    NSTextField *hdr = [NSTextField labelWithString:@"Server Preferences"];
    hdr.frame = NSMakeRect(24, y, 400, 24);
    hdr.font = [NSFont systemFontOfSize:16 weight:NSFontWeightBold];
    hdr.textColor = [NSColor colorWithCalibratedRed:1.0 green:0.75 blue:0.25 alpha:1.0]; // Amber
    [content addSubview:hdr];

    y -= 38.0;

    // Section 1: Screen Capturing & Local Mode
    NSTextField *sec1 = [NSTextField labelWithString:@"SCREEN CAPTURING & DISPLAY MODE"];
    sec1.frame = NSMakeRect(24, y, 480, 18);
    sec1.font = [NSFont systemFontOfSize:11 weight:NSFontWeightBold];
    sec1.textColor = [NSColor colorWithCalibratedWhite:0.65 alpha:1.0];
    [content addSubview:sec1];

    y -= 28.0;
    NSTextField *capLbl = [NSTextField labelWithString:@"Capture Mode:"];
    capLbl.frame = NSMakeRect(28, y, 100, 20);
    capLbl.font = [NSFont systemFontOfSize:12 weight:NSFontWeightMedium];
    capLbl.textColor = [NSColor whiteColor];
    [content addSubview:capLbl];

    self.captureModePopup = [[NSPopUpButton alloc] initWithFrame:NSMakeRect(135, y - 2, 375, 24) pullsDown:NO];
    [self.captureModePopup addItemWithTitle:@"Local Mode (Direct Native Window, Zero-Overhead)"];
    [self.captureModePopup addItemWithTitle:@"Native Diff Stream (Remote LAN / Software Capture)"];
    [self.captureModePopup addItemWithTitle:@"Disabled (Headless Audio Server / No Display)"];
    [content addSubview:self.captureModePopup];

    y -= 26.0;
    self.diffDetectionCheck = [NSButton checkboxWithTitle:@"Enable Screen Diff Detection (Reduces LAN bandwidth)" target:nil action:nil];
    self.diffDetectionCheck.frame = NSMakeRect(135, y, 375, 20);
    self.diffDetectionCheck.font = [NSFont systemFontOfSize:11 weight:NSFontWeightRegular];
    [content addSubview:self.diffDetectionCheck];

    y -= 24.0;
    self.alwaysOnTopCheck = [NSButton checkboxWithTitle:@"Keep Plugin Windows Always On Top in Local Mode" target:nil action:nil];
    self.alwaysOnTopCheck.frame = NSMakeRect(135, y, 375, 20);
    self.alwaysOnTopCheck.font = [NSFont systemFontOfSize:11 weight:NSFontWeightRegular];
    [content addSubview:self.alwaysOnTopCheck];

    y -= 36.0;

    // Section 2: Plugin Formats
    NSTextField *sec2 = [NSTextField labelWithString:@"HOSTED PLUGIN FORMATS"];
    sec2.frame = NSMakeRect(24, y, 480, 18);
    sec2.font = [NSFont systemFontOfSize:11 weight:NSFontWeightBold];
    sec2.textColor = [NSColor colorWithCalibratedWhite:0.65 alpha:1.0];
    [content addSubview:sec2];

    y -= 26.0;
    self.auCheck = [NSButton checkboxWithTitle:@"AudioUnit (AU)" target:nil action:nil];
    self.auCheck.frame = NSMakeRect(28, y, 115, 20);
    [content addSubview:self.auCheck];

    self.vst3Check = [NSButton checkboxWithTitle:@"VST3" target:nil action:nil];
    self.vst3Check.frame = NSMakeRect(150, y, 80, 20);
    [content addSubview:self.vst3Check];

    self.vst2Check = [NSButton checkboxWithTitle:@"VST2" target:nil action:nil];
    self.vst2Check.frame = NSMakeRect(240, y, 80, 20);
    [content addSubview:self.vst2Check];

    self.lv2Check = [NSButton checkboxWithTitle:@"LV2" target:nil action:nil];
    self.lv2Check.frame = NSMakeRect(330, y, 80, 20);
    [content addSubview:self.lv2Check];

    y -= 36.0;

    // Section 3: Server Coordination
    NSTextField *sec3 = [NSTextField labelWithString:@"SERVER IDENTIFICATION & STARTUP"];
    sec3.frame = NSMakeRect(24, y, 480, 18);
    sec3.font = [NSFont systemFontOfSize:11 weight:NSFontWeightBold];
    sec3.textColor = [NSColor colorWithCalibratedWhite:0.65 alpha:1.0];
    [content addSubview:sec3];

    y -= 28.0;
    NSTextField *nameLbl = [NSTextField labelWithString:@"Server Name:"];
    nameLbl.frame = NSMakeRect(28, y, 100, 20);
    nameLbl.font = [NSFont systemFontOfSize:12 weight:NSFontWeightMedium];
    nameLbl.textColor = [NSColor whiteColor];
    [content addSubview:nameLbl];

    self.serverNameField = [[NSTextField alloc] initWithFrame:NSMakeRect(135, y, 260, 22)];
    [content addSubview:self.serverNameField];

    y -= 28.0;
    NSTextField *idLbl = [NSTextField labelWithString:@"Server ID:"];
    idLbl.frame = NSMakeRect(28, y, 100, 20);
    idLbl.font = [NSFont systemFontOfSize:12 weight:NSFontWeightMedium];
    idLbl.textColor = [NSColor whiteColor];
    [content addSubview:idLbl];

    self.serverIdField = [[NSTextField alloc] initWithFrame:NSMakeRect(135, y, 60, 22)];
    [content addSubview:self.serverIdField];

    NSTextField *portHelp = [NSTextField labelWithString:@"(Base Port 55056 + Server ID)"];
    portHelp.frame = NSMakeRect(205, y + 2, 250, 18);
    portHelp.font = [NSFont systemFontOfSize:11 weight:NSFontWeightRegular];
    portHelp.textColor = [NSColor colorWithCalibratedWhite:0.65 alpha:1.0];
    [content addSubview:portHelp];

    y -= 26.0;
    self.scanAtStartupCheck = [NSButton checkboxWithTitle:@"Scan for new & updated plugins on server startup" target:nil action:nil];
    self.scanAtStartupCheck.frame = NSMakeRect(135, y, 375, 20);
    self.scanAtStartupCheck.font = [NSFont systemFontOfSize:11 weight:NSFontWeightRegular];
    [content addSubview:self.scanAtStartupCheck];

    // Bottom Action Buttons
    NSButton *cancelBtn = [[NSButton alloc] initWithFrame:NSMakeRect(320, 20, 90, 30)];
    cancelBtn.title = @"Cancel";
    cancelBtn.bezelStyle = NSBezelStyleRounded;
    cancelBtn.target = self;
    cancelBtn.action = @selector(onCancel:);
    [content addSubview:cancelBtn];

    NSButton *saveBtn = [[NSButton alloc] initWithFrame:NSMakeRect(420, 20, 100, 30)];
    saveBtn.title = @"Save & Apply";
    saveBtn.bezelStyle = NSBezelStyleRounded;
    saveBtn.keyEquivalent = @"\r";
    saveBtn.target = self;
    saveBtn.action = @selector(onSave:);
    [content addSubview:saveBtn];
}

- (void)loadCurrentSettings {
    NSDictionary *cfg = nil;
    if ([[NSFileManager defaultManager] fileExistsAtPath:self.configPath]) {
        NSData *data = [NSData dataWithContentsOfFile:self.configPath];
        if (data) {
            cfg = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
        }
    }

    BOOL slm = cfg ? [cfg[@"ScreenLocalMode"] boolValue] : YES;
    BOOL sco = cfg ? [cfg[@"ScreenCapturingOff"] boolValue] : NO;
    if (slm) {
        [self.captureModePopup selectItemAtIndex:0]; // Local Mode
    } else if (sco) {
        [self.captureModePopup selectItemAtIndex:2]; // Disabled
    } else {
        [self.captureModePopup selectItemAtIndex:1]; // Remote LAN
    }

    self.diffDetectionCheck.state = (!cfg || [cfg[@"ScreenDiffDetection"] boolValue]) ? NSControlStateValueOn : NSControlStateValueOff;
    self.alwaysOnTopCheck.state = (cfg && [cfg[@"PluginWindowsOnTop"] boolValue]) ? NSControlStateValueOn : NSControlStateValueOff;

    self.auCheck.state = (!cfg || [cfg[@"AU"] boolValue]) ? NSControlStateValueOn : NSControlStateValueOff;
    self.vst3Check.state = (!cfg || [cfg[@"VST3"] boolValue]) ? NSControlStateValueOn : NSControlStateValueOff;
    self.vst2Check.state = (!cfg || [cfg[@"VST2"] boolValue]) ? NSControlStateValueOn : NSControlStateValueOff;
    self.lv2Check.state = (!cfg || [cfg[@"LV2"] boolValue]) ? NSControlStateValueOn : NSControlStateValueOff;

    NSString *name = cfg[@"NAME"] ?: [[NSHost currentHost] localizedName];
    self.serverNameField.stringValue = name ?: @"OpenGridder Server";
    self.serverIdField.stringValue = [NSString stringWithFormat:@"%ld", (long)[cfg[@"ID"] integerValue]];
    self.scanAtStartupCheck.state = (!cfg || [cfg[@"ScanForPlugins"] boolValue]) ? NSControlStateValueOn : NSControlStateValueOff;
}

- (void)onCancel:(id)sender {
    [self.window close];
}

- (void)onSave:(id)sender {
    [self saveAndApply];
    [self.window close];
}

- (void)saveAndApply {
    NSMutableDictionary *cfg = [NSMutableDictionary dictionary];
    if ([[NSFileManager defaultManager] fileExistsAtPath:self.configPath]) {
        NSData *data = [NSData dataWithContentsOfFile:self.configPath];
        if (data) {
            NSDictionary *existing = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
            if (existing) [cfg addEntriesFromDictionary:existing];
        }
    }

    NSInteger modeIdx = self.captureModePopup.indexOfSelectedItem;
    cfg[@"ScreenLocalMode"] = @(modeIdx == 0);
    cfg[@"ScreenCapturingOff"] = @(modeIdx == 2);
    cfg[@"ScreenDiffDetection"] = @(self.diffDetectionCheck.state == NSControlStateValueOn);
    cfg[@"PluginWindowsOnTop"] = @(self.alwaysOnTopCheck.state == NSControlStateValueOn);

    cfg[@"AU"] = @(self.auCheck.state == NSControlStateValueOn);
    cfg[@"VST3"] = @(self.vst3Check.state == NSControlStateValueOn);
    cfg[@"VST2"] = @(self.vst2Check.state == NSControlStateValueOn);
    cfg[@"LV2"] = @(self.lv2Check.state == NSControlStateValueOn);

    cfg[@"NAME"] = self.serverNameField.stringValue ?: @"OpenGridder Server";
    cfg[@"ID"] = @([self.serverIdField.stringValue integerValue]);
    cfg[@"ScanForPlugins"] = @(self.scanAtStartupCheck.state == NSControlStateValueOn);

    NSString *dir = [self.configPath stringByDeletingLastPathComponent];
    [[NSFileManager defaultManager] createDirectoryAtPath:dir withIntermediateDirectories:YES attributes:nil error:nil];

    NSData *outData = [NSJSONSerialization dataWithJSONObject:cfg options:NSJSONWritingPrettyPrinted error:nil];
    if (outData) {
        [outData writeToFile:self.configPath atomically:YES];
    }

    // Also write to old config path ~/.audiogridderserver if needed
    NSString *oldPath = [NSHomeDirectory() stringByAppendingPathComponent:@".audiogridderserver"];
    if ([[NSFileManager defaultManager] fileExistsAtPath:oldPath]) {
        [outData writeToFile:oldPath atomically:YES];
    }

    if ([self.delegate respondsToSelector:@selector(restartServer:)]) {
        [self.delegate performSelector:@selector(restartServer:) withObject:nil];
    }
}

@end

#pragma mark - Main Application Delegate

@interface OpenGridderAppDelegate : NSObject <NSApplicationDelegate>
@property (strong, nonatomic) NSStatusItem *statusItem;
@property (strong, nonatomic) NSTask *serverTask;
@property (strong, nonatomic) NSString *logPath;
@property (strong, nonatomic) NSMenuItem *statusMenuItem;
@property (strong, nonatomic) NSMenuItem *pluginCountMenuItem;
@property (strong, nonatomic) SplashWindowController *splashController;
@property (strong, nonatomic) AboutWindowController *aboutController;
@property (strong, nonatomic) PluginListWindowController *pluginListController;
@property (strong, nonatomic) SettingsWindowController *settingsController;
@property (strong, nonatomic) NSArray<NSDictionary *> *catalog;
@property (nonatomic) NSInteger serverPort;
@end

@implementation OpenGridderAppDelegate

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    // 1. Prepare log directory and file
    NSString *appSupport = [NSSearchPathForDirectoriesInDomains(NSLibraryDirectory, NSUserDomainMask, YES) firstObject];
    NSString *logDir = [appSupport stringByAppendingPathComponent:@"Logs/OpenGridder"];
    [[NSFileManager defaultManager] createDirectoryAtPath:logDir withIntermediateDirectories:YES attributes:nil error:nil];
    self.logPath = [logDir stringByAppendingPathComponent:@"server.log"];
    self.serverPort = 55056;

    // 1b. Request Accessibility Permissions for remote mouse/keyboard synthetic event forwarding
    [self requestAccessibilityPermissions];

    // 2. Setup System Status Bar Item
    self.statusItem = [[NSStatusBar systemStatusBar] statusItemWithLength:NSVariableStatusItemLength];
    NSImage *icon = [NSImage imageNamed:@"OpenGridderServer"];
    if (!icon) {
        NSString *iconPath = [[NSBundle mainBundle] pathForResource:@"OpenGridderServer" ofType:@"icns"];
        if (iconPath) icon = [[NSImage alloc] initWithContentsOfFile:iconPath];
    }
    if (icon) {
        [icon setSize:NSMakeSize(18, 18)];
        self.statusItem.button.image = icon;
        self.statusItem.button.imagePosition = NSImageLeft;
    }
    self.statusItem.button.title = @" OpenGridder";

    NSMenu *menu = [[NSMenu alloc] initWithTitle:@"OpenGridder Server"];
    NSMenuItem *aboutItem = [[NSMenuItem alloc] initWithTitle:@"About OpenGridder Server..." action:@selector(showAbout:) keyEquivalent:@""];
    [aboutItem setTarget:self];
    [menu addItem:aboutItem];

    NSMenuItem *settingsItem = [[NSMenuItem alloc] initWithTitle:@"Server Settings..." action:@selector(showSettings:) keyEquivalent:@","];
    [settingsItem setTarget:self];
    [menu addItem:settingsItem];

    [menu addItem:[NSMenuItem separatorItem]];

    self.statusMenuItem = [[NSMenuItem alloc] initWithTitle:@"OpenGridder Server: Starting..." action:nil keyEquivalent:@""];
    [self.statusMenuItem setEnabled:NO];
    [menu addItem:self.statusMenuItem];

    self.pluginCountMenuItem = [[NSMenuItem alloc] initWithTitle:@"Plugins: Discovering..." action:nil keyEquivalent:@""];
    [self.pluginCountMenuItem setEnabled:NO];
    [menu addItem:self.pluginCountMenuItem];

    [menu addItem:[NSMenuItem separatorItem]];

    NSMenuItem *pluginManagerItem = [[NSMenuItem alloc] initWithTitle:@"Plugin Manager..." action:@selector(showPluginManager:) keyEquivalent:@"p"];
    [pluginManagerItem setTarget:self];
    [menu addItem:pluginManagerItem];

    NSMenuItem *rescanItem = [[NSMenuItem alloc] initWithTitle:@"Rescan All Plugins..." action:@selector(rescanAll:) keyEquivalent:@"r"];
    [rescanItem setTarget:self];
    [menu addItem:rescanItem];

    [menu addItem:[NSMenuItem separatorItem]];

    NSMenuItem *logsItem = [[NSMenuItem alloc] initWithTitle:@"View Server Logs..." action:@selector(openLogs:) keyEquivalent:@"l"];
    [logsItem setTarget:self];
    [menu addItem:logsItem];

    NSMenuItem *docsItem = [[NSMenuItem alloc] initWithTitle:@"Open User Manual (PDF)..." action:@selector(openDocs:) keyEquivalent:@"d"];
    [docsItem setTarget:self];
    [menu addItem:docsItem];

    NSMenuItem *configItem = [[NSMenuItem alloc] initWithTitle:@"Open Config Folder..." action:@selector(openConfig:) keyEquivalent:@""];
    [configItem setTarget:self];
    [menu addItem:configItem];

    NSMenuItem *accessItem = [[NSMenuItem alloc] initWithTitle:@"Accessibility Permissions..." action:@selector(openAccessibilitySettings:) keyEquivalent:@""];
    [accessItem setTarget:self];
    [menu addItem:accessItem];

    [menu addItem:[NSMenuItem separatorItem]];

    NSMenuItem *restartItem = [[NSMenuItem alloc] initWithTitle:@"Restart Server" action:@selector(restartServer:) keyEquivalent:@""];
    [restartItem setTarget:self];
    [menu addItem:restartItem];

    NSMenuItem *quitItem = [[NSMenuItem alloc] initWithTitle:@"Quit OpenGridder Server" action:@selector(quitApp:) keyEquivalent:@"q"];
    [quitItem setTarget:self];
    [menu addItem:quitItem];

    self.statusItem.menu = menu;

    // Standard Application Menu Bar
    NSMenu *mainMenu = [[NSMenu alloc] init];
    NSMenuItem *appMenuItem = [[NSMenuItem alloc] init];
    [mainMenu addItem:appMenuItem];
    NSMenu *appMenu = [[NSMenu alloc] initWithTitle:@"OpenGridder Server"];
    NSMenuItem *appAboutItem = [[NSMenuItem alloc] initWithTitle:@"About OpenGridder Server" action:@selector(showAbout:) keyEquivalent:@""];
    [appAboutItem setTarget:self];
    [appMenu addItem:appAboutItem];
    NSMenuItem *appSettingsItem = [[NSMenuItem alloc] initWithTitle:@"Preferences..." action:@selector(showSettings:) keyEquivalent:@","];
    [appSettingsItem setTarget:self];
    [appMenu addItem:appSettingsItem];
    [appMenu addItem:[NSMenuItem separatorItem]];
    [appMenu addItem:[[NSMenuItem alloc] initWithTitle:@"Hide OpenGridder Server" action:@selector(hide:) keyEquivalent:@"h"]];
    [appMenu addItem:[[NSMenuItem alloc] initWithTitle:@"Hide Others" action:@selector(hideOtherApplications:) keyEquivalent:@"h"]];
    [appMenu addItem:[[NSMenuItem alloc] initWithTitle:@"Show All" action:@selector(unhideAllApplications:) keyEquivalent:@""]];
    [appMenu addItem:[NSMenuItem separatorItem]];
    NSMenuItem *appQuitItem = [[NSMenuItem alloc] initWithTitle:@"Quit OpenGridder Server" action:@selector(quitApp:) keyEquivalent:@"q"];
    [appQuitItem setTarget:self];
    [appMenu addItem:appQuitItem];
    [appMenuItem setSubmenu:appMenu];
    [NSApp setMainMenu:mainMenu];

    // 3. Initialize Window Controllers
    self.splashController = [[SplashWindowController alloc] init];
    __weak typeof(self) weakSelf = self;
    self.splashController.onCancel = ^{
        // User requested skip scan: restart server immediately with --no-scan
        [weakSelf startServerWithRescan:NO noScan:YES];
    };

    self.pluginListController = [[PluginListWindowController alloc] init];
    self.pluginListController.onRescanRequested = ^{
        [weakSelf rescanAll:nil];
    };

    // Show Splash on launch
    [self.splashController.window makeKeyAndOrderFront:nil];

    // 4. Start Server Daemon
    [self startServerWithRescan:NO];
}

- (void)startServerWithRescan:(BOOL)rescan {
    [self startServerWithRescan:rescan noScan:NO];
}

- (void)startServerWithRescan:(BOOL)rescan noScan:(BOOL)noScan {
    [self stopServer];

    NSString *bundlePath = [[NSBundle mainBundle] bundlePath];
    NSString *binPath = [[NSBundle mainBundle] pathForResource:@"OpenGridderServer-bin" ofType:nil inDirectory:@"MacOS"];
    if (!binPath) {
        binPath = [bundlePath stringByAppendingPathComponent:@"Contents/MacOS/OpenGridderServer-bin"];
    }

    if (![[NSFileManager defaultManager] fileExistsAtPath:binPath]) {
        self.statusMenuItem.title = @"OpenGridder Server: Binary Not Found";
        [self.splashController setProgress:1.0 text:@"Error: Server binary not found"];
        return;
    }

    NSPipe *pipe = [NSPipe pipe];
    self.serverTask = [[NSTask alloc] init];
    self.serverTask.launchPath = binPath;

    NSMutableArray *taskArgs = [NSMutableArray arrayWithObject:@"--scan-progress"];
    if (noScan) {
        [taskArgs addObject:@"--no-scan"];
    } else if (rescan) {
        [taskArgs addObject:@"--rescan"];
    }
    self.serverTask.arguments = taskArgs;
    self.serverTask.standardOutput = pipe;

    if (![[NSFileManager defaultManager] fileExistsAtPath:self.logPath]) {
        [[NSFileManager defaultManager] createFileAtPath:self.logPath contents:nil attributes:nil];
    }
    NSFileHandle *logHandle = [NSFileHandle fileHandleForWritingAtPath:self.logPath];
    [logHandle seekToEndOfFile];
    self.serverTask.standardError = logHandle;

    NSFileHandle *readHandle = [pipe fileHandleForReading];
    __weak typeof(self) weakSelf = self;

    dispatch_async(dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_DEFAULT, 0), ^{
        NSData *data;
        NSMutableData *lineBuffer = [NSMutableData data];
        while ((data = [readHandle availableData]) && data.length > 0) {
            [lineBuffer appendData:data];
            // Split by newline
            const char *bytes = (const char *)lineBuffer.bytes;
            NSUInteger len = lineBuffer.length;
            NSUInteger start = 0;
            for (NSUInteger i = 0; i < len; ++i) {
                if (bytes[i] == '\n') {
                    NSData *lineData = [lineBuffer subdataWithRange:NSMakeRange(start, i - start)];
                    NSString *line = [[NSString alloc] initWithData:lineData encoding:NSUTF8StringEncoding];
                    if (line) {
                        [weakSelf handleServerOutputLine:line];
                    }
                    start = i + 1;
                }
            }
            if (start > 0) {
                [lineBuffer replaceBytesInRange:NSMakeRange(0, start) withBytes:NULL length:0];
            }
        }
    });

    self.serverTask.terminationHandler = ^(NSTask *task) {
        dispatch_async(dispatch_get_main_queue(), ^{
            weakSelf.statusMenuItem.title = [NSString stringWithFormat:@"OpenGridder Server: Stopped (Code %d)", task.terminationStatus];
        });
    };

    @try {
        [self.serverTask launch];
        self.statusMenuItem.title = @"OpenGridder Server: Running";
    } @catch (NSException *e) {
        self.statusMenuItem.title = @"OpenGridder Server: Launch Error";
        [self.splashController setProgress:1.0 text:@"Error launching server"];
    }
}

- (void)handleServerOutputLine:(NSString *)line {
    if ([line hasPrefix:@"SCAN_PROGRESS:"]) {
        NSString *jsonStr = [line substringFromIndex:14];
        NSData *jsonData = [jsonStr dataUsingEncoding:NSUTF8StringEncoding];
        NSDictionary *dict = [NSJSONSerialization JSONObjectWithData:jsonData options:0 error:nil];
        if (dict) {
            NSInteger cur = [dict[@"current"] integerValue];
            NSInteger total = [dict[@"total"] integerValue];
            NSString *name = dict[@"name"] ?: @"";
            NSString *fmt = dict[@"format"] ?: @"";
            double fraction = total > 0 ? (double)cur / (double)total : 0.0;
            NSString *msg = [NSString stringWithFormat:@"Scanning [%@]: %@ (%ld / %ld)", fmt, name, (long)cur, (long)total];
            [self.splashController setProgress:fraction text:msg];
        }
    } else if ([line hasPrefix:@"SCAN_CATALOG:"]) {
        NSString *jsonStr = [line substringFromIndex:13];
        NSData *jsonData = [jsonStr dataUsingEncoding:NSUTF8StringEncoding];
        NSArray *plugins = [NSJSONSerialization JSONObjectWithData:jsonData options:0 error:nil];
        if (plugins) {
            self.catalog = plugins;
            dispatch_async(dispatch_get_main_queue(), ^{
                [self.pluginListController updatePlugins:plugins];
            });
        }
    } else if ([line hasPrefix:@"SCAN_COMPLETE:"]) {
        NSString *jsonStr = [line substringFromIndex:14];
        NSData *jsonData = [jsonStr dataUsingEncoding:NSUTF8StringEncoding];
        NSDictionary *dict = [NSJSONSerialization JSONObjectWithData:jsonData options:0 error:nil];
        if (dict) {
            NSInteger total = [dict[@"total"] integerValue];
            NSInteger port = [dict[@"port"] integerValue];
            self.serverPort = port;
            dispatch_async(dispatch_get_main_queue(), ^{
                self.pluginCountMenuItem.title = [NSString stringWithFormat:@"Plugins: %ld Active", (long)total];
                self.statusMenuItem.title = [NSString stringWithFormat:@"OpenGridder Server: Running (Port %ld)", (long)port];
                [self.splashController setCompletedWithTotal:total port:port];
            });
        }
    }
}

- (void)stopServer {
    if (self.serverTask && [self.serverTask isRunning]) {
        [self.serverTask terminate];
        [self.serverTask waitUntilExit];
        self.serverTask = nil;
    }
}

- (void)restartServer:(id)sender {
    [self startServerWithRescan:NO];
}

- (void)rescanAll:(id)sender {
    [self.splashController.window makeKeyAndOrderFront:nil];
    [self.splashController setProgress:0.0 text:@"Starting full plugin rescan..."];
    [self startServerWithRescan:YES];
}

- (void)showPluginManager:(id)sender {
    [self.pluginListController.window makeKeyAndOrderFront:nil];
}

- (void)showAbout:(id)sender {
    if (!self.aboutController) {
        self.aboutController = [[AboutWindowController alloc] init];
    }
    [self.aboutController.window center];
    [self.aboutController.window makeKeyAndOrderFront:nil];
    [self.aboutController startScrollTimer];
    [NSApp activateIgnoringOtherApps:YES];
}

- (void)showSettings:(id)sender {
    if (!self.settingsController) {
        self.settingsController = [[SettingsWindowController alloc] init];
        self.settingsController.delegate = self;
    }
    [self.settingsController loadCurrentSettings];
    [self.settingsController.window center];
    [self.settingsController.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
}

- (NSMenu *)applicationDockMenu:(NSApplication *)sender {
    NSMenu *dockMenu = [[NSMenu alloc] initWithTitle:@"Dock Menu"];
    NSMenuItem *aboutItem = [[NSMenuItem alloc] initWithTitle:@"About OpenGridder Server..." action:@selector(showAbout:) keyEquivalent:@""];
    [aboutItem setTarget:self];
    [dockMenu addItem:aboutItem];

    NSMenuItem *settItem = [[NSMenuItem alloc] initWithTitle:@"Server Settings..." action:@selector(showSettings:) keyEquivalent:@""];
    [settItem setTarget:self];
    [dockMenu addItem:settItem];

    NSMenuItem *pmItem = [[NSMenuItem alloc] initWithTitle:@"Plugin Manager..." action:@selector(showPluginManager:) keyEquivalent:@""];
    [pmItem setTarget:self];
    [dockMenu addItem:pmItem];

    return dockMenu;
}

- (void)openLogs:(id)sender {
    [[NSWorkspace sharedWorkspace] openURL:[NSURL fileURLWithPath:self.logPath]];
}

- (void)openDocs:(id)sender {
    NSString *docPath = @"/Library/Application Support/OpenGridder/OpenGridder-User-Manual.pdf";
    if (![[NSFileManager defaultManager] fileExistsAtPath:docPath]) {
        docPath = [[NSBundle mainBundle] pathForResource:@"OpenGridder-User-Manual" ofType:@"pdf"];
    }
    if (docPath && [[NSFileManager defaultManager] fileExistsAtPath:docPath]) {
        [[NSWorkspace sharedWorkspace] openURL:[NSURL fileURLWithPath:docPath]];
    }
}

- (void)openConfig:(id)sender {
    NSString *appSupport = [NSSearchPathForDirectoriesInDomains(NSApplicationSupportDirectory, NSUserDomainMask, YES) firstObject];
    NSString *cfgDir = [appSupport stringByAppendingPathComponent:@"OpenGridder"];
    [[NSFileManager defaultManager] createDirectoryAtPath:cfgDir withIntermediateDirectories:YES attributes:nil error:nil];
    [[NSWorkspace sharedWorkspace] openURL:[NSURL fileURLWithPath:cfgDir]];
}

- (void)requestAccessibilityPermissions {
    NSDictionary *options = @{(__bridge id)kAXTrustedCheckOptionPrompt: @YES};
    BOOL trusted = AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)options);
    if (trusted) {
        NSLog(@"[OpenGridder] Accessibility features are authorized.");
    } else {
        NSLog(@"[OpenGridder] Accessibility features requested via macOS system prompt.");
    }
}

- (void)openAccessibilitySettings:(id)sender {
    NSDictionary *options = @{(__bridge id)kAXTrustedCheckOptionPrompt: @YES};
    BOOL trusted = AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)options);
    if (trusted) {
        NSAlert *alert = [[NSAlert alloc] init];
        alert.messageText = @"Accessibility Permissions Active";
        alert.informativeText = @"OpenGridder Server is fully authorized to send synthetic mouse and keyboard events to hosted plugin windows.";
        alert.alertStyle = NSAlertStyleInformational;
        [alert addButtonWithTitle:@"OK"];
        [alert runModal];
    } else {
        NSURL *url = [NSURL URLWithString:@"x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"];
        [[NSWorkspace sharedWorkspace] openURL:url];
    }
}

- (void)quitApp:(id)sender {
    [self stopServer];
    [NSApp terminate:nil];
}

- (void)applicationWillTerminate:(NSNotification *)notification {
    [self stopServer];
}

@end

#pragma mark - Main Entry Point

int main(int argc, char *argv[]) {
    // If CLI arguments were provided (e.g. --help, --version, --au, --list), bypass Cocoa and exec server-bin directly!
    if (argc > 1) {
        NSString *bundlePath = [[NSBundle mainBundle] bundlePath];
        NSString *binPath = [bundlePath stringByAppendingPathComponent:@"Contents/MacOS/OpenGridderServer-bin"];
        if ([[NSFileManager defaultManager] fileExistsAtPath:binPath]) {
            execv([binPath UTF8String], argv);
        }
    }

    @autoreleasepool {
        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyRegular];
        OpenGridderAppDelegate *delegate = [[OpenGridderAppDelegate alloc] init];
        [app setDelegate:delegate];
        [app run];
    }
    return 0;
}
