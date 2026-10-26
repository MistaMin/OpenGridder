// OpenGridder CLAP Plugin Adapter (Zero-Overhead C ABI + Cocoa NSView GUI)
#define OG_BUILD_CLAP 1
#define OPENGRIDDER_VIEW_CLASS_NAME OpenGridderCLAPEditorView
#define OPENGRIDDER_REMOTE_VIEW_CLASS_NAME OpenGridderCLAPRemoteView
#define OPENGRIDDER_REMOTE_WC_CLASS_NAME OpenGridderCLAPRemoteWindowController

#import <Cocoa/Cocoa.h>
#import "OpenGridderEditorView.h"
#import "OpenGridderEditorView.mm"
#include "opengridder.h"

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <cstdio>
#include <algorithm>
#include <vector>

extern "C" {

// --- Minimal Self-Contained CLAP 1.2 C ABI Definitions ---
typedef struct clap_version {
    uint32_t major;
    uint32_t minor;
    uint32_t revision;
} clap_version_t;

static constexpr clap_version_t CLAP_VERSION = { 1, 2, 0 };
static constexpr uint32_t CLAP_NAME_SIZE = 256;
static constexpr uint32_t CLAP_PATH_SIZE = 1024;
typedef uint32_t clap_id;
static constexpr clap_id CLAP_INVALID_ID = 0xffffffff;

struct clap_host;
struct clap_plugin;

typedef struct clap_plugin_descriptor {
    clap_version_t clap_version;
    const char* id;
    const char* name;
    const char* vendor;
    const char* url;
    const char* manual_url;
    const char* support_url;
    const char* version;
    const char* description;
    const char* const* features;
} clap_plugin_descriptor_t;

typedef struct clap_host {
    clap_version_t clap_version;
    void* host_data;
    const char* name;
    const char* vendor;
    const char* url;
    const char* version;
    const void* (*get_extension)(const struct clap_host* host, const char* extension_id);
    void (*request_restart)(const struct clap_host* host);
    void (*request_process)(const struct clap_host* host);
    void (*request_callback)(const struct clap_host* host);
} clap_host_t;

// Events
typedef struct clap_event_header {
    uint32_t size;
    uint32_t time;
    uint16_t space_id;
    uint16_t type;
    uint32_t flags;
} clap_event_header_t;

enum {
    CLAP_EVENT_NOTE_ON = 0,
    CLAP_EVENT_NOTE_OFF = 1,
    CLAP_EVENT_PARAM_VALUE = 5,
    CLAP_EVENT_MIDI = 10
};

typedef struct clap_event_param_value {
    clap_event_header_t header;
    clap_id param_id;
    void* cookie;
    int32_t note_id;
    int16_t port_index;
    int16_t channel;
    int16_t key;
    double value;
} clap_event_param_value_t;

typedef struct clap_event_midi {
    clap_event_header_t header;
    uint16_t port_index;
    uint8_t data[3];
} clap_event_midi_t;

typedef struct clap_input_events {
    void* ctx;
    uint32_t (*size)(const struct clap_input_events* list);
    const clap_event_header_t* (*get)(const struct clap_input_events* list, uint32_t index);
} clap_input_events_t;

typedef struct clap_output_events {
    void* ctx;
    bool (*try_push)(const struct clap_output_events* list, const clap_event_header_t* event);
} clap_output_events_t;

typedef struct clap_audio_buffer {
    float** data32;
    double** data64;
    uint32_t channel_count;
    uint32_t latency;
    uint64_t constant_mask;
} clap_audio_buffer_t;

typedef struct clap_process {
    int64_t steady_time;
    uint32_t frames_count;
    const void* transport;
    const clap_audio_buffer_t* audio_inputs;
    clap_audio_buffer_t* audio_outputs;
    uint32_t audio_inputs_count;
    uint32_t audio_outputs_count;
    const clap_input_events_t* in_events;
    const clap_output_events_t* out_events;
} clap_process_t;

enum {
    CLAP_PROCESS_ERROR = 0,
    CLAP_PROCESS_CONTINUE = 1
};
typedef int32_t clap_process_status;

typedef struct clap_plugin {
    const clap_plugin_descriptor_t* desc;
    void* plugin_data;
    bool (*init)(const struct clap_plugin* plugin);
    void (*destroy)(const struct clap_plugin* plugin);
    bool (*activate)(const struct clap_plugin* plugin, double sample_rate, uint32_t min_frames_count, uint32_t max_frames_count);
    void (*deactivate)(const struct clap_plugin* plugin);
    bool (*start_processing)(const struct clap_plugin* plugin);
    void (*stop_processing)(const struct clap_plugin* plugin);
    void (*reset)(const struct clap_plugin* plugin);
    clap_process_status (*process)(const struct clap_plugin* plugin, const clap_process_t* process);
    const void* (*get_extension)(const struct clap_plugin* plugin, const char* id);
    void (*on_main_thread)(const struct clap_plugin* plugin);
} clap_plugin_t;

// Audio Ports Extension
typedef struct clap_audio_port_info {
    clap_id id;
    char name[CLAP_NAME_SIZE];
    uint32_t flags;
    uint32_t channel_count;
    const char* port_type;
    clap_id in_place_pair;
} clap_audio_port_info_t;

typedef struct clap_plugin_audio_ports {
    uint32_t (*count)(const clap_plugin_t* plugin, bool is_input);
    bool (*get)(const clap_plugin_t* plugin, uint32_t index, bool is_input, clap_audio_port_info_t* info);
} clap_plugin_audio_ports_t;

// Note Ports Extension
typedef struct clap_note_port_info {
    clap_id id;
    uint32_t supported_dialects;
    uint32_t preferred_dialect;
    char name[CLAP_NAME_SIZE];
} clap_note_port_info_t;

typedef struct clap_plugin_note_ports {
    uint32_t (*count)(const clap_plugin_t* plugin, bool is_input);
    bool (*get)(const clap_plugin_t* plugin, uint32_t index, bool is_input, clap_note_port_info_t* info);
} clap_plugin_note_ports_t;

// Latency Extension
typedef struct clap_plugin_latency {
    uint32_t (*get)(const clap_plugin_t* plugin);
} clap_plugin_latency_t;

// Params Extension
typedef struct clap_param_info {
    clap_id id;
    uint32_t flags;
    void* cookie;
    char name[CLAP_NAME_SIZE];
    char module[CLAP_PATH_SIZE];
    double min_value;
    double max_value;
    double default_value;
} clap_param_info_t;

typedef struct clap_plugin_params {
    uint32_t (*count)(const clap_plugin_t* plugin);
    bool (*get_info)(const clap_plugin_t* plugin, uint32_t param_index, clap_param_info_t* param_info);
    bool (*get_value)(const clap_plugin_t* plugin, clap_id param_id, double* out_value);
    bool (*value_to_text)(const clap_plugin_t* plugin, clap_id param_id, double value, char* out_buffer, uint32_t out_buffer_capacity);
    bool (*text_to_value)(const clap_plugin_t* plugin, clap_id param_id, const char* param_value_text, double* out_value);
    void (*flush)(const clap_plugin_t* plugin, const clap_input_events_t* in, const clap_output_events_t* out);
} clap_plugin_params_t;

// State Extension
typedef struct clap_istream {
    void* ctx;
    int64_t (*read)(const struct clap_istream* stream, void* buffer, uint64_t size);
} clap_istream_t;

typedef struct clap_ostream {
    void* ctx;
    int64_t (*write)(const struct clap_ostream* stream, const void* buffer, uint64_t size);
} clap_ostream_t;

typedef struct clap_plugin_state {
    bool (*save)(const clap_plugin_t* plugin, const clap_ostream_t* stream);
    bool (*load)(const clap_plugin_t* plugin, const clap_istream_t* stream);
} clap_plugin_state_t;

// GUI Extension
typedef struct clap_window {
    const char* api;
    void* ptr;
} clap_window_t;

typedef struct clap_gui_resize_hints {
    bool can_resize_horizontally;
    bool can_resize_vertically;
    bool preserve_aspect_ratio;
    uint32_t aspect_ratio_width;
    uint32_t aspect_ratio_height;
} clap_gui_resize_hints_t;

typedef struct clap_plugin_gui {
    bool (*is_api_supported)(const clap_plugin_t* plugin, const char* api, bool is_floating);
    bool (*get_preferred_api)(const clap_plugin_t* plugin, const char** api, bool* is_floating);
    bool (*create)(const clap_plugin_t* plugin, const char* api, bool is_floating);
    void (*destroy)(const clap_plugin_t* plugin);
    bool (*set_scale)(const clap_plugin_t* plugin, double scale);
    bool (*get_size)(const clap_plugin_t* plugin, uint32_t* width, uint32_t* height);
    bool (*can_resize)(const clap_plugin_t* plugin);
    bool (*get_resize_hints)(const clap_plugin_t* plugin, clap_gui_resize_hints_t* hints);
    bool (*adjust_size)(const clap_plugin_t* plugin, uint32_t* width, uint32_t* height);
    bool (*set_size)(const clap_plugin_t* plugin, uint32_t width, uint32_t height);
    bool (*set_parent)(const clap_plugin_t* plugin, const clap_window_t* window);
    bool (*set_transient)(const clap_plugin_t* plugin, const clap_window_t* window);
    void (*suggest_title)(const clap_plugin_t* plugin, const char* title);
    bool (*show)(const clap_plugin_t* plugin);
    bool (*hide)(const clap_plugin_t* plugin);
} clap_plugin_gui_t;

typedef struct clap_plugin_factory {
    uint32_t (*get_plugin_count)(const struct clap_plugin_factory* factory);
    const clap_plugin_descriptor_t* (*get_plugin_descriptor)(const struct clap_plugin_factory* factory, uint32_t index);
    const clap_plugin_t* (*create_plugin)(const struct clap_plugin_factory* factory, const clap_host_t* host, const char* plugin_id);
} clap_plugin_factory_t;

typedef struct clap_plugin_entry {
    clap_version_t clap_version;
    bool (*init)(const char* plugin_path);
    void (*deinit)(void);
    const void* (*get_factory)(const char* factory_id);
} clap_plugin_entry_t;

// --- OpenGridder CLAP Implementation ---
struct OpenGridderCLAPData {
    clap_plugin_t plugin;
    const clap_host_t* host;
    NativePlugin* rust;
    OpenGridderCLAPEditorView* view;
    double sampleRate;
};

static const char* const kClapFeatures[] = {
    "audio-effect",
    "stereo",
    nullptr
};

static const clap_plugin_descriptor_t kOpenGridderClapDesc = {
    CLAP_VERSION,
    "studio.opengrid.OpenGridder",
    "OpenGridder",
    "OpenGrid Studio",
    "https://github.com/MistaMin/OpenGridder",
    "https://github.com/MistaMin/OpenGridder",
    "https://github.com/MistaMin/OpenGridder",
    "1.2.1",
    "OpenGridder Network DSP Offload Plugin",
    kClapFeatures
};

static void applyClapEvents(NativePlugin* rust, const clap_input_events_t* in_events) {
    if (!rust || !in_events || !in_events->size || !in_events->get) return;
    uint32_t count = in_events->size(in_events);
    for (uint32_t i = 0; i < count; ++i) {
        const clap_event_header_t* hdr = in_events->get(in_events, i);
        if (!hdr) continue;
        if (hdr->type == CLAP_EVENT_PARAM_VALUE) {
            const auto* pev = reinterpret_cast<const clap_event_param_value_t*>(hdr);
            og_native_set_param(rust, pev->param_id, static_cast<float>(pev->value));
        } else if (hdr->type == CLAP_EVENT_MIDI) {
            const auto* mev = reinterpret_cast<const clap_event_midi_t*>(hdr);
            og_native_queue_midi(rust, static_cast<int32_t>(hdr->time), mev->data[0], mev->data[1], mev->data[2]);
        }
    }
}

// Audio Ports
static uint32_t clap_audio_ports_count(const clap_plugin_t*, bool) { return 1; }
static bool clap_audio_ports_get(const clap_plugin_t*, uint32_t index, bool is_input, clap_audio_port_info_t* info) {
    if (index != 0 || !info) return false;
    std::memset(info, 0, sizeof(*info));
    info->id = 0;
    std::snprintf(info->name, sizeof(info->name), "%s", is_input ? "Audio In" : "Audio Out");
    info->flags = 1; // CLAP_AUDIO_PORT_IS_MAIN
    info->channel_count = 2;
    info->port_type = "stereo";
    info->in_place_pair = 0;
    return true;
}
static const clap_plugin_audio_ports_t kClapAudioPorts = {
    clap_audio_ports_count,
    clap_audio_ports_get
};

// Note Ports
static uint32_t clap_note_ports_count(const clap_plugin_t*, bool is_input) { return is_input ? 1 : 0; }
static bool clap_note_ports_get(const clap_plugin_t*, uint32_t index, bool is_input, clap_note_port_info_t* info) {
    if (!is_input || index != 0 || !info) return false;
    std::memset(info, 0, sizeof(*info));
    info->id = 0;
    info->supported_dialects = 1 | 2; // CLAP + MIDI
    info->preferred_dialect = 2;      // MIDI
    std::snprintf(info->name, sizeof(info->name), "MIDI In");
    return true;
}
static const clap_plugin_note_ports_t kClapNotePorts = {
    clap_note_ports_count,
    clap_note_ports_get
};

// Latency
static uint32_t clap_latency_get(const clap_plugin_t* plugin) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    return (d && d->rust) ? og_native_latency(d->rust) : 256;
}
static const clap_plugin_latency_t kClapLatency = { clap_latency_get };

