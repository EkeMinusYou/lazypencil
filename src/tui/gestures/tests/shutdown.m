// Exercise the production thread and shutdown without a desktop event tap.
#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#include <assert.h>
#include <pthread.h>
#include <stdbool.h>

static pthread_mutex_t gate = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t changed = PTHREAD_COND_INITIALIZER;
static bool entering_run = false;
static bool stop_delivered = false;
static bool rescue_fired = false;

static void rescue(CFRunLoopTimerRef timer, void *context) {
    (void)timer;
    rescue_fired = true;
    CFRunLoopStop((CFRunLoopRef)context);
}

static void controlled_run(void) {
    CFRunLoopRef loop = CFRunLoopGetCurrent();
    CFRunLoopTimerContext context = {0, loop, NULL, NULL, NULL};
    CFRunLoopTimerRef timer = CFRunLoopTimerCreate(NULL, CFAbsoluteTimeGetCurrent() + 1,
        0, 0, 0, rescue, &context);
    assert(timer);
    CFRunLoopAddTimer(loop, timer, kCFRunLoopDefaultMode);
    pthread_mutex_lock(&gate);
    entering_run = true;
    pthread_cond_broadcast(&changed);
    while (!stop_delivered) pthread_cond_wait(&changed, &gate);
    pthread_mutex_unlock(&gate);
    CFRunLoopRun();
    CFRunLoopTimerInvalidate(timer);
    CFRelease(timer);
}

static void delivered(void) {
    pthread_mutex_lock(&gate);
    stop_delivered = true;
    pthread_cond_broadcast(&changed);
    pthread_mutex_unlock(&gate);
}

static void controlled_stop(CFRunLoopRef loop) {
    CFRunLoopStop(loop);
    delivered();
}

void controlled_wake(CFRunLoopRef loop) {
    CFRunLoopWakeUp(loop);
    delivered();
}

#define CFRunLoopRun controlled_run
#define CFRunLoopStop controlled_stop
#define CFRunLoopWakeUp controlled_wake
#include "macos.m"
#undef CFRunLoopRun
#undef CFRunLoopStop
#undef CFRunLoopWakeUp

static void unused_callback(CFMachPortRef port, void *message, CFIndex size, void *info) {
    (void)port; (void)message; (void)size; (void)info;
}

static Pinch *fixture(void) {
    Pinch *pinch = calloc(1, sizeof(*pinch));
    assert(pinch);
    assert(pthread_mutex_init(&pinch->mutex, NULL) == 0);
    CFMachPortContext context = {0, NULL, NULL, NULL, NULL};
    pinch->tap = CFMachPortCreate(NULL, unused_callback, &context, NULL);
    assert(pinch->tap);
    return pinch;
}

int main(void) {
    @autoreleasepool {
        // Force shutdown into the gap after the last stopped check, before Run.
        Pinch *pinch = fixture();
        assert(pthread_create(&pinch->thread, NULL, run, pinch) == 0);
        pthread_mutex_lock(&gate);
        while (!entering_run) pthread_cond_wait(&changed, &gate);
        pthread_mutex_unlock(&gate);
        lp_pinch_stop(pinch);
        assert(!rescue_fired);

        // Shutdown before loop publication must also complete without running.
        pinch = fixture();
        atomic_store(&pinch->stopped, true);
        assert(pthread_create(&pinch->thread, NULL, run, pinch) == 0);
        lp_pinch_stop(pinch);
    }
    return 0;
}
