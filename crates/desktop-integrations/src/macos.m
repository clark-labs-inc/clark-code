#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#include <stdatomic.h>

static _Atomic uint64_t accessEpoch = 1;
static _Atomic bool suspended = false;

// Called once on the real app's main thread, never from a shell helper. OS
// consent therefore names the application that actually owns this feature.
void cc_integrations_initialize(void) {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        NSNotificationCenter *workspace = NSWorkspace.sharedWorkspace.notificationCenter;
        for (NSString *name in @[NSWorkspaceWillSleepNotification, NSWorkspaceSessionDidResignActiveNotification]) {
            [workspace addObserverForName:name object:nil queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *note) {
                (void)note;
                atomic_fetch_add(&accessEpoch, 1);
                atomic_store(&suspended, true);
            }];
        }
        for (NSString *name in @[NSWorkspaceDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification]) {
            [workspace addObserverForName:name object:nil queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *note) {
                (void)note;
                atomic_fetch_add(&accessEpoch, 1);
                atomic_store(&suspended, false);
            }];
        }
        for (NSString *name in @[@"com.apple.screenIsLocked", @"com.apple.screenIsUnlocked"]) {
            [NSDistributedNotificationCenter.defaultCenter addObserverForName:name object:nil queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *note) {
                (void)note;
                atomic_fetch_add(&accessEpoch, 1);
            }];
        }
    });
}

uint64_t cc_integrations_epoch(void) { return atomic_load(&accessEpoch); }

// Read-tool invocations run on a provider worker. This check performs no UI
// work and is safe there; native approval below remains main-thread-only.
bool cc_integrations_interactive(void) {
    if (atomic_load(&suspended)) return false;
    NSDictionary *session = CFBridgingRelease(CGSessionCopyCurrentDictionary());
    return session && [session[(__bridge NSString *)kCGSessionOnConsoleKey] boolValue]
        && ![session[@"CGSSessionScreenIsLocked"] boolValue];
}

bool cc_integrations_confirm(const char *title, const char *body, const char *button) {
    if (!NSThread.isMainThread || !cc_integrations_interactive()) return false;
    NSAlert *alert = [[NSAlert alloc] init];
    alert.messageText = [NSString stringWithUTF8String:title];
    alert.informativeText = @"Review the complete details below before continuing.";
    NSScrollView *scroll = [[NSScrollView alloc] initWithFrame:NSMakeRect(0, 0, 480, 260)];
    scroll.hasVerticalScroller = YES;
    NSTextView *details = [[NSTextView alloc] initWithFrame:NSMakeRect(0, 0, 460, 260)];
    details.editable = NO;
    details.selectable = YES;
    details.font = [NSFont systemFontOfSize:13];
    details.string = [NSString stringWithUTF8String:body];
    details.verticallyResizable = YES;
    details.textContainer.widthTracksTextView = YES;
    scroll.documentView = details;
    alert.accessoryView = scroll;
    [alert addButtonWithTitle:@"Cancel"];
    [alert addButtonWithTitle:[NSString stringWithUTF8String:button]];
    [NSApp activateIgnoringOtherApps:YES];
    return [alert runModal] == NSAlertSecondButtonReturn && cc_integrations_interactive();
}

void cc_integrations_settings(void) {
    NSURL *url = [NSURL URLWithString:@"x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"];
    [NSWorkspace.sharedWorkspace openURL:url];
}