// Params (7 parameters matching VST3/AU/AAX)
static uint32_t clap_params_count(const clap_plugin_t*) { return 7; }
static bool clap_params_get_info(const clap_plugin_t*, uint32_t idx, clap_param_info_t* info) {
    if (!info || idx >= 7) return false;
    std::memset(info, 0, sizeof(*info));
    info->id = idx;
    info->flags = (1 << 3); // CLAP_PARAM_IS_AUTOMATABLE
    const char* names[7] = {
        "Master Bypass", "Remote Plugin", "Latency Profile",
        "Input Gain", "Wet/Dry Mix", "Drive / Sat", "Output Trim"
    };
    const double defs[7] = { 0.0, 0.0, 1.0 / 3.0, 0.5, 1.0, 0.0, 0.5 };
    std::snprintf(info->name, sizeof(info->name), "%s", names[idx]);
    info->min_value = 0.0;
    info->max_value = 1.0;
    info->default_value = defs[idx];
    return true;
}
static bool clap_params_get_value(const clap_plugin_t* plugin, clap_id param_id, double* out_value) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (!d || !d->rust || !out_value || param_id >= 7) return false;
    *out_value = og_native_get_param(d->rust, param_id);
    return true;
}
static bool clap_params_value_to_text(const clap_plugin_t*, clap_id param_id, double value, char* out_buf, uint32_t cap) {
    if (!out_buf || cap == 0) return false;
    switch (param_id) {
        case 0: std::snprintf(out_buf, cap, "%s", value >= 0.5 ? "Bypassed" : "Active"); return true;
        case 1: std::snprintf(out_buf, cap, "%d", (int)(value * 128.0 + 0.5)); return true;
        case 2: std::snprintf(out_buf, cap, "Profile %d", (int)(value * 3.0 + 0.5)); return true;
        case 3:
        case 6: std::snprintf(out_buf, cap, "%+.1f dB", (value - 0.5) * 48.0); return true;
        case 4: std::snprintf(out_buf, cap, "%.0f %%", value * 100.0); return true;
        case 5: std::snprintf(out_buf, cap, "%.1f", value * 10.0); return true;
        default: return false;
    }
}
static bool clap_params_text_to_value(const clap_plugin_t*, clap_id, const char* txt, double* out_val) {
    if (!txt || !out_val) return false;
    *out_val = std::clamp(std::atof(txt), 0.0, 1.0);
    return true;
}
static void clap_params_flush(const clap_plugin_t* plugin, const clap_input_events_t* in, const clap_output_events_t*) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (d && d->rust) applyClapEvents(d->rust, in);
}
static const clap_plugin_params_t kClapParams = {
    clap_params_count,
    clap_params_get_info,
    clap_params_get_value,
    clap_params_value_to_text,
    clap_params_text_to_value,
    clap_params_flush
};

