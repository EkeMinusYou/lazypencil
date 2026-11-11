// Exercise live foreground changes without activating apps or posting input.
#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#include <assert.h>

static pid_t focused_pid = 42;
static AXError query_error = kAXErrorSuccess;
static AXError pid_error = kAXErrorSuccess;
static AXError timeout_error = kAXErrorSuccess;
static bool missing_system = false;
static bool wrong_type = false;
static bool missing_application = false;
static unsigned queries = 0;

static AXUIElementRef test_system(void) {
    return missing_system ? NULL : AXUIElementCreateSystemWide();
}

static AXError test_timeout(AXUIElementRef element, float seconds) {
    assert(element && seconds > 0 && seconds <= 0.011f);
    return timeout_error;
}

static AXError test_attribute(AXUIElementRef element, CFStringRef attribute, CFTypeRef *value) {
    assert(element && CFEqual(attribute, kAXFocusedApplicationAttribute));
    queries++;
    if (query_error != kAXErrorSuccess) return query_error;
    if (missing_application) {
        *value = NULL;
    } else if (wrong_type) {
        *value = CFRetain(CFSTR("Unexpected focus value"));
    } else {
        *value = AXUIElementCreateApplication(42);
    }
    return kAXErrorSuccess;
}

static AXError test_pid(AXUIElementRef element, pid_t *pid) {
    assert(element);
    *pid = focused_pid;
    return pid_error;
}

#define AXUIElementCreateSystemWide test_system
#define AXUIElementSetMessagingTimeout test_timeout
#define AXUIElementCopyAttributeValue test_attribute
#define AXUIElementGetPid test_pid
#include "macos.m"

static int32_t sample_host(void) {
    double x, y, scale;
    int32_t host;
    lp_pinch_pointer(&x, &y, &host, &scale);
    return host;
}

int main(void) {
    @autoreleasepool {
        assert(sample_host() == 42);
        focused_pid = 43;
        assert(sample_host() == 43);
        assert(queries == 2);
        for (focused_pid = -1; focused_pid <= 0; focused_pid++) assert(sample_host() == 0);
        focused_pid = 42;
        query_error = kAXErrorCannotComplete;
        assert(sample_host() == 0);
        query_error = kAXErrorSuccess;
        pid_error = kAXErrorCannotComplete;
        assert(sample_host() == 0);
        pid_error = kAXErrorSuccess;
        wrong_type = true;
        assert(sample_host() == 0);
        wrong_type = false;
        missing_application = true;
        assert(sample_host() == 0);
        missing_application = false;
        timeout_error = kAXErrorCannotComplete;
        assert(sample_host() == 0);
        timeout_error = kAXErrorSuccess;
        missing_system = true;
        assert(sample_host() == 0);
    }
    return 0;
}
