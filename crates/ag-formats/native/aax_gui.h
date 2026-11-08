#pragma once
// AAX Cocoa GUI: embeds the existing native editor (editor.h, ag_editor_*) in the NSView
// container Pro Tools provides. Edits and gestures go back through the AAX data model so the
// host records automation. No JUCE.
#include "AAX_CEffectGUI.h"
#include <string>

class OpenGridder_GUI : public AAX_CEffectGUI {
public:
    static AAX_IEffectGUI* AAX_CALLBACK Create();
    OpenGridder_GUI();
    ~OpenGridder_GUI() AAX_OVERRIDE;

    AAX_Result GetViewSize(AAX_Point* oViewSize) const AAX_OVERRIDE;
    AAX_Result TimerWakeup() AAX_OVERRIDE;
    AAX_Result ParameterUpdated(AAX_CParamID iParameterID) AAX_OVERRIDE;

    // Editor callbacks (normalized values, ag parameter index).
    void Edit(uint32_t index, double normalized);
    void Gesture(uint32_t index, bool begin);

protected:
    void CreateViewContents() AAX_OVERRIDE;
    void CreateViewContainer() AAX_OVERRIDE;
    void DeleteViewContainer() AAX_OVERRIDE;

private:
    void* Processor();
    std::string IdForIndex(uint32_t index) const;

    void* mEditor = nullptr;  // AGOpenBridgeView, owned via ag_editor_create/release
    float mWidth = 864.0f;
    float mHeight = 420.0f;
};