// State (State v7)
static bool clap_state_save(const clap_plugin_t* plugin, const clap_ostream_t* stream) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (!d || !d->rust || !stream || !stream->write) return false;
    uint32_t len = og_native_get_state(d->rust, nullptr, 0);
    if (len == 0) return false;
    std::vector<uint8_t> buf(len);
    uint32_t written = og_native_get_state(d->rust, buf.data(), len);
    return stream->write(stream, buf.data(), written) == static_cast<int64_t>(written);
}
static bool clap_state_load(const clap_plugin_t* plugin, const clap_istream_t* stream) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (!d || !d->rust || !stream || !stream->read) return false;
    std::vector<uint8_t> buf;
    uint8_t chunk[4096];
    while (true) {
        int64_t n = stream->read(stream, chunk, sizeof(chunk));
        if (n <= 0) break;
        buf.insert(buf.end(), chunk, chunk + n);
    }
    if (buf.empty()) return false;
    return og_native_set_state(d->rust, buf.data(), static_cast<uint32_t>(buf.size()));
}
static const clap_plugin_state_t kClapState = { clap_state_save, clap_state_load };

// GUI (Cocoa NSView 720x420)
static bool clap_gui_is_api_supported(const clap_plugin_t*, const char* api, bool is_floating) {
    return !is_floating && api && std::strcmp(api, "cocoa") == 0;
}
static bool clap_gui_get_preferred_api(const clap_plugin_t*, const char** api, bool* is_floating) {
    if (api) *api = "cocoa";
    if (is_floating) *is_floating = false;
    return true;
}
static bool clap_gui_create(const clap_plugin_t* plugin, const char* api, bool is_floating) {
    if (!clap_gui_is_api_supported(plugin, api, is_floating)) return false;
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (!d) return false;
    if (!d->view) {
        d->view = [[OpenGridderCLAPEditorView alloc] initWithFrame:NSMakeRect(0, 0, 720, 420)];
        d->view.rust = d->rust;
    }
    return true;
}
static void clap_gui_destroy(const clap_plugin_t* plugin) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (d && d->view) {
        [d->view removeFromSuperview];
        d->view = nil;
    }
}
static bool clap_gui_set_scale(const clap_plugin_t*, double) { return true; }
static bool clap_gui_get_size(const clap_plugin_t* plugin, uint32_t* w, uint32_t* h) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    NSSize sz = (d && d->view) ? [d->view preferredViewSize] : NSMakeSize(864, 420);
    if (w) *w = static_cast<uint32_t>(sz.width);
    if (h) *h = static_cast<uint32_t>(sz.height);
    return true;
}
static bool clap_gui_can_resize(const clap_plugin_t*) { return true; }
static bool clap_gui_get_resize_hints(const clap_plugin_t*, clap_gui_resize_hints_t* hints) {
    if (!hints) return false;
    hints->can_resize_horizontally = true;
    hints->can_resize_vertically = false;
    hints->preserve_aspect_ratio = false;
    hints->aspect_ratio_width = 0;
    hints->aspect_ratio_height = 0;
    return true;
}
static bool clap_gui_adjust_size(const clap_plugin_t* plugin, uint32_t* w, uint32_t* h) {
    return clap_gui_get_size(plugin, w, h);
}
static bool clap_gui_set_size(const clap_plugin_t*, uint32_t, uint32_t) { return true; }
static bool clap_gui_set_parent(const clap_plugin_t* plugin, const clap_window_t* window) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (!d || !d->view || !window || !window->ptr) return false;
    NSView* parent = (__bridge NSView*)window->ptr;
    d->view.rust = d->rust;
    [parent addSubview:d->view];
    [d->view setNeedsDisplay:YES];
    return true;
}
static bool clap_gui_set_transient(const clap_plugin_t*, const clap_window_t*) { return false; }
static void clap_gui_suggest_title(const clap_plugin_t*, const char*) {}
static bool clap_gui_show(const clap_plugin_t* plugin) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (d && d->view) [d->view setHidden:NO];
    return true;
}
static bool clap_gui_hide(const clap_plugin_t* plugin) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (d && d->view) [d->view setHidden:YES];
    return true;
}
static const clap_plugin_gui_t kClapGui = {
    clap_gui_is_api_supported,
    clap_gui_get_preferred_api,
    clap_gui_create,
    clap_gui_destroy,
    clap_gui_set_scale,
    clap_gui_get_size,
    clap_gui_can_resize,
    clap_gui_get_resize_hints,
    clap_gui_adjust_size,
    clap_gui_set_size,
    clap_gui_set_parent,
    clap_gui_set_transient,
    clap_gui_suggest_title,
    clap_gui_show,
    clap_gui_hide
};

