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
#ifdef __cplusplus
}
#endif
