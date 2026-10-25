// OpenGridder Server native macOS Application with GoodLookinDri Hardware UI
// Pure AppKit / Cocoa implementation - Zero JUCE.
#import <Cocoa/Cocoa.h>
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
    NSRect frame = NSMakeRect(0, 0, 640, 320);
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

        // Logo
        NSImageView *logoView = [[NSImageView alloc] initWithFrame:NSMakeRect(40, 185, 80, 80)];
        NSImage *icon = [NSImage imageNamed:@"OpenGridderServer"];
        if (!icon) {
            NSString *iconPath = [[NSBundle mainBundle] pathForResource:@"OpenGridderServer" ofType:@"icns"];
            if (iconPath) icon = [[NSImage alloc] initWithContentsOfFile:iconPath];
        }
        logoView.image = icon;
        [plate addSubview:logoView];

        // Title
        self.titleLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(140, 225, 460, 36)];
        self.titleLabel.editable = NO;
        self.titleLabel.bordered = NO;
        self.titleLabel.backgroundColor = [NSColor clearColor];
        self.titleLabel.font = [NSFont systemFontOfSize:24 weight:NSFontWeightBold];
        self.titleLabel.textColor = [NSColor colorWithCalibratedRed:0.95 green:0.95 blue:0.97 alpha:1.0];
        self.titleLabel.stringValue = @"OPENGRIDDER SERVER";
        [plate addSubview:self.titleLabel];

        // Subtitle
        self.subtitleLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(140, 200, 460, 24)];
        self.subtitleLabel.editable = NO;
        self.subtitleLabel.bordered = NO;
        self.subtitleLabel.backgroundColor = [NSColor clearColor];
        self.subtitleLabel.font = [NSFont systemFontOfSize:13 weight:NSFontWeightMedium];
        self.subtitleLabel.textColor = [NSColor colorWithCalibratedRed:0.95 green:0.75 blue:0.25 alpha:1.0]; // GoodLookin Amber
        self.subtitleLabel.stringValue = @"Zero-JUCE Audio Streaming Grid • GoodLookin UI Engine";
        [plate addSubview:self.subtitleLabel];

        // Version & Arch
        self.versionLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(140, 180, 460, 18)];
        self.versionLabel.editable = NO;
        self.versionLabel.bordered = NO;
        self.versionLabel.backgroundColor = [NSColor clearColor];
        self.versionLabel.font = [NSFont systemFontOfSize:11 weight:NSFontWeightRegular];
        self.versionLabel.textColor = [NSColor colorWithCalibratedRed:0.60 green:0.65 blue:0.70 alpha:0.8];
        self.versionLabel.stringValue = @"v0.2.1-alpha • macOS Universal (Apple Silicon & Intel)";
        [plate addSubview:self.versionLabel];

        // Progress bar
        self.progressBar = [[NSProgressIndicator alloc] initWithFrame:NSMakeRect(40, 110, 560, 14)];
        self.progressBar.indeterminate = NO;
        self.progressBar.minValue = 0.0;
        self.progressBar.maxValue = 1.0;
        self.progressBar.doubleValue = 0.0;
        [plate addSubview:self.progressBar];

        // Status message
        self.statusLabel = [[NSTextField alloc] initWithFrame:NSMakeRect(40, 75, 450, 26)];
        self.statusLabel.editable = NO;
        self.statusLabel.bordered = NO;
        self.statusLabel.backgroundColor = [NSColor clearColor];
        self.statusLabel.font = [NSFont monospacedSystemFontOfSize:12 weight:NSFontWeightRegular];
        self.statusLabel.textColor = [NSColor colorWithCalibratedRed:0.85 green:0.88 blue:0.92 alpha:1.0];
        self.statusLabel.stringValue = @"Initializing plugin discovery...";
        [plate addSubview:self.statusLabel];

        // Skip / Cancel button
        self.cancelButton = [[NSButton alloc] initWithFrame:NSMakeRect(500, 72, 100, 28)];
        self.cancelButton.bezelStyle = NSBezelStyleRounded;
        self.cancelButton.title = @"Skip Scan";
        self.cancelButton.target = self;
        self.cancelButton.action = @selector(cancelClicked:);
        [plate addSubview:self.cancelButton];

        // Footer note
        NSTextField *footer = [[NSTextField alloc] initWithFrame:NSMakeRect(40, 24, 560, 16)];
        footer.editable = NO;
        footer.bordered = NO;
        footer.backgroundColor = [NSColor clearColor];
        footer.font = [NSFont systemFontOfSize:10 weight:NSFontWeightLight];
        footer.textColor = [NSColor colorWithCalibratedRed:0.45 green:0.48 blue:0.52 alpha:0.8];
        footer.stringValue = @"AudioGridder concept by Andreas Pohl • GoodLookin hardware UI by Marcos Deida";
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

    for (NSDictionary *p in self.allPlugins) {
        NSString *name = p[@"name"] ?: @"";
        NSString *vendor = p[@"company"] ?: @"";
        NSString *fmt = p[@"type"] ?: @"AudioUnit";
        NSString *cat = p[@"category"] ?: @"";

        if ([fmt containsString:@"AudioUnit"] || [fmt containsString:@"AU"]) auCount++;
        else if ([fmt containsString:@"VST3"]) vst3Count++;

        if (seg == 1 && ![fmt containsString:@"AudioUnit"] && ![fmt containsString:@"AU"]) continue;
        if (seg == 2 && ![fmt containsString:@"VST3"]) continue;
        if (seg == 3 && ![fmt containsString:@"VST2"] && ![fmt containsString:@"VST"]) continue;

        if (search.length > 0) {
            NSString *full = [NSString stringWithFormat:@"%@ %@ %@ %@", name, vendor, fmt, cat].lowercaseString;
            if (![full containsString:search]) continue;
        }
        [filtered addObject:p];
    }
    self.filteredPlugins = filtered;
    [self.tableView reloadData];

    self.countLabel.stringValue = [NSString stringWithFormat:@"Showing %ld of %ld plugins (%ld AudioUnits, %ld VST3)",
                                  (long)self.filteredPlugins.count, (long)self.allPlugins.count, (long)auCount, (long)vst3Count];
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
        cell.textColor = [fmt containsString:@"VST3"] ? [NSColor colorWithCalibratedRed:0.4 green:0.8 blue:1.0 alpha:1.0] : [NSColor colorWithCalibratedRed:1.0 green:0.8 blue:0.4 alpha:1.0];
        cell.stringValue = fmt;
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