// Plugin Lifecycle
static bool og_clap_init(const clap_plugin_t* plugin) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (!d) return false;
    if (!d->rust) {
        uint32_t lat = og_native_default_latency();
        d->rust = og_native_create(d->sampleRate, 8192, 2, lat);
        if (d->rust) og_native_accept_latency(d->rust, lat);
    }
    return d->rust != nullptr;
}
static void og_clap_destroy(const clap_plugin_t* plugin) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (d) {
        if (d->view) {
            [d->view removeFromSuperview];
            d->view = nil;
        }
        if (d->rust) {
            og_native_destroy(d->rust);
            d->rust = nullptr;
        }
        delete d;
    }
}
static bool og_clap_activate(const clap_plugin_t* plugin, double sample_rate, uint32_t, uint32_t) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (!d) return false;
    d->sampleRate = sample_rate;
    if (!d->rust) {
        uint32_t lat = og_native_default_latency();
        d->rust = og_native_create(sample_rate, 8192, 2, lat);
        if (d->rust) og_native_accept_latency(d->rust, lat);
    }
    return d->rust != nullptr;
}
static void og_clap_deactivate(const clap_plugin_t*) {}
static bool og_clap_start_processing(const clap_plugin_t*) { return true; }
static void og_clap_stop_processing(const clap_plugin_t*) {}
static void og_clap_reset(const clap_plugin_t*) {}

