// Objective-C exception safety wrappers for AXUIElement API calls.
//
// These functions wrap macOS accessibility API calls in @try/@catch blocks
// so that ObjC exceptions (NSException) are caught at the C level and never
// unwind through Rust frames. This is necessary because Rust's stable ABI
// aborts on foreign exceptions in extern "C" functions.

#import <ApplicationServices/ApplicationServices.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

// ── Attribute Access ─────────────────────────────────────────────────────────

// Safe wrapper for AXUIElementCopyAttributeValue.
// Returns the AX error code, or -9999 if an ObjC exception was thrown.
int safe_ax_copy_attribute_value(
    AXUIElementRef element,
    CFStringRef attribute,
    CFTypeRef *value
) {
    @try {
        return AXUIElementCopyAttributeValue(element, attribute, value);
    } @catch (NSException *e) {
        *value = NULL;
        return -9999;
    }
}

// ── Batch Attribute Access ───────────────────────────────────────────────────

// Safe wrapper for AXUIElementCopyMultipleAttributeValues.
// Fetches multiple attributes in a single Mach IPC round-trip.
// Returns the AX error code, or -9999 if an ObjC exception was thrown.
int safe_ax_copy_multiple_attribute_values(
    AXUIElementRef element,
    CFArrayRef attributes,
    CFArrayRef *values
) {
    @try {
        // 0 = kAXCopyMultipleAttributeOptionStopOnError — don't stop on error
        return AXUIElementCopyMultipleAttributeValues(element, attributes, 0, values);
    } @catch (NSException *e) {
        *values = NULL;
        return -9999;
    }
}

// ── Action Names ─────────────────────────────────────────────────────────────

// Safe wrapper for AXUIElementCopyActionNames.
// Returns the AX error code, or -9999 if an ObjC exception was thrown.
int safe_ax_copy_action_names(
    AXUIElementRef element,
    CFArrayRef *names
) {
    @try {
        return AXUIElementCopyActionNames(element, names);
    } @catch (NSException *e) {
        *names = NULL;
        return -9999;
    }
}

// ── Perform Action ───────────────────────────────────────────────────────────

// Safe wrapper for AXUIElementPerformAction.
// Returns the AX error code, or -9999 if an ObjC exception was thrown.
int safe_ax_perform_action(
    AXUIElementRef element,
    CFStringRef action
) {
    @try {
        return AXUIElementPerformAction(element, action);
    } @catch (NSException *e) {
        return -9999;
    }
}

// ── Set Attribute ────────────────────────────────────────────────────────────

// Safe wrapper for AXUIElementSetAttributeValue.
// Returns the AX error code, or -9999 if an ObjC exception was thrown.
int safe_ax_set_attribute_value(
    AXUIElementRef element,
    CFStringRef attribute,
    CFTypeRef value
) {
    @try {
        return AXUIElementSetAttributeValue(element, attribute, value);
    } @catch (NSException *e) {
        return -9999;
    }
}

// Safe wrapper for AXUIElementIsAttributeSettable. Converts the MacTypes
// Boolean out-param to a C99 bool so the Rust side can use `*mut bool`.
int safe_ax_is_attribute_settable(
    AXUIElementRef element,
    CFStringRef attribute,
    bool *outSettable
) {
    Boolean settable = false;
    @try {
        AXError err = AXUIElementIsAttributeSettable(element, attribute, &settable);
        *outSettable = (settable != 0);
        return err;
    } @catch (NSException *e) {
        *outSettable = false;
        return -9999;
    }
}

// ── Window List ──────────────────────────────────────────────────────────────

// Safe wrapper for CGWindowListCopyWindowInfo.
// Returns NULL if an ObjC exception was thrown.
CFArrayRef safe_cg_window_list_copy(uint32_t option, uint32_t relativeToWindow) {
    @try {
        return CGWindowListCopyWindowInfo(option, relativeToWindow);
    } @catch (NSException *e) {
        return NULL;
    }
}

// ── Create Application Element ───────────────────────────────────────────────

// Safe wrapper for AXUIElementCreateApplication.
// Returns NULL if an ObjC exception was thrown.
AXUIElementRef safe_ax_create_application(int pid) {
    @try {
        return AXUIElementCreateApplication(pid);
    } @catch (NSException *e) {
        return NULL;
    }
}

// ── System-Wide Element / Focused Application ────────────────────────────────

// Safe wrapper for AXUIElementCreateSystemWide.
// The system-wide element is the entry point for global attributes such as
// kAXFocusedApplicationAttribute. Returns NULL if an ObjC exception was thrown.
AXUIElementRef safe_ax_create_system_wide(void) {
    @try {
        return AXUIElementCreateSystemWide();
    } @catch (NSException *e) {
        return NULL;
    }
}

