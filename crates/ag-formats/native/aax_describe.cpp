// AAX description: one effect with mono and stereo Native components. The AAX library's
// ACF exports (AAX_Exports.o in libAAXLibrary_libcpp.a) call GetEffectDescriptions.
#include "aax_defs.h"
#include "aax_parameters.h"

#include "AAX_ICollection.h"
#include "AAX_IComponentDescriptor.h"
#include "AAX_IEffectDescriptor.h"
#include "AAX_IEffectGUI.h"
#include "AAX_IPropertyMap.h"
#include "AAX_Errors.h"
#include "AAX_Exception.h"

// aax_gui.mm
AAX_IEffectGUI* AAX_CALLBACK OpenGridder_CreateGUI();

namespace {

AAX_Result DescribeComponent(AAX_IEffectDescriptor* effect, AAX_EStemFormat stem, AAX_CTypeID plugInID) {
    AAX_CheckedResult err;
    AAX_IComponentDescriptor* const comp = effect->NewComponentDescriptor();
    if (!comp) return AAX_ERROR_NULL_OBJECT;
    err = comp->AddAudioIn(eAlgFieldID_AudioIn);
    err = comp->AddAudioOut(eAlgFieldID_AudioOut);
    err = comp->AddAudioBufferLength(eAlgFieldID_BufferSize);
    err = comp->AddPrivateData(eAlgFieldID_Private, sizeof(SOpenGridder_Private), AAX_ePrivateDataOptions_DefaultOptions);

    AAX_IPropertyMap* const props = comp->NewPropertyMap();
    if (!props) return AAX_ERROR_NULL_OBJECT;
    err = props->AddProperty(AAX_eProperty_ManufacturerID, cOpenGridder_ManufacturerID);
    err = props->AddProperty(AAX_eProperty_ProductID, cOpenGridder_ProductID);
    err = props->AddProperty(AAX_eProperty_PlugInID_Native, plugInID);
    err = props->AddProperty(AAX_eProperty_InputStemFormat, stem);
    err = props->AddProperty(AAX_eProperty_OutputStemFormat, stem);
    err = props->AddProperty(AAX_eProperty_CanBypass, true);
    err = props->AddProperty(AAX_eProperty_UsesClientGUI, false);
    // The server keeps streaming while the track is silent, so never let the host skip calls.
    err = props->AddProperty(AAX_eProperty_Constraint_AlwaysProcess, true);
    err = comp->AddProcessProc_Native(OpenGridder_AlgorithmProcessFunction, props);
    err = effect->AddComponent(comp);
    return err;
}

AAX_Result DescribeEffect(AAX_IEffectDescriptor* effect) {
    AAX_CheckedResult err;
    err = effect->AddName("OpenGridder");
    err = effect->AddName("OpnGridder");
    err = effect->AddName("OpnGrd");
    err = effect->AddName("OGrd");
    err = effect->AddName("OG");
    err = effect->AddCategory(AAX_EPlugInCategory_Effect);
    err = DescribeComponent(effect, AAX_eStemFormat_Stereo, cOpenGridder_PlugInID_Native);
    err = DescribeComponent(effect, AAX_eStemFormat_Mono, cOpenGridder_PlugInID_NativeMono);
    err = effect->AddProcPtr(reinterpret_cast<void*>(OpenGridder_Parameters::Create), kAAX_ProcPtrID_Create_EffectParameters);
    err = effect->AddProcPtr(reinterpret_cast<void*>(OpenGridder_CreateGUI), kAAX_ProcPtrID_Create_EffectGUI);
    // Contents/Resources/OpenGridderPages.xml: control-surface page tables.
    err = effect->AddResourceInfo(AAX_eResourceType_PageTable, "OpenGridderPages.xml");
    return err;
}

}  // namespace

AAX_Result GetEffectDescriptions(AAX_ICollection* outCollection) {
    if (!outCollection) return AAX_ERROR_NULL_OBJECT;
    try {
        AAX_CheckedResult err;
        AAX_IEffectDescriptor* const effect = outCollection->NewDescriptor();
        if (!effect) return AAX_ERROR_NULL_OBJECT;
        err = DescribeEffect(effect);
        err = outCollection->AddEffect(kOpenGridder_EffectID, effect);
        err = outCollection->SetManufacturerName("OpenGrid");
        err = outCollection->AddPackageName("OpenGridder");
        err = outCollection->AddPackageName("OpnGridder");
        err = outCollection->AddPackageName("OGrd");
        err = outCollection->SetPackageVersion(1);
        return err;
    } catch (const AAX::Exception::ResultError& e) {
        return e.Result();
    } catch (...) {
        return AAX_ERROR_UNKNOWN_EXCEPTION;
    }
}
