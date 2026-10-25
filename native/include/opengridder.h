#pragma once
#include <stdint.h>
#include <stdbool.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct NativePlugin NativePlugin;
uint32_t og_native_default_latency(void);
NativePlugin* og_native_create(double rate, uint32_t max_frames, uint32_t channels, uint32_t latency);
void og_native_destroy(NativePlugin*);
int32_t og_native_process(NativePlugin*, const float* const* inputs, float* const* outputs, uint32_t frames);
void og_native_select(NativePlugin*, uint32_t index);
void og_native_bypass(NativePlugin*, bool bypass);
uint32_t og_native_latency(NativePlugin*);
void og_native_accept_latency(NativePlugin*, uint32_t total);
void og_native_set_transport_latency(NativePlugin*, uint32_t transport);
int32_t og_native_status(NativePlugin*);
void og_native_name(NativePlugin*, uint32_t index, char* buffer, uint32_t capacity);

// --- Extended Dynamic Slot & Plugin Management API (AudioGridder Parity) ---
uint32_t og_native_num_loaded_plugins(NativePlugin*);
bool og_native_get_loaded_plugin(NativePlugin*, uint32_t idx, char* name_buf, uint32_t name_cap, bool* bypassed, uint32_t* oversampling);
uint32_t og_native_num_available_plugins(NativePlugin*);
bool og_native_get_available_plugin(NativePlugin*, uint32_t idx, char* name_buf, uint32_t name_cap, char* id_buf, uint32_t id_cap, char* cat_buf, uint32_t cat_cap);
bool og_native_load_plugin(NativePlugin*, uint32_t available_idx);
bool og_native_unload_plugin(NativePlugin*, uint32_t loaded_idx);
bool og_native_set_plugin_oversampling(NativePlugin*, uint32_t loaded_idx, uint32_t factor);
bool og_native_set_plugin_bypassed(NativePlugin*, uint32_t loaded_idx, bool bypassed);
bool og_native_edit_plugin(NativePlugin*, uint32_t loaded_idx, int32_t x, int32_t y);
bool og_native_hide_plugin(NativePlugin*);
#ifdef __cplusplus
}
#endif
