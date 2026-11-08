#pragma once
#include <cstdint>
extern "C" {
void* ag_insert_create();
void ag_insert_destroy(void*);
bool ag_insert_prepare(void*, double, uint32_t, uint32_t);
void ag_insert_reset(void*);
uint32_t ag_insert_param_count();
bool ag_insert_param_info(uint32_t,char*,double*,double*,double*,int32_t*);
double ag_insert_get(void*,uint32_t);
bool ag_insert_set(void*,uint32_t,double);
uint32_t ag_insert_latency(void*);
bool ag_insert_process(void*,const float*const*,float*const*,uint32_t,uint32_t,bool);
bool ag_insert_process64(void*,const double*const*,double*const*,uint32_t,uint32_t,bool);
uint32_t ag_insert_save(void*,uint8_t*,uint32_t);
bool ag_insert_load(void*,const uint8_t*,uint32_t);

int32_t ag_insert_status(void*);
uint32_t ag_insert_num_available_plugins(void*);
bool ag_insert_get_available_plugin_ext(void*, uint32_t, char*, uint32_t, char*, uint32_t, char*, uint32_t, char*, uint32_t, char*, uint32_t);
bool ag_insert_load_plugin(void*, uint32_t);
uint32_t ag_insert_num_loaded_plugins(void*);
bool ag_insert_get_loaded_plugin(void*, uint32_t, char*, uint32_t, bool*, uint32_t*);
bool ag_insert_unload_plugin(void*, uint32_t);
bool ag_insert_set_plugin_bypassed(void*, uint32_t, bool);
bool ag_insert_set_plugin_oversampling(void*, uint32_t, uint32_t);
bool ag_insert_exchange_plugins(void*, uint32_t, uint32_t);
bool ag_insert_edit_plugin(void*, uint32_t);
bool ag_insert_hide_plugin(void*);
uint32_t ag_insert_num_presets(void*, uint32_t);
bool ag_insert_get_preset_name(void*, uint32_t, uint32_t, char*, uint32_t);
bool ag_insert_set_preset(void*, uint32_t, uint32_t);
uint32_t ag_insert_num_plugin_params(void*, uint32_t);
bool ag_insert_get_plugin_param_info(void*, uint32_t, uint32_t, char*, uint32_t, float*, float*);
bool ag_insert_set_plugin_param(void*, uint32_t, uint32_t, float);
bool ag_insert_connect(void*, const char*);
}

extern "C" void ag_insert_midi(void*,uint32_t,uint8_t,uint8_t,uint8_t);
extern "C" void ag_insert_position(void*,double,int32_t,int32_t,int64_t,double,double,bool,bool,bool);

extern "C" bool ag_insert_retain(void*);
extern "C" bool ag_insert_alive(void*);
extern "C" void ag_insert_release(void*);
