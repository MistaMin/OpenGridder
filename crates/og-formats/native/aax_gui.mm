#import <Cocoa/Cocoa.h>
#include "aax_gui.h"
#include "aax_defs.h"
#include "bridge.h"
#include "editor.h"

#include "AAX_CEffectParameters.h"  // cDefaultMasterBypassID
#include "AAX_IEffectParameters.h"
#include "AAX_IViewContainer.h"

namespace {
void EditThunk(void* owner, uint32_t index, double value) {
    if (owner) static_cast<OpenGridder_GUI*>(owner)->Edit(index, value);
}
void GestureThunk(void* owner, uint32_t index, bool begin) {
    if (owner) static_cast<OpenGridder_GUI*>(owner)->Gesture(index, begin);
}
}  // namespace

AAX_IEffectGUI* AAX_CALLBACK OpenGridder_GUI::Create() { return new OpenGridder_GUI(); }
AAX_IEffectGUI* AAX_CALLBACK OpenGridder_CreateGUI() { return OpenGridder_GUI::Create(); }

OpenGridder_GUI::OpenGridder_GUI() : AAX_CEffectGUI() {}

OpenGridder_GUI::~OpenGridder_GUI() {
    // Tear down while this is still an OpenGridder_GUI (the base destructor would call the
    // pure virtual DeleteViewContainer through Uninitialize).
    if (GetViewContainer()) SetViewContainer(nullptr);
    DeleteViewContainer();
}

void* OpenGridder_GUI::Processor() {
    AAX_IEffectParameters* params = GetEffectParameters();
    if (!params) return nullptr;
    void* processor = nullptr;
    uint32_t written = 0;
    if (params->GetCustomData(cOpenGridder_ProcessorDataID, sizeof processor, &processor, &written) != AAX_SUCCESS ||
        written != sizeof processor)
        return nullptr;
    return processor;
}

std::string OpenGridder_GUI::IdForIndex(uint32_t index) const {
    if (index == kOpenGridder_BypassIndex) return cDefaultMasterBypassID;
    char id[64] = {};
    if (!ag_insert_param_id(index, id, sizeof id) || !id[0]) snprintf(id, sizeof id, "p%u", index);
    return id;
}

void OpenGridder_GUI::CreateViewContents() {
    // Build the editor early so GetViewSize reports its real size before the host opens a window.
    if (mEditor) return;
    if (void* processor = Processor()) mEditor = ag_editor_create(processor, this, EditThunk, GestureThunk);
    if (mEditor) {
        NSRect frame = [(__bridge NSView*)mEditor frame];
        mWidth = (float)frame.size.width;
        mHeight = (float)frame.size.height;
    }
}

void OpenGridder_GUI::CreateViewContainer() {
    if (GetViewContainerType() != AAX_eViewContainer_Type_NSView) return;
    NSView* parent = (__bridge NSView*)GetViewContainerPtr();
    if (!parent) return;
    CreateViewContents();
    if (mEditor) ag_editor_attach(mEditor, (__bridge void*)parent);
}

void OpenGridder_GUI::DeleteViewContainer() {
    if (mEditor) {
        ag_editor_release(mEditor);  // detaches, stops callbacks, drops its processor reference
        mEditor = nullptr;
    }
}

AAX_Result OpenGridder_GUI::GetViewSize(AAX_Point* oViewSize) const {
    if (!oViewSize) return AAX_ERROR_NULL_ARGUMENT;
    oViewSize->horz = mWidth;
    oViewSize->vert = mHeight;
    return AAX_SUCCESS;
}

AAX_Result OpenGridder_GUI::TimerWakeup() {
    // The editor resizes itself (e.g. when the plugin browser opens); tell the host window.
    if (mEditor) {
        NSRect frame = [(__bridge NSView*)mEditor frame];
        const float w = (float)frame.size.width, h = (float)frame.size.height;
        if (w > 0 && h > 0 && (w != mWidth || h != mHeight)) {
            mWidth = w;
            mHeight = h;
            if (AAX_IViewContainer* container = GetViewContainer()) {
                AAX_Point size(h, w);
                container->SetViewSize(size);
            }
        }
    }
    return AAX_SUCCESS;
}

AAX_Result OpenGridder_GUI::ParameterUpdated(AAX_CParamID) {
    // The editor redraws from ag_insert_revision; the data model already pushed the value.
    return AAX_SUCCESS;
}

void OpenGridder_GUI::Edit(uint32_t index, double normalized) {
    AAX_IEffectParameters* params = GetEffectParameters();
    if (!params) return;
    const std::string id = IdForIndex(index);
    params->SetParameterNormalizedValue(id.c_str(), normalized);
}

void OpenGridder_GUI::Gesture(uint32_t index, bool begin) {
    AAX_IEffectParameters* params = GetEffectParameters();
    if (!params) return;
    const std::string id = IdForIndex(index);
    if (begin)
        params->TouchParameter(id.c_str());
    else
        params->ReleaseParameter(id.c_str());
}