// Safe wrapper for AXUIElementGetPid. Writes the owning process id of
// `element` to `*outPid`. Returns the AX error code, or -9999 if an ObjC
// exception was thrown.
int safe_ax_get_pid(AXUIElementRef element, int *outPid) {
    @try {
        pid_t pid = 0;
        AXError err = AXUIElementGetPid(element, &pid);
        *outPid = (int)pid;
        return err;
    } @catch (NSException *e) {
        *outPid = 0;
        return -9999;
    }
}

// ── Messaging Timeout ───────────────────────────────────────────────────────

// Safe wrapper for AXUIElementSetMessagingTimeout. Apple documents the scope by
// what the element IS: set on the system-wide object it affects every
// application, set on an application element it affects every message to that
// application, and only on some other element does it affect that element
// alone. The shell scan sets it on application elements, so it is NOT
// element-scoped there — the bound must be released again (see
// ShellProbeBound in ax.rs) or the app stays pinned at the scan's timeout.
// The default is ~1.5s per attribute, which is why the scan bounds each probe.
// Returns the AX error code, or -9999 if an ObjC exception was thrown.
int safe_ax_set_messaging_timeout(AXUIElementRef element, float timeoutInSeconds) {
    @try {
        return AXUIElementSetMessagingTimeout(element, timeoutInSeconds);
    } @catch (NSException *e) {
        return -9999;
    }
}

// ── AXValue Extraction ──────────────────────────────────────────────────────

// Safe wrapper for AXValueGetValue.
// Returns false if an ObjC exception was thrown.
Boolean safe_ax_value_get_value(AXValueRef value, AXValueType type, void *valuePtr) {
    @try {
        return AXValueGetValue(value, type, valuePtr);
    } @catch (NSException *e) {
        return false;
    }
}

// [M4] One CopyMultipleAttributeValues slot as an AXError: 0 = a value,
// kAXErrorNoValue = kCFNull, the carried code for an AXValue of
// kAXValueAXErrorType, -1 = no slot (short array), -9999 = exception.
int safe_ax_batch_slot_error(CFTypeRef value) {
    @try {
        if (value == NULL) return -1;
        if (value == kCFNull) return kAXErrorNoValue;
        if (CFGetTypeID(value) != AXValueGetTypeID()
            || AXValueGetType((AXValueRef)value) != kAXValueAXErrorType) return 0;
        AXError code = kAXErrorFailure;
        if (!AXValueGetValue((AXValueRef)value, kAXValueAXErrorType, &code)) return kAXErrorFailure;
        return code;
    } @catch (NSException *e) {
        return -9999;
    }
}

// ── CoreFoundation Helpers ──────────────────────────────────────────────────

// Safe CFRetain — some apps return objects that throw on retain.
CFTypeRef safe_cf_retain(CFTypeRef cf) {
    @try {
        if (cf != NULL) {
            return CFRetain(cf);
        }
        return NULL;
    } @catch (NSException *e) {
        return NULL;
    }
}

// Safe CFRelease.
void safe_cf_release(CFTypeRef cf) {
    @try {
        if (cf != NULL) {
            CFRelease(cf);
        }
    } @catch (NSException *e) {
        // Swallow exception — leaking is better than crashing
    }
}

// Safe CFGetTypeID.
CFTypeID safe_cf_get_type_id(CFTypeRef cf) {
    @try {
        if (cf != NULL) {
            return CFGetTypeID(cf);
        }
        return 0;
    } @catch (NSException *e) {
        return 0;
    }
}

// Safe CFArrayGetCount.
CFIndex safe_cf_array_get_count(CFArrayRef arr) {
    @try {
        if (arr != NULL) {
            return CFArrayGetCount(arr);
        }
        return 0;
    } @catch (NSException *e) {
        return 0;
    }
}

// Safe CFArrayGetValueAtIndex.
CFTypeRef safe_cf_array_get_value(CFArrayRef arr, CFIndex idx) {
    @try {
        if (arr != NULL) {
            return CFArrayGetValueAtIndex(arr, idx);
        }
        return NULL;
    } @catch (NSException *e) {
        return NULL;
    }
}

// Safe CFBooleanGetValue.
Boolean safe_cf_boolean_get_value(CFTypeRef b) {
    @try {
        if (b != NULL) {
            return CFBooleanGetValue(b);
        }
        return false;
    } @catch (NSException *e) {
        return false;
    }
}

