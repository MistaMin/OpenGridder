#pragma once
#include <cstdint>
// Callbacks use normalized host values. The owner keeps the processor alive.
using AgEdit = void(*)(void*,uint32_t,double);
using AgGesture = void(*)(void*,uint32_t,bool);
extern "C" void* ag_editor_create(void* processor,void* owner,AgEdit edit,AgGesture gesture);
extern "C" void ag_editor_release(void* view);
extern "C" uint64_t ag_insert_revision(void*);
extern "C" bool ag_insert_render(void*,uint32_t*,uint32_t,uint32_t);

extern "C" bool ag_editor_attach(void* view,void* parent);
// The editor's current size (its panels collapse and expand, and the meter bay doubles in VU
// mode), and a callback for when it changes, so a host window can follow (VST3 resizeView).
using AgResize = void(*)(void* ctx,int width,int height);
extern "C" void ag_editor_size(void* view,int* width,int* height);
extern "C" void ag_editor_default_size(int* width,int* height);
extern "C" void ag_editor_set_resize(void* view,AgResize callback,void* ctx);
