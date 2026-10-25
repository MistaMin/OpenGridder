// OpenGridder Server native macOS Application wrapper
// Pure AppKit / Cocoa implementation - Zero JUCE.
#import <Cocoa/Cocoa.h>
#import <signal.h>
#import <unistd.h>

@interface OpenGridderAppDelegate : NSObject <NSApplicationDelegate>
@property (strong, nonatomic) NSStatusItem *statusItem;
@property (strong, nonatomic) NSTask *serverTask;
@property (strong, nonatomic) NSString *logPath;
@property (strong, nonatomic) NSMenuItem *statusMenuItem;
@end

@implementation OpenGridderAppDelegate

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    // 1. Prepare log directory and file
    NSString *appSupport = [NSSearchPathForDirectoriesInDomains(NSLibraryDirectory, NSUserDomainMask, YES) firstObject];
    NSString *logDir = [appSupport stringByAppendingPathComponent:@"Logs/OpenGridder"];
    [[NSFileManager defaultManager] createDirectoryAtPath:logDir withIntermediateDirectories:YES attributes:nil error:nil];
    self.logPath = [logDir stringByAppendingPathComponent:@"server.log"];

    // 2. Setup System Status Bar Item
    self.statusItem = [[NSStatusBar systemStatusBar] statusItemWithLength:NSVariableStatusItemLength];
    
    // Attempt to load bundled status icon
    NSImage *icon = [NSImage imageNamed:@"OpenGridderServer"];
    if (!icon) {
        NSString *iconPath = [[NSBundle mainBundle] pathForResource:@"OpenGridderServer" ofType:@"icns"];
        if (iconPath) {
            icon = [[NSImage alloc] initWithContentsOfFile:iconPath];
        }
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

    NSMenuItem *portItem = [[NSMenuItem alloc] initWithTitle:@"Port: 55056 (Default)" action:nil keyEquivalent:@""];
    [portItem setEnabled:NO];
    [menu addItem:portItem];

    [menu addItem:[NSMenuItem separatorItem]];

    NSMenuItem *logsItem = [[NSMenuItem alloc] initWithTitle:@"View Server Logs..." action:@selector(openLogs:) keyEquivalent:@"l"];
    [logsItem setTarget:self];
    [menu addItem:logsItem];

    NSMenuItem *docsItem = [[NSMenuItem alloc] initWithTitle:@"Open Documentation (PDF)..." action:@selector(openDocs:) keyEquivalent:@"d"];
    [docsItem setTarget:self];
    [menu addItem:docsItem];

    NSMenuItem *configItem = [[NSMenuItem alloc] initWithTitle:@"Open Config Folder..." action:@selector(openConfig:) keyEquivalent:@""];
    [configItem setTarget:self];
    [menu addItem:configItem];

    [menu addItem:[NSMenuItem separatorItem]];

    NSMenuItem *restartItem = [[NSMenuItem alloc] initWithTitle:@"Restart Server" action:@selector(restartServer:) keyEquivalent:@"r"];
    [restartItem setTarget:self];
    [menu addItem:restartItem];

    NSMenuItem *quitItem = [[NSMenuItem alloc] initWithTitle:@"Quit OpenGridder Server" action:@selector(quitApp:) keyEquivalent:@"q"];
    [quitItem setTarget:self];
    [menu addItem:quitItem];

    self.statusItem.menu = menu;

    // 3. Launch the Server Subprocess
    [self startServer];
}

- (void)startServer {
    NSString *bundlePath = [[NSBundle mainBundle] bundlePath];
    NSString *binPath = [[NSBundle mainBundle] pathForResource:@"OpenGridderServer-bin" ofType:nil inDirectory:@"MacOS"];
    if (!binPath) {
        binPath = [bundlePath stringByAppendingPathComponent:@"Contents/MacOS/OpenGridderServer-bin"];
    }

    if (![[NSFileManager defaultManager] fileExistsAtPath:binPath]) {
        self.statusMenuItem.title = @"OpenGridder Server: Binary Not Found";
        return;
    }

    if (![[NSFileManager defaultManager] fileExistsAtPath:self.logPath]) {
        [[NSFileManager defaultManager] createFileAtPath:self.logPath contents:nil attributes:nil];
    }
    NSFileHandle *logHandle = [NSFileHandle fileHandleForWritingAtPath:self.logPath];
    [logHandle seekToEndOfFile];

    self.serverTask = [[NSTask alloc] init];
    self.serverTask.launchPath = binPath;
    self.serverTask.arguments = @[];
    self.serverTask.standardOutput = logHandle;
    self.serverTask.standardError = logHandle;

    __weak typeof(self) weakSelf = self;
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
    [self stopServer];
    [self startServer];
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

int main(int argc, char *argv[]) {
    // If CLI arguments were provided (e.g. --help, --version, --au), bypass Cocoa and exec server-bin directly!
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
