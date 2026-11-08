#import <Cocoa/Cocoa.h>
#import <AudioUnit/AUCocoaUIView.h>
#import <AudioToolbox/AudioToolbox.h>
#include "bridge.h"
#include "editor.h"
#include "opengridder.h"
#include "OpenGridderEditorView.h"
#include <vector>
#include <map>
#include <cmath>
#import <ImageIO/ImageIO.h>

@interface AGOpenBridgeView : NSView {
    void* _processor;
    void* _owner;
    AgEdit _edit;
    AgGesture _gesture;
    OpenGridderEditorView* _editor;
}
-(id)initWithProcessor:(void*)processor owner:(void*)owner edit:(AgEdit)edit gesture:(AgGesture)gesture;
@end

@implementation AGOpenBridgeView

-(id)initWithProcessor:(void*)processor owner:(void*)owner edit:(AgEdit)edit gesture:(AgGesture)gesture {
    NSRect initialRect = NSMakeRect(0, 0, 864, 420);
    if ((self = [super initWithFrame:initialRect])) {
        if (!ag_insert_retain(processor)) return nil;
        _processor = processor;
        _owner = owner;
        _edit = edit;
        _gesture = gesture;

        _editor = [[OpenGridderEditorView alloc] initWithFrame:initialRect];
        _editor.rust = (NativePlugin*)processor;

        __weak AGOpenBridgeView* weakSelf = self;
        _editor.onParameterChanged = ^(uint32_t paramId, float normVal) {
            AGOpenBridgeView* s = weakSelf;
            if (s && s->_processor && ag_insert_alive(s->_processor)) {
                if (s->_edit) {
                    s->_edit(s->_owner, paramId, normVal);
                } else {
                    ag_insert_set(s->_processor, paramId, normVal);
                }
            }
        };

        _editor.onSizeChanged = ^(CGFloat newWidth, CGFloat newHeight) {
            AGOpenBridgeView* s = weakSelf;
            if (s) {
                [s setFrameSize:NSMakeSize(newWidth, newHeight)];
            }
        };

        [self addSubview:_editor];
    }
    return self;
}

-(void)shutdown {
    if (_editor) {
        [_editor removeFromSuperview];
        _editor = nil;
    }
    if (_processor) {
        ag_insert_release(_processor);
        _processor = nullptr;
    }
    _owner = nullptr;
    _edit = nullptr;
    _gesture = nullptr;
}

-(void)dealloc {
    [self shutdown];
}

-(BOOL)isFlipped { return YES; }
-(BOOL)isOpaque { return YES; }
-(BOOL)acceptsFirstResponder { return YES; }

-(void)setFrameSize:(NSSize)newSize {
    [super setFrameSize:newSize];
    if (_editor) {
        [_editor setFrameSize:newSize];
    }
}

@end

extern "C" void* ag_editor_create(void* processor, void* owner, AgEdit edit, AgGesture gesture) {
    AGOpenBridgeView* view = [[AGOpenBridgeView alloc] initWithProcessor:processor owner:owner edit:edit gesture:gesture];
    return (__bridge_retained void*)view;
}

extern "C" void ag_editor_release(void* view) {
    if (view) {
        AGOpenBridgeView* owned = (__bridge_transfer AGOpenBridgeView*)view;
        [owned shutdown];
        [owned removeFromSuperview];
    }
}

extern "C" bool ag_editor_attach(void* raw, void* parent) {
    if (!raw || !parent) return false;
    [(__bridge NSView*)parent addSubview:(__bridge NSView*)raw];
    return true;
}

@interface AGOpenCocoaViewFactory : NSObject<AUCocoaUIBase>
@end

static void auEdit(void* owner, uint32_t i, double value) {
    double lo, hi;
    ag_insert_param_info(i, nullptr, &lo, &hi, nullptr, nullptr);
    AudioUnitSetParameter((AudioUnit)owner, i, kAudioUnitScope_Global, 0, lo + value * (hi - lo), 0);
    AudioUnitParameter p{(AudioUnit)owner, i, kAudioUnitScope_Global, 0};
    AUParameterListenerNotify(nullptr, nullptr, &p);
}

static void auGesture(void* owner, uint32_t i, bool start) {
    AudioUnitEvent event{};
    event.mEventType = start ? kAudioUnitEvent_BeginParameterChangeGesture : kAudioUnitEvent_EndParameterChangeGesture;
    event.mArgument.mParameter = {(AudioUnit)owner, i, kAudioUnitScope_Global, 0};
    AUEventListenerNotify(nullptr, nullptr, &event);
}