#pragma mark - Main Application Delegate

@interface OpenGridderAppDelegate : NSObject <NSApplicationDelegate>
@property (strong, nonatomic) NSStatusItem *statusItem;
@property (strong, nonatomic) NSTask *serverTask;
@property (strong, nonatomic) NSString *logPath;
@property (strong, nonatomic) NSMenuItem *statusMenuItem;
@property (strong, nonatomic) NSMenuItem *pluginCountMenuItem;
@property (strong, nonatomic) SplashWindowController *splashController;
@property (strong, nonatomic) PluginListWindowController *pluginListController;
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

    [menu addItem:[NSMenuItem separatorItem]];

    NSMenuItem *restartItem = [[NSMenuItem alloc] initWithTitle:@"Restart Server" action:@selector(restartServer:) keyEquivalent:@""];
    [restartItem setTarget:self];
    [menu addItem:restartItem];

    NSMenuItem *quitItem = [[NSMenuItem alloc] initWithTitle:@"Quit OpenGridder Server" action:@selector(quitApp:) keyEquivalent:@"q"];
    [quitItem setTarget:self];
    [menu addItem:quitItem];

    self.statusItem.menu = menu;

    // 3. Initialize Window Controllers
    self.splashController = [[SplashWindowController alloc] init];
    __weak typeof(self) weakSelf = self;
    self.splashController.onCancel = ^{
        // User requested cancel slow scan: continue running server
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
    if (rescan) {
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