// Safe CFEqual. AXUIElementRefs compare by underlying accessibility object,
// which is what the container-selection membership check in ax.rs relies on.
Boolean safe_cf_equal(CFTypeRef a, CFTypeRef b) {
    @try {
        if (a != NULL && b != NULL) {
            return CFEqual(a, b);
        }
        return false;
    } @catch (NSException *e) {
        return false;
    }
}

// Safe CFNumberGetValue.
Boolean safe_cf_number_get_value(CFNumberRef num, CFNumberType type, void *valuePtr) {
    @try {
        if (num != NULL) {
            return CFNumberGetValue(num, type, valuePtr);
        }
        return false;
    } @catch (NSException *e) {
        return false;
    }
}

// Safe CFDictionaryGetValue.
CFTypeRef safe_cf_dict_get_value(CFDictionaryRef dict, CFTypeRef key) {
    @try {
        if (dict != NULL && key != NULL) {
            return CFDictionaryGetValue(dict, key);
        }
        return NULL;
    } @catch (NSException *e) {
        return NULL;
    }
}

// Safe wrappers for the well-known CF type-ID functions. These are called
// frequently (once per type check) so we wrap each one to keep the call sites
// uniform with the rest of the safe_cf_* API. They're defined in system
// frameworks and shouldn't throw, but we wrap them for consistency and
// defence-in-depth.
CFTypeID safe_cf_string_get_type_id(void) {
    @try {
        return CFStringGetTypeID();
    } @catch (NSException *e) {
        return 0;
    }
}

CFTypeID safe_cf_number_get_type_id(void) {
    @try {
        return CFNumberGetTypeID();
    } @catch (NSException *e) {
        return 0;
    }
}

CFTypeID safe_cf_boolean_get_type_id(void) {
    @try {
        return CFBooleanGetTypeID();
    } @catch (NSException *e) {
        return 0;
    }
}

CFTypeID safe_cf_array_get_type_id(void) {
    @try {
        return CFArrayGetTypeID();
    } @catch (NSException *e) {
        return 0;
    }
}

// Safe CFArrayCreate. Creates a new CFArray from an array of CFTypeRef
// pointers using kCFTypeArrayCallBacks (retains each value). Returns NULL
// on failure or if an ObjC exception was thrown.
CFArrayRef safe_cf_array_create(const void **values, CFIndex num_values) {
    @try {
        return CFArrayCreate(NULL, values, num_values, &kCFTypeArrayCallBacks);
    } @catch (NSException *e) {
        return NULL;
    }
}

// ── Process Trust ─────────────────────────────────────────────────────────────

// Safe wrapper for AXIsProcessTrusted.
Boolean safe_ax_is_process_trusted(void) {
    @try {
        return AXIsProcessTrusted();
    } @catch (NSException *e) {
        return false;
    }
}

// ── AXObserver Helpers (for EventProvider) ───────────────────────────────────

// Create an AXObserver for a given PID with a callback.
int safe_ax_observer_create(pid_t pid, AXObserverCallback callback, AXObserverRef *outObserver) {
    @try {
        return AXObserverCreate(pid, callback, outObserver);
    } @catch (NSException *e) {
        *outObserver = NULL;
        return -9999;
    }
}

// Add a notification to an AXObserver.
int safe_ax_observer_add_notification(AXObserverRef observer, AXUIElementRef element,
                                       CFStringRef notification, void *refcon) {
    @try {
        return AXObserverAddNotification(observer, element, notification, refcon);
    } @catch (NSException *e) {
        return -9999;
    }
}

// Remove a notification from an AXObserver.
int safe_ax_observer_remove_notification(AXObserverRef observer, AXUIElementRef element,
                                          CFStringRef notification) {
    @try {
        return AXObserverRemoveNotification(observer, element, notification);
    } @catch (NSException *e) {
        return -9999;
    }
}

// Get the RunLoop source for an AXObserver.
CFRunLoopSourceRef safe_ax_observer_get_run_loop_source(AXObserverRef observer) {
    @try {
        return AXObserverGetRunLoopSource(observer);
    } @catch (NSException *e) {
        return NULL;
    }
}

// Add a source to the current thread's RunLoop (default mode).
void safe_cf_run_loop_add_source(CFRunLoopSourceRef source) {
    @try {
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopDefaultMode);
    } @catch (NSException *e) {
        // swallow
    }
}

// Get the current thread's RunLoop.
CFRunLoopRef safe_cf_run_loop_get_current(void) {
    return CFRunLoopGetCurrent();
}

// Run the current thread's RunLoop (blocks until stopped).
void safe_cf_run_loop_run(void) {
    CFRunLoopRun();
}