static clap_process_status og_clap_process(const clap_plugin_t* plugin, const clap_process_t* proc) {
    auto* d = static_cast<OpenGridderCLAPData*>(plugin->plugin_data);
    if (!d || !d->rust || !proc) return CLAP_PROCESS_ERROR;

    applyClapEvents(d->rust, proc->in_events);
    if (proc->frames_count == 0 || proc->audio_inputs_count == 0 || proc->audio_outputs_count == 0) {
        return CLAP_PROCESS_CONTINUE;
    }

    uint32_t lat = og_native_latency(d->rust);
    og_native_accept_latency(d->rust, lat);

    float* inL = proc->audio_inputs[0].data32[0];
    float* inR = proc->audio_inputs[0].channel_count > 1 ? proc->audio_inputs[0].data32[1] : inL;
    float* outL = proc->audio_outputs[0].data32[0];
    float* outR = proc->audio_outputs[0].channel_count > 1 ? proc->audio_outputs[0].data32[1] : outL;
    const float* inputs[2] = { inL, inR };
    float* outputs[2] = { outL, outR };
    og_native_process(d->rust, inputs, outputs, proc->frames_count);
    return CLAP_PROCESS_CONTINUE;
}

static const void* og_clap_get_extension(const clap_plugin_t*, const char* id) {
    if (!id) return nullptr;
    if (std::strcmp(id, "clap.audio-ports") == 0) return &kClapAudioPorts;
    if (std::strcmp(id, "clap.note-ports") == 0) return &kClapNotePorts;
    if (std::strcmp(id, "clap.latency") == 0) return &kClapLatency;
    if (std::strcmp(id, "clap.params") == 0) return &kClapParams;
    if (std::strcmp(id, "clap.state") == 0) return &kClapState;
    if (std::strcmp(id, "clap.gui") == 0) return &kClapGui;
    return nullptr;
}
static void og_clap_on_main_thread(const clap_plugin_t*) {}

