// OpenGridder's C ABI adapter for the external Steinberg VST3 SDK. Zero-overhead pure-native.
// Audio transport, catalog, buffer ownership and host policy are implemented in Rust.
#include <CoreFoundation/CoreFoundation.h>
#include "pluginterfaces/base/ipluginbase.h"
#include "pluginterfaces/base/ibstream.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/ivstmessage.h"
#include "pluginterfaces/gui/iplugview.h"
#include "pluginterfaces/vst/ivstprocesscontext.h"
#include "public.sdk/source/vst/hosting/hostclasses.h"
#include "public.sdk/source/vst/hosting/parameterchanges.h"
#include <algorithm>
#include <cmath>
#include <cstring>
#include <memory>
#include <stdexcept>
#include <vector>
using namespace Steinberg;
using namespace Steinberg::Vst;
// The SDK sources we compile do not define this IID.
DEF_CLASS_IID(Steinberg::IPlugFrame)

namespace {
void check(tresult status, const char* operation) { if (status != kResultOk) throw std::runtime_error(std::string(operation) + " (VST3 result " + std::to_string(status) + ")"); }
void error_text(char* target, const char* source) { if (target) { std::strncpy(target, source, 511); target[511] = 0; } }
class Host : public HostApplication {
    tresult PLUGIN_API getName(String128 name) override {
        std::fill(name, name + 128, 0);
        const char* text = "OpenGridder";
        for (int i = 0; text[i]; ++i) name[i] = text[i];
        return kResultOk;
    }
};
struct Module {
    CFBundleRef bundle = nullptr;
    IPluginFactory* factory = nullptr;
    bool entered = false;
    ~Module() {
        if (factory) factory->release();
        if (entered) { auto exit = reinterpret_cast<bool(*)()>(CFBundleGetFunctionPointerForName(bundle, CFSTR("bundleExit"))); if (exit) exit(); }
        if (bundle) CFRelease(bundle);
    }
    void open(const char* path) {
        auto url = CFURLCreateFromFileSystemRepresentation(nullptr, reinterpret_cast<const UInt8*>(path), std::strlen(path), true);
        if (!url) throw std::runtime_error("Invalid VST3 bundle path");
        bundle = CFBundleCreate(nullptr, url); CFRelease(url);
        if (!bundle || !CFBundleLoadExecutable(bundle)) throw std::runtime_error("Could not load VST3 module for this architecture");
        auto entry = reinterpret_cast<bool(*)(CFBundleRef)>(CFBundleGetFunctionPointerForName(bundle, CFSTR("bundleEntry")));
        auto getFactory = reinterpret_cast<IPluginFactory*(*)()>(CFBundleGetFunctionPointerForName(bundle, CFSTR("GetPluginFactory")));
        if (!entry || !getFactory) throw std::runtime_error("Missing VST3 module entry points");
        if (!entry(bundle)) throw std::runtime_error("VST3 bundleEntry failed");
        entered = true;
        factory = getFactory();
        if (!factory) throw std::runtime_error("VST3 returned a null factory");
    }
};
// Bounded state stream. Owned by the calling thread; plugins must not retain it.
class Stream : public IBStream {
public:
    std::vector<uint8> bytes;
    int64 cursor = 0;
    tresult PLUGIN_API queryInterface(const TUID id, void** obj) override {
        if (!obj) return kInvalidArgument;
        *obj = nullptr;
        if (FUnknownPrivate::iidEqual(id, IBStream::iid) || FUnknownPrivate::iidEqual(id, FUnknown::iid)) { *obj = static_cast<IBStream*>(this); return kResultOk; }
        return kNoInterface;
    }
    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }
    tresult PLUGIN_API read(void* data, int32 count, int32* read) override {
        if (read) *read = 0;
        if (count < 0 || (count && !data)) return kInvalidArgument;
        auto available = cursor < static_cast<int64>(bytes.size()) ? static_cast<int64>(bytes.size()) - cursor : 0;
        auto n = static_cast<int32>(std::min<int64>(count, available));
        if (n) std::memcpy(data, bytes.data() + cursor, n);
        cursor += n; if (read) *read = n;
        // IBStream reports a successful short read at EOF; numBytesRead carries the count.
        return kResultOk;
    }
    tresult PLUGIN_API write(void* data, int32 count, int32* written) override {
        if (written) *written = 0;
        if (count < 0 || (count && !data) || cursor + count > 64 * 1024 * 1024) return kInvalidArgument;
        if (cursor + count > static_cast<int64>(bytes.size())) bytes.resize(cursor + count);
        if (count) std::memcpy(bytes.data() + cursor, data, count);
        cursor += count; if (written) *written = count; return kResultOk;
    }
    tresult PLUGIN_API seek(int64 position, int32 mode, int64* result) override {
        if (mode < 0 || mode > 2) return kInvalidArgument;
        const int64 base = mode == 0 ? 0 : mode == 1 ? cursor : static_cast<int64>(bytes.size());
        if (position < -base || position > 64 * 1024 * 1024 - base) return kInvalidArgument;
        cursor = base + position; if (result) *result = cursor; return kResultOk;
    }
    tresult PLUGIN_API tell(int64* position) override { if (!position) return kInvalidArgument; *position = cursor; return kResultOk; }
};
// Host side of IPlugFrame: forwards plugin-initiated editor resizes to the window owner.
class Frame : public IPlugFrame {
public:
    void (*resize)(void*, int, int) = nullptr;
    void* user = nullptr;
    tresult PLUGIN_API queryInterface(const TUID id, void** obj) override {
        if (!obj) return kInvalidArgument;
        *obj = nullptr;
        if (FUnknownPrivate::iidEqual(id, IPlugFrame::iid) || FUnknownPrivate::iidEqual(id, FUnknown::iid)) { *obj = static_cast<IPlugFrame*>(this); return kResultOk; }
        return kNoInterface;
    }
    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }
    tresult PLUGIN_API resizeView(IPlugView* view, ViewRect* rect) override {
        if (!view || !rect) return kInvalidArgument;
        if (resize) resize(user, rect->getWidth(), rect->getHeight());
        view->onSize(rect);
        return kResultOk;
    }
};
struct Instance {
    Module module;
    Host host;
    IComponent* component = nullptr;
    IAudioProcessor* processor = nullptr;
    IEditController* controller = nullptr;
    IConnectionPoint* componentConnection = nullptr;
    IConnectionPoint* controllerConnection = nullptr;
    IPlugView* view = nullptr;
    Frame frame;
    bool initialized = false, separateController = false, active = false, processing = false;
    ParameterChanges changes;
    std::vector<ParameterInfo> parameters;
    std::vector<AudioBusBuffers> inputs, outputs;
    std::vector<std::vector<float>> silentInputs;
    std::vector<std::vector<float*>> inputPointers;
    int channels = 0, maxFrames = 0;
    double rate = 48000;
    void closeView() {
        if (!view) return;
        view->removed(); view->setFrame(nullptr); view->release(); view = nullptr;
    }
    ~Instance() {
        closeView();
        stop();
        if (componentConnection && controllerConnection) { componentConnection->disconnect(controllerConnection); controllerConnection->disconnect(componentConnection); }
        if (componentConnection) componentConnection->release();
        if (controllerConnection) controllerConnection->release();
        if (controller) { if (separateController) controller->terminate(); controller->release(); }
        if (processor) processor->release();
        if (component) { if (initialized) component->terminate(); component->release(); }
    }
    void stop() {
        if (processing) { processor->setProcessing(false); processing = false; }
        if (active) { component->setActive(false); active = false; }
    }
    void open(const char* path, const uint8* cid) {
        module.open(path);
        check(module.factory->createInstance(reinterpret_cast<const char*>(cid), IComponent::iid, reinterpret_cast<void**>(&component)), "Could not create VST3 component");
        if (!component) throw std::runtime_error("Null VST3 component");
        TUID controllerId{};
        const auto controllerStatus = component->getControllerClassId(controllerId);
        check(component->initialize(&host), "VST3 component initialize failed"); initialized = true;
        check(component->queryInterface(IAudioProcessor::iid, reinterpret_cast<void**>(&processor)), "VST3 component has no audio processor");
        if (component->queryInterface(IEditController::iid, reinterpret_cast<void**>(&controller)) != kResultOk && controllerStatus == kResultOk) {
            check(module.factory->createInstance(controllerId, IEditController::iid, reinterpret_cast<void**>(&controller)), "Could not create VST3 controller");
            if (!controller) throw std::runtime_error("Null VST3 controller");
            check(controller->initialize(&host), "VST3 controller initialize failed"); separateController = true;
        }
        if (controller) {
            component->queryInterface(IConnectionPoint::iid, reinterpret_cast<void**>(&componentConnection));
            controller->queryInterface(IConnectionPoint::iid, reinterpret_cast<void**>(&controllerConnection));
            if (componentConnection && controllerConnection) { componentConnection->connect(controllerConnection); controllerConnection->connect(componentConnection); }
            Stream state;
            if (component->getState(&state) == kResultOk) { state.cursor = 0; controller->setComponentState(&state); }
            auto count = controller->getParameterCount();
            if (count < 0 || count > 65536) throw std::runtime_error("Invalid VST3 parameter count");
            for (int index = 0; index < count; ++index) { ParameterInfo info{}; if (controller->getParameterInfo(index, info) == kResultOk) parameters.push_back(info); }
            changes.setMaxParameters(static_cast<int32>(parameters.size()));
        }
    }
    void prepare(double sampleRate, int frames, int channelCount) {
        if (processing && rate == sampleRate && maxFrames == frames && channels == channelCount) return;
        stop();
        if (channelCount < 1 || channelCount > 2 || frames < 1 || frames > 32768 || !std::isfinite(sampleRate) || sampleRate < 8000 || sampleRate > 768000) throw std::runtime_error("Unsupported VST3 sample rate, block size or channel layout");
        check(processor->canProcessSampleSize(kSample32), "VST3 plugin does not support float32 processing");
        const auto inputCount = component->getBusCount(kAudio, kInput);
        const auto outputCount = component->getBusCount(kAudio, kOutput);
        if (inputCount < 1 || inputCount > 16 || outputCount != 1) throw std::runtime_error("VST3 host currently requires one main audio output bus and 1–16 input buses");
        std::vector<SpeakerArrangement> in(inputCount, channelCount == 1 ? SpeakerArr::kMono : SpeakerArr::kStereo);
        SpeakerArrangement out = in.front();
        // Aux buses retain the plugin's preferred layout and receive silence.
        for (int bus = 1; bus < inputCount; ++bus) check(processor->getBusArrangement(kInput, bus, in[bus]), "Could not query VST3 auxiliary bus");
        check(processor->setBusArrangements(in.data(), inputCount, &out, 1), "VST3 plugin rejected mono/stereo bus arrangement");
        inputs.assign(inputCount, {}); outputs.assign(1, {});
        silentInputs.clear(); inputPointers.clear(); inputPointers.resize(inputCount);
        for (int bus = 0; bus < inputCount; ++bus) {
            BusInfo info{}; check(component->getBusInfo(kAudio, kInput, bus, info), "Could not query VST3 input bus");
            if (info.channelCount < 1 || info.channelCount > 32 || (bus == 0 && info.channelCount != channelCount)) throw std::runtime_error("VST3 input channel mismatch");
            inputs[bus].numChannels = info.channelCount;
            inputPointers[bus].resize(info.channelCount);
            if (bus > 0) for (int ch = 0; ch < info.channelCount; ++ch) { silentInputs.emplace_back(frames, 0.f); inputPointers[bus][ch] = silentInputs.back().data(); }
            check(component->activateBus(kAudio, kInput, bus, bus == 0), "Could not activate VST3 input bus");
        }
        BusInfo outputInfo{}; check(component->getBusInfo(kAudio, kOutput, 0, outputInfo), "Could not query VST3 output bus");
        if (outputInfo.channelCount != channelCount) throw std::runtime_error("VST3 output channel mismatch");
        check(component->activateBus(kAudio, kOutput, 0, true), "Could not activate VST3 output bus");
        ProcessSetup setup{kRealtime, kSample32, frames, sampleRate};
        check(processor->setupProcessing(setup), "VST3 setupProcessing failed");
        check(component->setActive(true), "VST3 setActive failed"); active = true;
        const auto processingStatus = processor->setProcessing(true);
        // Some older VST3 implementations leave this notification unimplemented.
        if (processingStatus != kNotImplemented) check(processingStatus, "VST3 setProcessing failed");
        processing = true;
        rate = sampleRate; maxFrames = frames; channels = channelCount;
    }
};
}
extern "C" {
struct OgVstClass { uint8 cid[16]; char name[64], vendor[64], version[64], category[128]; };
struct OgVstParameter { uint32 id; int32 steps, flags; double defaultValue, currentValue; uint16 name[128], units[128]; };
// Enumeration uses factory class IDs, never path hashes or invented gain parameters.
int og_vst3_scan(const char* path, OgVstClass* result, int capacity, char* error) noexcept {
    try {
        Module module; module.open(path);
        PFactoryInfo factoryInfo{}; module.factory->getFactoryInfo(&factoryInfo);
        IPluginFactory2* factory2 = nullptr; module.factory->queryInterface(IPluginFactory2::iid, reinterpret_cast<void**>(&factory2));
        int found = 0;
        const auto count = module.factory->countClasses();
        if (count < 0 || count > 65536) { if (factory2) factory2->release(); throw std::runtime_error("Invalid VST3 class count"); }
        for (int index = 0; index < count && found < capacity; ++index) {
            PClassInfo basic{};
            if (module.factory->getClassInfo(index, &basic) != kResultOk || std::strcmp(basic.category, kVstAudioEffectClass)) continue;
            auto& info = result[found++]; info = {};
            std::memcpy(info.cid, basic.cid, 16); std::strncpy(info.name, basic.name, 63); std::strncpy(info.vendor, factoryInfo.vendor, 63);
            if (factory2) { PClassInfo2 extended{}; if (factory2->getClassInfo2(index, &extended) == kResultOk) { if (extended.vendor[0]) std::strncpy(info.vendor, extended.vendor, 63); std::strncpy(info.version, extended.version, 63); std::strncpy(info.category, extended.subCategories, 127); } }
        }
        if (factory2) factory2->release();
        return found;
    } catch (const std::exception& ex) { error_text(error, ex.what()); return -1; } catch (...) { error_text(error, "VST3 scan threw an unknown exception"); return -1; }
}
void* og_vst3_create(const char* path, const uint8* cid, char* error) noexcept {
    try { auto instance = std::make_unique<Instance>(); instance->open(path, cid); return instance.release(); }
    catch (const std::exception& ex) { error_text(error, ex.what()); return nullptr; } catch (...) { error_text(error, "VST3 initialization threw an unknown exception"); return nullptr; }
}
void og_vst3_destroy(void* handle) noexcept { try { delete static_cast<Instance*>(handle); } catch (...) {} }
int og_vst3_prepare(void* handle, double rate, int frames, int channels, char* error) noexcept {
    try { static_cast<Instance*>(handle)->prepare(rate, frames, channels); return 0; }
    catch (const std::exception& ex) { error_text(error, ex.what()); return -1; } catch (...) { error_text(error, "VST3 prepare threw an unknown exception"); return -1; }
}
void og_vst3_stop(void* handle) noexcept { try { static_cast<Instance*>(handle)->stop(); } catch (...) {} }
int og_vst3_parameter_count(void* handle) noexcept { return static_cast<int>(static_cast<Instance*>(handle)->parameters.size()); }
int og_vst3_parameter(void* handle, int index, OgVstParameter* result) noexcept {
    try {
        auto& p = *static_cast<Instance*>(handle);
        if (index < 0 || index >= static_cast<int>(p.parameters.size())) return -1;
        auto& info = p.parameters[index]; *result = {};
        result->id = info.id; result->steps = info.stepCount; result->flags = info.flags;
        result->defaultValue = info.defaultNormalizedValue; result->currentValue = p.controller->getParamNormalized(info.id);
        std::memcpy(result->name, info.title, sizeof(info.title)); std::memcpy(result->units, info.units, sizeof(info.units)); return 0;
    } catch (...) { return -1; }
}
double og_vst3_get_parameter(void* handle, int index) noexcept {
    try { auto& p = *static_cast<Instance*>(handle); return p.controller->getParamNormalized(p.parameters.at(index).id); } catch (...) { return 0; }
}
int og_vst3_set_parameter(void* handle, int index, double value) noexcept {
    try {
        auto& p = *static_cast<Instance*>(handle); auto& info = p.parameters.at(index);
        if (!std::isfinite(value) || (info.flags & ParameterInfo::kIsReadOnly)) return -1;
        value = std::clamp(value, 0., 1.); check(p.controller->setParamNormalized(info.id, value), "VST3 parameter rejected");
        int32 queueIndex = 0, pointIndex = 0;
        auto queue = p.changes.addParameterData(info.id, queueIndex);
        if (!queue) return -1;
        // Coalesce pending changes to one value per parameter, applied at block start.
        static_cast<ParameterValueQueue*>(queue)->clear();
        return queue->addPoint(0, value, pointIndex) == kResultOk ? 0 : -1;
    } catch (...) { return -1; }
}
int og_vst3_process(void* handle, float** input, float** output, int frames, int64 position, double ppq, double bpm, int numerator, int denominator, uint32 transport) noexcept {
    try {
        auto& p = *static_cast<Instance*>(handle);
        if (!p.processing || frames < 1 || frames > p.maxFrames) return -1;
        for (int ch = 0; ch < p.channels; ++ch) p.inputPointers[0][ch] = input[ch];
        for (size_t bus = 0; bus < p.inputs.size(); ++bus) { p.inputs[bus].channelBuffers32 = p.inputPointers[bus].data(); p.inputs[bus].silenceFlags = bus == 0 ? 0 : (uint64(1) << p.inputs[bus].numChannels) - 1; }
        p.outputs[0].numChannels = p.channels; p.outputs[0].silenceFlags = 0; p.outputs[0].channelBuffers32 = output;
        ProcessContext context{};
        context.sampleRate = p.rate; context.projectTimeSamples = position;
        context.projectTimeMusic = ppq; context.tempo = bpm; context.timeSigNumerator = numerator; context.timeSigDenominator = denominator;
        context.state = transport | ProcessContext::kProjectTimeMusicValid | ProcessContext::kTempoValid | ProcessContext::kTimeSigValid;
        ProcessData data{};
        data.processMode = kRealtime; data.symbolicSampleSize = kSample32; data.numSamples = frames;
        data.numInputs = static_cast<int32>(p.inputs.size()); data.numOutputs = 1;
        data.inputs = p.inputs.data(); data.outputs = p.outputs.data(); data.processContext = &context; data.inputParameterChanges = &p.changes;
        auto status = p.processor->process(data); p.changes.clearQueue();
        if (status != kResultOk) return -1;
        for (int ch = 0; ch < p.channels; ++ch) if (p.outputs[0].silenceFlags & (uint64(1) << ch)) std::fill(output[ch], output[ch] + frames, 0.f);
        return 0;
    } catch (...) { return -1; }
}
uint32 og_vst3_latency(void* handle) noexcept { try { return static_cast<Instance*>(handle)->processor->getLatencySamples(); } catch (...) { return 0; } }
int og_vst3_get_state(void* handle, uint8** bytes, size_t* length) noexcept {
    try {
        auto& p = *static_cast<Instance*>(handle); Stream stream;
        check(p.component->getState(&stream), "VST3 getState failed");
        *length = stream.bytes.size(); *bytes = static_cast<uint8*>(std::malloc(*length));
        if (*length && !*bytes) return -1;
        if (*length) std::memcpy(*bytes, stream.bytes.data(), *length); return 0;
    } catch (...) { return -1; }
}
void og_vst3_free_state(uint8* bytes) noexcept { std::free(bytes); }
int og_vst3_set_state(void* handle, const uint8* bytes, size_t length) noexcept {
    try {
        if (length > 64 * 1024 * 1024) return -1;
        auto& p = *static_cast<Instance*>(handle); Stream stream; stream.bytes.assign(bytes, bytes + length);
        check(p.component->setState(&stream), "VST3 setState failed");
        if (p.controller) { stream.cursor = 0; check(p.controller->setComponentState(&stream), "VST3 controller state sync failed"); }
        p.changes.clearQueue(); return 0;
    } catch (...) { return -1; }
}
// Editor hosting. All view calls must run on the main thread; the window owner guarantees that.
int og_vst3_view_open(void* handle, int* width, int* height, void (*resize)(void*, int, int), void* user) noexcept {
    try {
        auto& p = *static_cast<Instance*>(handle);
        if (!p.controller) return -1;
        if (p.view) { ViewRect r{}; if (p.view->getSize(&r) == kResultOk) { *width = r.getWidth(); *height = r.getHeight(); } return 0; }
        p.view = p.controller->createView(ViewType::kEditor);
        if (!p.view) return -1;
        if (p.view->isPlatformTypeSupported(kPlatformTypeNSView) != kResultTrue) { p.view->release(); p.view = nullptr; return -1; }
        p.frame.resize = resize; p.frame.user = user;
        p.view->setFrame(&p.frame);
        ViewRect r{};
        if (p.view->getSize(&r) != kResultOk) r = ViewRect(0, 0, 640, 360);
        *width = r.getWidth(); *height = r.getHeight();
        return 0;
    } catch (...) { return -1; }
}
int og_vst3_view_attach(void* handle, void* nsview) noexcept {
    try { auto& p = *static_cast<Instance*>(handle); return p.view && p.view->attached(nsview, kPlatformTypeNSView) == kResultOk ? 0 : -1; } catch (...) { return -1; }
}
void og_vst3_view_close(void* handle) noexcept { try { static_cast<Instance*>(handle)->closeView(); } catch (...) {} }
}