// Stop a RunLoop (thread-safe, can be called from any thread).
void safe_cf_run_loop_stop(CFRunLoopRef rl) {
    @try {
        if (rl != NULL) {
            CFRunLoopStop(rl);
        }
    } @catch (NSException *e) {
        // swallow
    }
}

// Create an AXValue containing a CFRange (for AXSelectedTextRange).
CFTypeRef safe_ax_value_create_cf_range(CFIndex location, CFIndex length) {
    @try {
        CFRange range = CFRangeMake(location, length);
        return AXValueCreate(kAXValueCFRangeType, &range);
    } @catch (NSException *e) {
        return NULL;
    }
}

// Create an AXValue containing a CGPoint (for AXPosition).
CFTypeRef safe_ax_value_create_cg_point(double x, double y) {
    @try {
        CGPoint point = CGPointMake(x, y);
        return AXValueCreate(kAXValueCGPointType, &point);
    } @catch (NSException *e) {
        return NULL;
    }
}

// Create an AXValue containing a CGSize (for AXSize).
CFTypeRef safe_ax_value_create_cg_size(double width, double height) {
    @try {
        CGSize size = CGSizeMake(width, height);
        return AXValueCreate(kAXValueCGSizeType, &size);
    } @catch (NSException *e) {
        return NULL;
    }
}

// ── Screen Capture (ScreenCaptureKit) ────────────────────────────────────────