// Factory
static uint32_t factory_get_plugin_count(const clap_plugin_factory_t*) { return 1; }
static const clap_plugin_descriptor_t* factory_get_plugin_descriptor(const clap_plugin_factory_t*, uint32_t index) {
    return (index == 0) ? &kOpenGridderClapDesc : nullptr;
}
static const clap_plugin_t* factory_create_plugin(const clap_plugin_factory_t*, const clap_host_t* host, const char* plugin_id) {
    if (!plugin_id || std::strcmp(plugin_id, kOpenGridderClapDesc.id) != 0) return nullptr;
    auto* d = new OpenGridderCLAPData();
    d->host = host;
    d->rust = nullptr;
    d->view = nil;
    d->sampleRate = 48000.0;
    d->plugin.desc = &kOpenGridderClapDesc;
    d->plugin.plugin_data = d;
    d->plugin.init = og_clap_init;
    d->plugin.destroy = og_clap_destroy;
    d->plugin.activate = og_clap_activate;
    d->plugin.deactivate = og_clap_deactivate;
    d->plugin.start_processing = og_clap_start_processing;
    d->plugin.stop_processing = og_clap_stop_processing;
    d->plugin.reset = og_clap_reset;
    d->plugin.process = og_clap_process;
    d->plugin.get_extension = og_clap_get_extension;
    d->plugin.on_main_thread = og_clap_on_main_thread;
    return &d->plugin;
}
static const clap_plugin_factory_t kClapFactory = {
    factory_get_plugin_count,
    factory_get_plugin_descriptor,
    factory_create_plugin
};

static bool entry_init(const char*) { return true; }
static void entry_deinit(void) {}
static const void* entry_get_factory(const char* factory_id) {
    if (factory_id && std::strcmp(factory_id, "clap.plugin-factory") == 0) {
        return &kClapFactory;
    }
    return nullptr;
}

__attribute__((visibility("default")))
extern const clap_plugin_entry_t clap_entry = {
    CLAP_VERSION,
    entry_init,
    entry_deinit,
    entry_get_factory
};

} // extern "C"
