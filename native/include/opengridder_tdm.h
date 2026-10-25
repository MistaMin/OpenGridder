#pragma once
#include <stdint.h>
#include <stdbool.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct TdmClient TdmClient;
/* Non-realtime, single worker ownership. Original SDK-neutral mailbox ABI. */
TdmClient* og_tdm_create(double rate, uint32_t epoch, uint32_t selection, bool bypass);
void og_tdm_destroy(TdmClient*);
uint32_t og_tdm_next_capture(TdmClient*);
int32_t og_tdm_status(TdmClient*);
/* Two distinct arrays of 132 words. Returns 1 packet, 0 prefill, negative fault. */
int32_t og_tdm_capture(TdmClient*, const uint32_t* capture, uint32_t* returned);
#ifdef __cplusplus
}
#endif