// Capture the primary display (or a sub-rect in logical screen points) into an
// RGBA8 buffer. The buffer is malloc'd and ownership transfers to the caller,
// who must free it via `safe_cg_free_pixels`.
//
// Uses ScreenCaptureKit's SCScreenshotManager. CGDisplayCreateImage family was
// obsoleted in macOS 15.0; SCK is the only supported path forward.
//
// `use_rect`: 0 = full display, non-zero = use rect_x/y/w/h (logical points,
//   in global screen space — same coords as Element.bounds).
// `out_pixels`: on success, points to `(*out_width) * (*out_height) * 4` bytes
//   of RGBA8888 data (R, G, B, A per pixel; row-major; tightly packed).
// `out_scale`: ratio of physical image pixels to logical input points
//   (1.0 on standard displays, 2.0 on typical Retina).
//
// Returns 0 on success; negative value on failure:
//   -1  SCShareableContent query failed / no displays
//   -2  SCScreenshotManager returned no image
//   -3  pixel buffer allocation failed
//   -4  bitmap context creation failed
//   -5  requested rect has zero / negative dimensions
//   -9999 ObjC exception
// `out_origin_x` / `out_origin_y` receive the **logical** screen coordinate
// that the returned image's pixel (0, 0) sits at: the requested rect's origin
// for a region capture, and the captured display's `frame.origin` for a full
// one. The display we capture is `content.displays[0]`, which is not
// necessarily the display at (0, 0), so a caller that assumed (0, 0) would
// place everything it maps onto these pixels off by that display's offset.
int safe_cg_capture_rgba(
    int use_rect,
    double rect_x, double rect_y, double rect_w, double rect_h,
    uint8_t **out_pixels,
    uint32_t *out_width,
    uint32_t *out_height,
    double *out_scale,
    double *out_origin_x,
    double *out_origin_y
) {
    @try {
        if (out_pixels) *out_pixels = NULL;
        if (out_width) *out_width = 0;
        if (out_height) *out_height = 0;
        if (out_scale) *out_scale = 1.0;
        if (out_origin_x) *out_origin_x = 0.0;
        if (out_origin_y) *out_origin_y = 0.0;

        if (use_rect && (rect_w <= 0.0 || rect_h <= 0.0)) {
            return -5;
        }

        // Step 1: fetch shareable content (async → sync via semaphore).
        __block SCShareableContent *content = nil;
        __block NSError *contentError = nil;
        dispatch_semaphore_t sem1 = dispatch_semaphore_create(0);
        [SCShareableContent getShareableContentWithCompletionHandler:
            ^(SCShareableContent *c, NSError *e) {
                content = c;
                contentError = e;
                dispatch_semaphore_signal(sem1);
            }];
        dispatch_semaphore_wait(sem1, DISPATCH_TIME_FOREVER);
        if (content == nil || content.displays.count == 0) {
            if (contentError) {
                NSLog(@"xa11y SCShareableContent error: %@", contentError);
            }
            return -1;
        }

        SCDisplay *display = content.displays[0];
        CGRect logical_bounds = display.frame;
        double disp_scale = (logical_bounds.size.width > 0.0)
            ? ((double)display.width / (double)logical_bounds.size.width)
            : 1.0;

        // Step 2: build the capture configuration.
        double src_x = use_rect ? rect_x : logical_bounds.origin.x;
        double src_y = use_rect ? rect_y : logical_bounds.origin.y;
        double src_w = use_rect ? rect_w : logical_bounds.size.width;
        double src_h = use_rect ? rect_h : logical_bounds.size.height;

        size_t phys_w = (size_t)(src_w * disp_scale + 0.5);
        size_t phys_h = (size_t)(src_h * disp_scale + 0.5);
        if (phys_w == 0 || phys_h == 0) return -5;

        SCContentFilter *filter = [[SCContentFilter alloc]
            initWithDisplay:display excludingWindows:@[]];
        SCStreamConfiguration *config = [[SCStreamConfiguration alloc] init];
        config.width = phys_w;
        config.height = phys_h;
        config.pixelFormat = 'BGRA';   // kCVPixelFormatType_32BGRA
        config.colorSpaceName = kCGColorSpaceSRGB;
        config.showsCursor = NO;
        if (use_rect) {
            // SCK sourceRect is in the display's local (logical) coordinates.
            config.sourceRect = CGRectMake(
                src_x - logical_bounds.origin.x,
                src_y - logical_bounds.origin.y,
                src_w, src_h
            );
            config.destinationRect = CGRectMake(0, 0, (CGFloat)phys_w, (CGFloat)phys_h);
        }

        // Step 3: capture one image (async → sync via semaphore).
        __block CGImageRef image = NULL;
        __block NSError *captureError = nil;
        dispatch_semaphore_t sem2 = dispatch_semaphore_create(0);
        [SCScreenshotManager
            captureImageWithFilter:filter
                    configuration:config
                completionHandler:^(CGImageRef img, NSError *e) {
                    captureError = e;
                    if (img) {
                        image = CGImageRetain(img);
                    }
                    dispatch_semaphore_signal(sem2);
                }];
        dispatch_semaphore_wait(sem2, DISPATCH_TIME_FOREVER);
        if (image == NULL) {
            if (captureError) {
                NSLog(@"xa11y SCScreenshotManager error: %@", captureError);
            }
            return -2;
        }

        size_t w = CGImageGetWidth(image);
        size_t h = CGImageGetHeight(image);
        if (w == 0 || h == 0) {
            CGImageRelease(image);
            return -2;
        }

        // Step 4: blit into a fresh RGBA8 buffer.
        size_t row_bytes = w * 4;
        size_t buf_size = row_bytes * h;
        uint8_t *buf = (uint8_t *)malloc(buf_size);
        if (buf == NULL) {
            CGImageRelease(image);
            return -3;
        }
        memset(buf, 0, buf_size);

        CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
        // RGBA8888 big-endian — R, G, B, A in memory order.
        CGContextRef ctx = CGBitmapContextCreate(
            buf, w, h, 8, row_bytes, cs,
            kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big
        );
        CGColorSpaceRelease(cs);
        if (ctx == NULL) {
            free(buf);
            CGImageRelease(image);
            return -4;
        }
        CGContextDrawImage(ctx, CGRectMake(0, 0, (CGFloat)w, (CGFloat)h), image);
        CGContextRelease(ctx);
        CGImageRelease(image);

        *out_pixels = buf;
        *out_width = (uint32_t)w;
        *out_height = (uint32_t)h;
        *out_scale = disp_scale;
        // `src_x` / `src_y` are already the logical top-left of what was
        // captured: the caller's rect for a region, the display's frame origin
        // for a full capture.
        if (out_origin_x) *out_origin_x = src_x;
        if (out_origin_y) *out_origin_y = src_y;
        return 0;
    } @catch (NSException *e) {
        return -9999;
    }
}

// Release a buffer returned from `safe_cg_capture_rgba`.
void safe_cg_free_pixels(uint8_t *pixels) {
    if (pixels != NULL) {
        free(pixels);
    }
}

// ── Test Helpers ─────────────────────────────────────────────────────────────

// [M4] An AXValue carrying an AXError, as a batch slot holds one.
CFTypeRef test_ax_value_create_error(int code) {
    AXError e = code;
    return AXValueCreate(kAXValueAXErrorType, &e);
}

// Throw an NSException (for testing that our Rust code handles it properly).
// Returns 0 if no exception was caught (should not happen), 1 if caught.
int test_throw_and_catch_nsexception(void) {
    @try {
        @throw [NSException exceptionWithName:@"TestException"
                                       reason:@"deliberate test throw"
                                     userInfo:nil];
        return 0;
    } @catch (NSException *e) {
        return 1;
    }
}
