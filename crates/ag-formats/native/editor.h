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
