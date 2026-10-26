// OpenGridder LV2 Plugin Adapter (Zero-Overhead C ABI)
#include "opengridder.h"
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <algorithm>

extern "C" {

typedef void* LV2_Handle;

typedef struct _LV2_Feature {
    const char* URI;
    void* data;
} LV2_Feature;

typedef struct _LV2_Descriptor {
    const char* URI;
    LV2_Handle (*instantiate)(const struct _LV2_Descriptor* descriptor,
                              double sample_rate,
                              const char* bundle_path,
                              const LV2_Feature* const* features);
    void (*connect_port)(LV2_Handle instance, uint32_t port, void* data_location);
    void (*activate)(LV2_Handle instance);
    void (*run)(LV2_Handle instance, uint32_t sample_count);
    void (*deactivate)(LV2_Handle instance);
    void (*cleanup)(LV2_Handle instance);
    const void* (*extension_data)(const char* uri);
} LV2_Descriptor;

enum OpenGridderLV2Ports {
    PORT_IN_L = 0,
    PORT_IN_R = 1,
    PORT_OUT_L = 2,
    PORT_OUT_R = 3,
    PORT_BYPASS = 4,
    PORT_REMOTE_PLUGIN = 5,
    PORT_LATENCY_PROFILE = 6,
    PORT_INPUT_GAIN = 7,
    PORT_WET_DRY_MIX = 8,
    PORT_DRIVE = 9,
    PORT_OUTPUT_GAIN = 10,
    PORT_LATENCY_OUT = 11
};

struct OpenGridderLV2Instance {
    NativePlugin* rust;
    const float* in_l;
    const float* in_r;
    float* out_l;
    float* out_r;
    const float* bypass;
    const float* remote_plugin;
    const float* latency_profile;
    const float* input_gain;
    const float* wet_dry_mix;
    const float* drive;
    const float* output_gain;
    float* latency_out;
};

static LV2_Handle og_lv2_instantiate(const LV2_Descriptor* /*descriptor*/,
                                     double sample_rate,
                                     const char* /*bundle_path*/,
                                     const LV2_Feature* const* /*features*/) {
    auto* inst = static_cast<OpenGridderLV2Instance*>(std::calloc(1, sizeof(OpenGridderLV2Instance)));
    if (!inst) return nullptr;
    uint32_t lat = og_native_default_latency();
    inst->rust = og_native_create(sample_rate, 8192, 2, lat);
    if (!inst->rust) {
        std::free(inst);
        return nullptr;
    }
    og_native_accept_latency(inst->rust, lat);
    return inst;
}

static void og_lv2_connect_port(LV2_Handle instance, uint32_t port, void* data_location) {
    auto* inst = static_cast<OpenGridderLV2Instance*>(instance);
    if (!inst) return;
    switch (port) {
        case PORT_IN_L: inst->in_l = static_cast<const float*>(data_location); break;
        case PORT_IN_R: inst->in_r = static_cast<const float*>(data_location); break;
        case PORT_OUT_L: inst->out_l = static_cast<float*>(data_location); break;
        case PORT_OUT_R: inst->out_r = static_cast<float*>(data_location); break;
        case PORT_BYPASS: inst->bypass = static_cast<const float*>(data_location); break;
        case PORT_REMOTE_PLUGIN: inst->remote_plugin = static_cast<const float*>(data_location); break;
        case PORT_LATENCY_PROFILE: inst->latency_profile = static_cast<const float*>(data_location); break;
        case PORT_INPUT_GAIN: inst->input_gain = static_cast<const float*>(data_location); break;
        case PORT_WET_DRY_MIX: inst->wet_dry_mix = static_cast<const float*>(data_location); break;
        case PORT_DRIVE: inst->drive = static_cast<const float*>(data_location); break;
        case PORT_OUTPUT_GAIN: inst->output_gain = static_cast<const float*>(data_location); break;
        case PORT_LATENCY_OUT: inst->latency_out = static_cast<float*>(data_location); break;
        default: break;
    }
}

static void og_lv2_activate(LV2_Handle /*instance*/) {}

static void og_lv2_run(LV2_Handle instance, uint32_t sample_count) {
    auto* inst = static_cast<OpenGridderLV2Instance*>(instance);
    if (!inst || !inst->rust || sample_count == 0) return;

    if (inst->bypass) og_native_set_param(inst->rust, 0, *inst->bypass >= 0.5f ? 1.0f : 0.0f);
    if (inst->remote_plugin) og_native_set_param(inst->rust, 1, std::clamp(*inst->remote_plugin / 128.0f, 0.0f, 1.0f));
    if (inst->latency_profile) og_native_set_param(inst->rust, 2, std::clamp(*inst->latency_profile / 3.0f, 0.0f, 1.0f));
    if (inst->input_gain) og_native_set_param(inst->rust, 3, std::clamp((*inst->input_gain + 24.0f) / 48.0f, 0.0f, 1.0f));
    if (inst->wet_dry_mix) og_native_set_param(inst->rust, 4, std::clamp(*inst->wet_dry_mix / 100.0f, 0.0f, 1.0f));
    if (inst->drive) og_native_set_param(inst->rust, 5, std::clamp(*inst->drive / 10.0f, 0.0f, 1.0f));
    if (inst->output_gain) og_native_set_param(inst->rust, 6, std::clamp((*inst->output_gain + 24.0f) / 48.0f, 0.0f, 1.0f));

    uint32_t lat = og_native_latency(inst->rust);
    og_native_accept_latency(inst->rust, lat);
    if (inst->latency_out) *inst->latency_out = static_cast<float>(lat);

    if (!inst->in_l || !inst->out_l) return;
    const float* inputs[2] = { inst->in_l, inst->in_r ? inst->in_r : inst->in_l };
    float* outputs[2] = { inst->out_l, inst->out_r ? inst->out_r : inst->out_l };
    og_native_process(inst->rust, inputs, outputs, sample_count);
}

static void og_lv2_deactivate(LV2_Handle /*instance*/) {}

static void og_lv2_cleanup(LV2_Handle instance) {
    auto* inst = static_cast<OpenGridderLV2Instance*>(instance);
    if (inst) {
        if (inst->rust) og_native_destroy(inst->rust);
        std::free(inst);
    }
}

static const void* og_lv2_extension_data(const char* /*uri*/) {
    return nullptr;
}

static const LV2_Descriptor kOpenGridderLV2Descriptor = {
    "https://opengrid.studio/plugins/opengridder",
    og_lv2_instantiate,
    og_lv2_connect_port,
    og_lv2_activate,
    og_lv2_run,
    og_lv2_deactivate,
    og_lv2_cleanup,
    og_lv2_extension_data
};

__attribute__((visibility("default")))
const LV2_Descriptor* lv2_descriptor(uint32_t index) {
    return (index == 0) ? &kOpenGridderLV2Descriptor : nullptr;
}

} // extern "C"
