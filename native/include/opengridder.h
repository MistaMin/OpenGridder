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

// --- Screen Streaming & Input Forwarding API ---
bool og_native_get_screen_info(NativePlugin*, uint32_t* width, uint32_t* height, double* scale, uint64_t* frame_id);
bool og_native_copy_screen_frame(NativePlugin*, uint8_t* dest, uint32_t dest_cap, uint64_t* frame_id);
void og_native_send_mouse(NativePlugin*, uint8_t ev_type, float x, float y, bool shift, bool ctrl, bool alt, float delta_x, float delta_y, bool is_smooth);
void og_native_send_key(NativePlugin*, const uint16_t* keys, uint32_t count);
bool og_native_is_editor_open(NativePlugin*);

// --- Local Console DSP Parameters & Real-Time Peak Meters (Phase 1) ---
// param_id: 0=Bypass, 1=Selection, 2=LatencyProfile, 3=InputGain, 4=WetDryMix, 5=Drive, 6=OutputGain, 7..22=AutomationSlots 0..15, 23=DriveCharacter (0 Clean, 0.5 Tb, 1 Tr), 24=DriveQuality (0 ADAA, 0.5 2x, 1 4x)
void og_native_set_param(NativePlugin*, uint32_t param_id, float norm_val);
float og_native_get_param(NativePlugin*, uint32_t param_id);
void og_native_get_meters(NativePlugin*, float* left, float* right);
void og_native_get_input_meters(NativePlugin*, float* left, float* right);

// --- Chain Reordering, Presets, Remote Parameters, A/B & Server Management (Phase 2) ---
bool og_native_exchange_plugins(NativePlugin*, uint32_t idx_a, uint32_t idx_b);
uint32_t og_native_num_presets(NativePlugin*, uint32_t loaded_idx);
bool og_native_get_preset_name(NativePlugin*, uint32_t loaded_idx, uint32_t preset_idx, char* buf, uint32_t cap);
bool og_native_set_preset(NativePlugin*, uint32_t loaded_idx, uint32_t preset_idx);
uint32_t og_native_num_plugin_params(NativePlugin*, uint32_t loaded_idx);
bool og_native_get_plugin_param_info(NativePlugin*, uint32_t loaded_idx, uint32_t param_idx, char* name_buf, uint32_t name_cap, float* default_val, float* current_val);
bool og_native_set_plugin_param(NativePlugin*, uint32_t loaded_idx, uint32_t param_idx, float val);
void og_native_store_a(NativePlugin*);
void og_native_store_b(NativePlugin*);
void og_native_toggle_ab(NativePlugin*);
bool og_native_is_b(NativePlugin*);
void og_native_get_active_server(NativePlugin*, char* buf, uint32_t cap);
bool og_native_set_server(NativePlugin*, const char* host_port);
uint32_t og_native_num_discovered_servers(NativePlugin*);
bool og_native_get_discovered_server(NativePlugin*, uint32_t idx, char* buf, uint32_t cap);

// --- Full State Serialization (State Version 7) (Phase 3) ---
uint32_t og_native_get_state(NativePlugin*, uint8_t* buf, uint32_t cap);
bool og_native_set_state(NativePlugin*, const uint8_t* data, uint32_t len);

// --- MIDI Over Network & 64-bit Float Audio Processing (Phase 4) ---
void og_native_queue_midi(NativePlugin*, int32_t sample_offset, uint8_t status, uint8_t data1, uint8_t data2);
int32_t og_native_process_f64(NativePlugin*, const double* const* inputs, double* const* outputs, uint32_t frames);
#ifdef __cplusplus
}
#endif
