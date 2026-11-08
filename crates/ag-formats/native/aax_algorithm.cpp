// AAX Native process callback. Each instance's private data names the data model that owns
// the ag_insert processor; the processor handles bypass itself (latency-aligned dry path),
// so audio is only copied straight through when there is no processor or it rejects a block.
#include "aax_defs.h"
#include "aax_parameters.h"

#include <cstring>

void AAX_CALLBACK OpenGridder_AlgorithmProcessFunction(SOpenGridder_Context* const inInstancesBegin[],
                                                       const void* inInstancesEnd) {
    for (SOpenGridder_Context* const* walk = inInstancesBegin; walk < inInstancesEnd; ++walk) {
        SOpenGridder_Context* const instance = *walk;
        if (!instance || !instance->mBufferSize) continue;
        const int32_t frames = *instance->mBufferSize;
        OpenGridder_Parameters* owner = instance->mPrivate ? instance->mPrivate->mOwner : nullptr;
        if (owner) {
            owner->Process(instance->mInputPP, instance->mOutputPP, frames);
            continue;
        }
        // No data model yet: pass audio through.
        const int32_t channels = instance->mPrivate ? instance->mPrivate->mChannels : 0;
        for (int32_t ch = 0; ch < channels && ch < 2 && frames > 0; ++ch) {
            float* const in = instance->mInputPP ? instance->mInputPP[ch] : nullptr;
            float* const out = instance->mOutputPP ? instance->mOutputPP[ch] : nullptr;
            if (in && out && in != out) std::memcpy(out, in, sizeof(float) * (size_t)frames);
        }
    }
}