@implementation AGOpenCocoaViewFactory
-(unsigned)interfaceVersion { return 0; }
-(NSView*)uiViewForAudioUnit:(AudioUnit)unit withSize:(NSSize)size {
    (void)size;
    void* processor = nullptr;
    UInt32 bytes = sizeof(processor);
    if (AudioUnitGetProperty(unit, 65536, 0, 0, &processor, &bytes) != 0 || !processor) return nil;
    void* raw = ag_editor_create(processor, unit, auEdit, auGesture);
    return (__bridge_transfer NSView*)raw;
}
@end

// C-ABI compatibility implementation for OpenGridder Editor View to talk to ag_insert
extern "C" {

uint32_t og_native_default_latency(void) { return 256; }

NativePlugin* og_native_create(double rate, uint32_t max_frames, uint32_t channels, uint32_t latency) {
    (void)rate; (void)max_frames; (void)channels; (void)latency;
    return (NativePlugin*)ag_insert_create();
}

void og_native_destroy(NativePlugin* p) {
    ag_insert_destroy((void*)p);
}

int32_t og_native_process(NativePlugin* p, const float* const* inputs, float* const* outputs, uint32_t frames) {
    return ag_insert_process((void*)p, inputs, outputs, 2, frames, false) ? 0 : -1;
}

void og_native_select(NativePlugin* p, uint32_t index) {
    ag_insert_set((void*)p, 1, (double)index / 128.0);
}

void og_native_bypass(NativePlugin* p, bool bypass) {
    ag_insert_set((void*)p, 0, bypass ? 1.0 : 0.0);
}

uint32_t og_native_latency(NativePlugin* p) {
    return ag_insert_latency((void*)p);
}

void og_native_accept_latency(NativePlugin* p, uint32_t total) {
    (void)p; (void)total;
}

void og_native_set_transport_latency(NativePlugin* p, uint32_t transport) {
    (void)p; (void)transport;
}

int32_t og_native_status(NativePlugin* p) {
    return ag_insert_status((void*)p);
}

void og_native_name(NativePlugin* p, uint32_t index, char* buffer, uint32_t capacity) {
    (void)p; (void)index;
    if (buffer && capacity > 0) snprintf(buffer, capacity, "OpenGridder");
}

uint32_t og_native_num_loaded_plugins(NativePlugin* p) {
    return ag_insert_num_loaded_plugins((void*)p);
}

bool og_native_get_loaded_plugin(NativePlugin* p, uint32_t idx, char* name_buf, uint32_t name_cap, bool* bypassed, uint32_t* oversampling) {
    return ag_insert_get_loaded_plugin((void*)p, idx, name_buf, name_cap, bypassed, oversampling);
}

uint32_t og_native_num_available_plugins(NativePlugin* p) {
    return ag_insert_num_available_plugins((void*)p);
}

bool og_native_get_available_plugin(NativePlugin* p, uint32_t idx, char* name_buf, uint32_t name_cap, char* id_buf, uint32_t id_cap, char* cat_buf, uint32_t cat_cap) {
    char type_buf[32] = {0};
    char company_buf[64] = {0};
    return ag_insert_get_available_plugin_ext((void*)p, idx, name_buf, name_cap, id_buf, id_cap, type_buf, sizeof(type_buf), company_buf, sizeof(company_buf), cat_buf, cat_cap);
}

bool og_native_get_available_plugin_ext(NativePlugin* p, uint32_t idx, char* name_buf, uint32_t name_cap, char* id_buf, uint32_t id_cap, char* type_buf, uint32_t type_cap, char* company_buf, uint32_t company_cap, char* cat_buf, uint32_t cat_cap) {
    return ag_insert_get_available_plugin_ext((void*)p, idx, name_buf, name_cap, id_buf, id_cap, type_buf, type_cap, company_buf, company_cap, cat_buf, cat_cap);
}

bool og_native_load_plugin(NativePlugin* p, uint32_t available_idx) {
    return ag_insert_load_plugin((void*)p, available_idx);
}

bool og_native_load_plugin_at(NativePlugin* p, uint32_t available_idx, int32_t x, int32_t y) {
    (void)x; (void)y;
    return ag_insert_load_plugin((void*)p, available_idx);
}

bool og_native_unload_plugin(NativePlugin* p, uint32_t loaded_idx) {
    return ag_insert_unload_plugin((void*)p, loaded_idx);
}

bool og_native_set_plugin_oversampling(NativePlugin* p, uint32_t loaded_idx, uint32_t factor) {
    return ag_insert_set_plugin_oversampling((void*)p, loaded_idx, factor);
}

bool og_native_set_plugin_bypassed(NativePlugin* p, uint32_t loaded_idx, bool bypassed) {
    return ag_insert_set_plugin_bypassed((void*)p, loaded_idx, bypassed);
}

bool og_native_edit_plugin(NativePlugin* p, uint32_t loaded_idx, int32_t x, int32_t y) {
    // (x, y) is a Cocoa screen point (bottom-left origin) for the server window's top-left;
    // the protocol uses top-left-origin coordinates (ProcessorWindow::move).
    const CGFloat primary = NSMaxY([NSScreen screens].firstObject.frame);
    return ag_insert_edit_plugin_at((void*)p, loaded_idx, x, (int32_t)lround(primary - y));
}

bool og_native_hide_plugin(NativePlugin* p) {
    return ag_insert_hide_plugin((void*)p);
}

// The remote editor picture: the server sends PNG frames where unchanged pixels are
// transparent (AudioGridder's diff detection), so each frame is drawn over the previous
// picture in order. One canvas per insert.
struct AgScreenCanvas {
    std::vector<uint8_t> rgba;
    uint32_t width = 0;
    uint32_t height = 0;
    uint64_t frame_id = 0;
};

static std::map<NativePlugin*, AgScreenCanvas>& screen_canvases() {
    static std::map<NativePlugin*, AgScreenCanvas> map;
    return map;
}

static void apply_screen_frame(void* ctx, const uint8_t* png, uint32_t len, int32_t width, int32_t height) {
    auto* canvas = static_cast<AgScreenCanvas*>(ctx);
    if (!png || len == 0 || width <= 0 || height <= 0) return;
    CFDataRef data = CFDataCreate(NULL, png, len);
    CGImageSourceRef source = CGImageSourceCreateWithData(data, NULL);
    CFRelease(data);
    if (!source) return;
    CGImageRef image = CGImageSourceCreateImageAtIndex(source, 0, NULL);
    CFRelease(source);
    if (!image) return;
    if (canvas->width != (uint32_t)width || canvas->height != (uint32_t)height) {
        canvas->width = (uint32_t)width;
        canvas->height = (uint32_t)height;
        canvas->rgba.assign((size_t)width * height * 4, 0);
    }
    CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef ctx2 = CGBitmapContextCreate(canvas->rgba.data(), width, height, 8, (size_t)width * 4, space,
                                              kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(space);
    if (ctx2) {
        CGContextDrawImage(ctx2, CGRectMake(0, 0, width, height), image);  // source-over: transparent keeps
        CGContextRelease(ctx2);
        canvas->frame_id++;
    }
    CGImageRelease(image);
}

bool og_native_get_screen_info(NativePlugin* p, uint32_t* width, uint32_t* height, double* scale, uint64_t* frame_id) {
    if (!p) return false;
    AgScreenCanvas& canvas = screen_canvases()[p];
    while (ag_insert_next_screen((void*)p, apply_screen_frame, &canvas)) {
    }
    if (canvas.width == 0 || canvas.height == 0) return false;
    if (width) *width = canvas.width;
    if (height) *height = canvas.height;
    if (scale) *scale = 1.0;
    if (frame_id) *frame_id = canvas.frame_id;
    return true;
}

bool og_native_copy_screen_frame(NativePlugin* p, uint8_t* dest, uint32_t dest_cap, uint64_t* frame_id) {
    auto it = screen_canvases().find(p);
    if (it == screen_canvases().end() || !dest) return false;
    const AgScreenCanvas& canvas = it->second;
    if (canvas.rgba.empty() || dest_cap < canvas.rgba.size()) return false;
    memcpy(dest, canvas.rgba.data(), canvas.rgba.size());
    if (frame_id) *frame_id = canvas.frame_id;
    return true;
}

int32_t og_native_pop_screen_message(NativePlugin* p, uint8_t* dest, uint32_t dest_cap, uint32_t* out_len) {
    (void)p; (void)dest; (void)dest_cap; (void)out_len;
    return 0;
}

void og_native_send_mouse(NativePlugin* p, uint8_t ev_type, float x, float y, bool shift, bool ctrl, bool alt, float delta_x, float delta_y, bool is_smooth) {
    (void)is_smooth;
    ag_insert_send_mouse((void*)p, ev_type, x, y, shift, ctrl, alt, delta_x, delta_y);
}

void og_native_send_key(NativePlugin* p, const uint16_t* keys, uint32_t count) {
    ag_insert_send_key((void*)p, keys, count);
}

bool og_native_is_editor_open(NativePlugin* p) {
    return ag_insert_editor_open((void*)p);
}

bool og_native_is_server_local_mode(NativePlugin* p) {
    return ag_insert_server_is_local((void*)p);
}

void og_native_set_offline(NativePlugin* p, bool offline) {
    (void)p; (void)offline;
}

void og_native_set_param(NativePlugin* p, uint32_t param_id, float norm_val) {
    ag_insert_set((void*)p, param_id, norm_val);
}

float og_native_get_param(NativePlugin* p, uint32_t param_id) {
    return (float)ag_insert_get((void*)p, param_id);
}

void og_native_get_meters(NativePlugin* p, float* left, float* right) {
    if (left) *left = (float)ag_insert_get((void*)p, 6);
    if (right) *right = (float)ag_insert_get((void*)p, 6);
}

void og_native_get_input_meters(NativePlugin* p, float* left, float* right) {
    if (left) *left = (float)ag_insert_get((void*)p, 3);
    if (right) *right = (float)ag_insert_get((void*)p, 3);
}

bool og_native_exchange_plugins(NativePlugin* p, uint32_t idx_a, uint32_t idx_b) {
    return ag_insert_exchange_plugins((void*)p, idx_a, idx_b);
}

uint32_t og_native_num_presets(NativePlugin* p, uint32_t loaded_idx) {
    return ag_insert_num_presets((void*)p, loaded_idx);
}

bool og_native_get_preset_name(NativePlugin* p, uint32_t loaded_idx, uint32_t preset_idx, char* buf, uint32_t cap) {
    return ag_insert_get_preset_name((void*)p, loaded_idx, preset_idx, buf, cap);
}

bool og_native_set_preset(NativePlugin* p, uint32_t loaded_idx, uint32_t preset_idx) {
    return ag_insert_set_preset((void*)p, loaded_idx, preset_idx);
}

uint32_t og_native_num_plugin_params(NativePlugin* p, uint32_t loaded_idx) {
    return ag_insert_num_plugin_params((void*)p, loaded_idx);
}

bool og_native_get_plugin_param_info(NativePlugin* p, uint32_t loaded_idx, uint32_t param_idx, char* name_buf, uint32_t name_cap, float* default_val, float* current_val) {
    return ag_insert_get_plugin_param_info((void*)p, loaded_idx, param_idx, name_buf, name_cap, default_val, current_val);
}

bool og_native_set_plugin_param(NativePlugin* p, uint32_t loaded_idx, uint32_t param_idx, float val) {
    return ag_insert_set_plugin_param((void*)p, loaded_idx, param_idx, val);
}

void og_native_store_a(NativePlugin* p) { (void)p; }
void og_native_store_b(NativePlugin* p) { (void)p; }
void og_native_toggle_ab(NativePlugin* p) { (void)p; }
bool og_native_is_b(NativePlugin* p) { (void)p; return false; }

void og_native_get_active_server(NativePlugin* p, char* buf, uint32_t cap) {
    (void)p;
    if (buf && cap > 0) snprintf(buf, cap, "localhost:55056");
}

bool og_native_set_server(NativePlugin* p, const char* host_port) {
    return ag_insert_connect((void*)p, host_port);
}

uint32_t og_native_num_discovered_servers(NativePlugin* p) {
    (void)p;
    return 1;
}

bool og_native_get_discovered_server(NativePlugin* p, uint32_t idx, char* buf, uint32_t cap) {
    (void)p; (void)idx;
    if (buf && cap > 0) snprintf(buf, cap, "localhost:55056");
    return true;
}

uint32_t og_native_get_state(NativePlugin* p, uint8_t* buf, uint32_t cap) {
    return ag_insert_save((void*)p, buf, cap);
}

bool og_native_set_state(NativePlugin* p, const uint8_t* data, uint32_t len) {
    return ag_insert_load((void*)p, data, len);
}

void og_native_queue_midi(NativePlugin* p, int32_t sample_offset, uint8_t status, uint8_t data1, uint8_t data2) {
    ag_insert_midi((void*)p, (uint32_t)sample_offset, status, data1, data2);
}

int32_t og_native_process_f64(NativePlugin* p, const double* const* inputs, double* const* outputs, uint32_t frames) {
    return ag_insert_process64((void*)p, inputs, outputs, 2, frames, false) ? 0 : -1;
}

}
