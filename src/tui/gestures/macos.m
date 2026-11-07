// Native gesture forwarding is independent of the host terminal application.
#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#include <float.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdlib.h>

typedef bool (*PinchCallback)(void *, uint32_t, double, double, double, double, int32_t, uint32_t, bool);
typedef struct {
    pthread_t thread;
    pthread_mutex_t mutex;
    atomic_bool stopped;
    CFRunLoopRef loop;
    CFMachPortRef tap;
    PinchCallback callback;
    void *context;
} Pinch;

bool lp_pinch_authorized(bool request) {
    if (AXIsProcessTrusted()) return true;
    if (!request) return false;
    NSDictionary *options = @{(__bridge NSString *)kAXTrustedCheckOptionPrompt: @YES};
    return AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)options);
}

void lp_pinch_pointer(double *x, double *y, int32_t *host, double *scale) {
    @autoreleasepool {
        CGEventRef event = CGEventCreate(NULL);
        CGPoint point = event ? CGEventGetLocation(event) : CGPointMake(-DBL_MAX, -DBL_MAX);
        if (event) CFRelease(event);
        *x = point.x; *y = point.y;
        *scale = 1.0;
        CGDirectDisplayID display = 0;
        uint32_t count = 0;
        if (CGGetDisplaysWithPoint(point, 1, &display, &count) == kCGErrorSuccess && count) {
            CGDisplayModeRef mode = CGDisplayCopyDisplayMode(display);
            if (mode) {
                size_t width = CGDisplayModeGetWidth(mode);
                if (width) *scale = (double)CGDisplayModeGetPixelWidth(mode) / (double)width;
                CGDisplayModeRelease(mode);
            }
        }
        *host = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
    }
}

bool lp_pinch_decode(CGEventRef event, double *delta, bool *ended) {
    @autoreleasepool { @try {
        NSEvent *native = [NSEvent eventWithCGEvent:event];
        if (native.type != NSEventTypeMagnify) return false;
        *delta = native.magnification;
        *ended = (native.phase & (NSEventPhaseEnded | NSEventPhaseCancelled)) != 0;
        return true;
    } @catch (NSException *exception) { (void)exception; return false; } }
}

bool lp_scroll_decode(CGEventRef event, double *dx, double *dy, uint32_t *modifiers, bool *ended) {
    @autoreleasepool { @try {
        NSEvent *native = [NSEvent eventWithCGEvent:event];
        if (native.type != NSEventTypeScrollWheel || !native.hasPreciseScrollingDeltas) return false;
        // AppKit supplies points, both axes and the user's natural-scroll direction.
        // Keep ordinary wheel detents on the terminal path.
        *dx = native.scrollingDeltaX;
        *dy = native.scrollingDeltaY;
        NSEventModifierFlags flags = native.modifierFlags;
        *modifiers = ((flags & NSEventModifierFlagShift) ? 1 : 0)
            | ((flags & NSEventModifierFlagControl) ? 2 : 0)
            | ((flags & NSEventModifierFlagOption) ? 4 : 0)
            | ((flags & NSEventModifierFlagCommand) ? 8 : 0);
        *ended = ((native.phase | native.momentumPhase)
            & (NSEventPhaseEnded | NSEventPhaseCancelled)) != 0;
        return true;
    } @catch (NSException *exception) { (void)exception; return false; } }
}

static CGEventRef receive(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *context) {
    (void)proxy;
    Pinch *pinch = context;
    if (type == kCGEventTapDisabledByTimeout) {
        if (!atomic_load(&pinch->stopped)) CGEventTapEnable(pinch->tap, true);
        return event;
    }
    if (type == kCGEventTapDisabledByUserInput || atomic_load(&pinch->stopped)) return event;
    @autoreleasepool { @try {
        double dx = 0, dy = 0; bool ended = false;
        uint32_t kind = 0, modifiers = 0;
        if (type == kCGEventScrollWheel) {
            if (!lp_scroll_decode(event, &dx, &dy, &modifiers, &ended)) return event;
            kind = 1;
        } else if (!lp_pinch_decode(event, &dx, &ended)) {
            return event;
        }
        // Gesture packets need not carry a mouse position. Query the pointer
        // through the same public API used for terminal-position calibration.
        double x = 0, y = 0, scale = 1; int32_t host = 0;
        lp_pinch_pointer(&x, &y, &host, &scale);
        // Only accepted packets are suppressed, preventing a second, rounded
        // copy from arriving through the terminal mouse protocol.
        if (pinch->callback(pinch->context, kind, dx, dy, x, y, host, modifiers, ended)) return NULL;
    } @catch (NSException *exception) { (void)exception; } }
    return event;
}
static void *run(void *context) {
    @autoreleasepool {
        Pinch *pinch = context;
        CFRunLoopRef loop = CFRunLoopGetCurrent();
        CFRunLoopSourceRef source = CFMachPortCreateRunLoopSource(NULL, pinch->tap, 0);
        CFRunLoopAddSource(loop, source, kCFRunLoopCommonModes);
        pthread_mutex_lock(&pinch->mutex);
        pinch->loop = loop;
        bool stopped = atomic_load(&pinch->stopped);
        pthread_mutex_unlock(&pinch->mutex);
        if (!stopped) CFRunLoopRun();
        pthread_mutex_lock(&pinch->mutex);
        pinch->loop = NULL;
        pthread_mutex_unlock(&pinch->mutex);
        CFMachPortInvalidate(pinch->tap);
        CFRelease(source);
    }
    return NULL;
}
void *lp_pinch_start(PinchCallback callback, void *context) {
    if (!lp_pinch_authorized(false)) return NULL;
    Pinch *pinch = calloc(1,sizeof(*pinch));
    if (!pinch) return NULL;
    pinch->callback = callback; pinch->context = context;
    pthread_mutex_init(&pinch->mutex,NULL);
    // AppKit's public gesture/magnify event types are not part of the TTY mouse protocol.
    CGEventMask mask = (1ULL << NSEventTypeGesture) | (1ULL << NSEventTypeMagnify) | CGEventMaskBit(kCGEventScrollWheel);
    pinch->tap = CGEventTapCreate(kCGHIDEventTap,kCGHeadInsertEventTap,kCGEventTapOptionDefault,mask,receive,pinch);
    if (!pinch->tap || pthread_create(&pinch->thread,NULL,run,pinch) != 0) {
        if (pinch->tap) { CFMachPortInvalidate(pinch->tap); CFRelease(pinch->tap); }
        pthread_mutex_destroy(&pinch->mutex); free(pinch); return NULL;
    }
    return pinch;
}
void lp_pinch_stop(void *handle) {
    if (!handle) return;
    Pinch *pinch = handle;
    pthread_mutex_lock(&pinch->mutex);
    atomic_store(&pinch->stopped,true);
    if (pinch->loop) CFRunLoopStop(pinch->loop);
    pthread_mutex_unlock(&pinch->mutex);
    pthread_join(pinch->thread,NULL);
    CFRelease(pinch->tap);
    pthread_mutex_destroy(&pinch->mutex);
    free(pinch);
}
