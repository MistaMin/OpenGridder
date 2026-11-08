// LV2 (lilv) and VST3 (module factory) hosting. The VST3 headers stay on
// Developer 2; this file is the host, not a copy of the SDK.
//
// VST2 is not implemented here. The VST2 SDK header is absent on this machine.

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <mutex>
#include <limits>
#include <string>
#include <vector>

#include <dlfcn.h>

#include <lilv/lilv.h>
#include <lv2/atom/atom.h>
#include <lv2/atom/forge.h>
#include <lv2/buf-size/buf-size.h>
#include <lv2/options/options.h>
#include <lv2/patch/patch.h>
#include <lv2/urid/urid.h>

#include "pluginterfaces/base/funknown.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/ivsthostapplication.h"
#include "pluginterfaces/gui/iplugview.h"
#include <clap/ext/gui.h>

// ui_mac.mm: the editor window follows a plugin-initiated resize.
extern "C" void ag_ui_view_resized(void* nsview, int width, int height);

namespace {

constexpr int kAtomWords = 4096;

void copy_cstr(char* dest, size_t cap, const char* src) {
    if (cap == 0) {
        return;
    }
    if (src == nullptr) {
        dest[0] = 0;
        return;
    }
    std::snprintf(dest, cap, "%s", src);
}

bool skipped_name(const std::string& text) {
    std::string lower = text;
    for (char& ch : lower) {
        if (ch >= 'A' && ch <= 'Z') {
            ch = static_cast<char>(ch - 'A' + 'a');
        }
    }
    return lower.find("audiogridder") != std::string::npos || lower.find("opengridder") != std::string::npos;
}

std::string home_dir() {
    if (const char* home = std::getenv("HOME")) {
        return home;
    }
    return {};
}

float clampf(float value, float low, float high) {
    if (value < low) {
        return low;
    }
    if (value > high) {
        return high;
    }
    return value;
}

struct Catalog {
    char name[128];
    char company[128];
    char id[768];
    char format[16];
    int is_instrument;
};

// --- LV2 ------------------------------------------------------------------------

struct Lv2Param {
    LV2_URID urid = 0;
    std::string name;
    float minimum = 0.0f;
    float maximum = 1.0f;
    float current = 0.0f;
    bool touched = false;
};

struct Lv2Plugin {
    LilvWorld* world = nullptr;
    LilvInstance* instance = nullptr;
    LV2_URID_Map urid_map{};
    LV2_Feature map_feature{};
    LV2_Feature options_feature{};
    LV2_Feature bounded_feature{};
    const LV2_Feature* features[4] = {};
    LV2_Options_Option options[4] = {};
    int32_t min_block = 1;
    int32_t max_block = 256;
    int32_t sequence_size = 8192;
    std::vector<std::string> uris;
    LV2_Atom_Forge forge{};
    std::vector<uint64_t> atom_in;
    std::vector<uint64_t> atom_out;
    std::vector<float> controls;
    std::vector<int> control_ports;
    std::vector<int> audio_in;
    std::vector<int> audio_out;
    int atom_in_port = -1;
    int atom_out_port = -1;
    std::vector<std::vector<float>> scratch_in;
    std::vector<std::vector<float>> scratch_out;
    std::vector<float> silence;
    std::vector<Lv2Param> params;
    LV2_URID patch_set = 0;
    LV2_URID patch_property = 0;
    LV2_URID patch_value = 0;
    int latency = 0;

    ~Lv2Plugin() {
        if (instance != nullptr) {
            lilv_instance_deactivate(instance);
            lilv_instance_free(instance);
        }
        if (world != nullptr) {
            lilv_world_free(world);
        }
    }
};

LV2_URID map_uri(LV2_URID_Map_Handle handle, const char* uri) {
    auto* self = static_cast<Lv2Plugin*>(handle);
    if (uri == nullptr) {
        return 0;
    }
    for (size_t index = 0; index < self->uris.size(); ++index) {
        if (self->uris[index] == uri) {
            return static_cast<LV2_URID>(index + 1);
        }
    }
    self->uris.emplace_back(uri);
    return static_cast<LV2_URID>(self->uris.size());
}

float node_float(LilvWorld* world, const LilvNode* subject, const char* predicate, float fallback) {
    LilvNode* pred = lilv_new_uri(world, predicate);
    LilvNodes* nodes = lilv_world_find_nodes(world, subject, pred, nullptr);
    float value = fallback;
    if (nodes != nullptr) {
        const LilvNode* first = lilv_nodes_get_first(nodes);
        if (first != nullptr && lilv_node_is_float(first)) {
            value = lilv_node_as_float(first);
        } else if (first != nullptr && lilv_node_is_int(first)) {
            value = static_cast<float>(lilv_node_as_int(first));
        }
        lilv_nodes_free(nodes);
    }
    lilv_node_free(pred);
    return value;
}

std::string node_string(LilvWorld* world, const LilvNode* subject, const char* predicate) {
    LilvNode* pred = lilv_new_uri(world, predicate);
    LilvNodes* nodes = lilv_world_find_nodes(world, subject, pred, nullptr);
    std::string value;
    if (nodes != nullptr) {
        const LilvNode* first = lilv_nodes_get_first(nodes);
        if (first != nullptr && lilv_node_is_string(first)) {
            if (const char* text = lilv_node_as_string(first)) {
                value = text;
            }
        }
        lilv_nodes_free(nodes);
    }
    lilv_node_free(pred);
    return value;
}

void load_lv2_bundles(LilvWorld* world) {
    const std::string roots[] = {
        "/Library/Audio/Plug-Ins/LV2",
        home_dir().empty() ? std::string() : home_dir() + "/Library/Audio/Plug-Ins/LV2",
    };
    for (const std::string& root : roots) {
        if (root.empty() || !std::filesystem::is_directory(root)) {
            continue;
        }
        for (const auto& entry : std::filesystem::directory_iterator(root)) {
            if (!entry.is_directory()) {
                continue;
            }
            const std::string name = entry.path().filename().string();
            if (name.size() < 4 || name.substr(name.size() - 4) != ".lv2") {
                continue;
            }
            if (skipped_name(name)) {
                continue;
            }
            std::string path = entry.path().string();
            if (path.back() != '/') {
                path.push_back('/');
            }
            LilvNode* bundle = lilv_new_file_uri(world, nullptr, path.c_str());
            if (bundle != nullptr) {
                lilv_world_load_bundle(world, bundle);
                lilv_node_free(bundle);
            }
        }
    }
}

void collect_lv2(std::vector<Catalog>& out, int cap) {
    LilvWorld* world = lilv_world_new();
    if (world == nullptr) {
        return;
    }
    load_lv2_bundles(world);
    const LilvPlugins* all = lilv_world_get_all_plugins(world);
    LILV_FOREACH (plugins, iter, all) {
        if (static_cast<int>(out.size()) >= cap) {
            break;
        }
        const LilvPlugin* plugin = lilv_plugins_get(all, iter);
        const LilvNode* uri_node = lilv_plugin_get_uri(plugin);
        const char* uri = uri_node != nullptr ? lilv_node_as_uri(uri_node) : nullptr;
        if (uri == nullptr || skipped_name(uri)) {
            continue;
        }
        LilvNode* name_node = lilv_plugin_get_name(plugin);
        const char* name = name_node != nullptr ? lilv_node_as_string(name_node) : uri;
        if (skipped_name(name != nullptr ? name : "")) {
            lilv_node_free(name_node);
            continue;
        }
        bool instrument = false;
        if (const LilvPluginClass* cls = lilv_plugin_get_class(plugin)) {
            if (const LilvNode* class_uri = lilv_plugin_class_get_uri(cls)) {
                if (const char* text = lilv_node_as_uri(class_uri)) {
                    instrument = std::strstr(text, "Instrument") != nullptr;
                }
            }
        }
        Catalog item{};
        copy_cstr(item.name, sizeof item.name, name);
        LilvNode* author = lilv_plugin_get_author_name(plugin);
        copy_cstr(item.company, sizeof item.company, author != nullptr ? lilv_node_as_string(author) : "LV2");
        lilv_node_free(author);
        std::string id = std::string("lv2:") + uri;
        if (id.size() >= sizeof item.id) {
            lilv_node_free(name_node);
            continue;
        }
        copy_cstr(item.id, sizeof item.id, id.c_str());
        copy_cstr(item.format, sizeof item.format, "LV2");
        item.is_instrument = instrument ? 1 : 0;
        out.push_back(item);
        lilv_node_free(name_node);
    }
    lilv_world_free(world);
}

Lv2Plugin* open_lv2(const char* id, float sample_rate, int max_block, int channels) {
    if (id == nullptr || std::strncmp(id, "lv2:", 4) != 0) {
        return nullptr;
    }
    const char* uri = id + 4;
    auto* self = new Lv2Plugin();
    self->world = lilv_world_new();
    if (self->world == nullptr) {
        delete self;
        return nullptr;
    }
    load_lv2_bundles(self->world);
    LilvNode* uri_node = lilv_new_uri(self->world, uri);
    const LilvPlugins* all = lilv_world_get_all_plugins(self->world);
    const LilvPlugin* plugin = lilv_plugins_get_by_uri(all, uri_node);
    lilv_node_free(uri_node);
    if (plugin == nullptr) {
        delete self;
        return nullptr;
    }

    self->max_block = std::max(1, max_block);
    self->urid_map.handle = self;
    self->urid_map.map = map_uri;
    self->map_feature.URI = LV2_URID__map;
    self->map_feature.data = &self->urid_map;
    self->bounded_feature.URI = LV2_BUF_SIZE__boundedBlockLength;
    self->bounded_feature.data = nullptr;
    self->options[0].context = LV2_OPTIONS_INSTANCE;
    self->options[0].subject = 0;
    self->options[0].key = map_uri(self, LV2_BUF_SIZE__minBlockLength);
    self->options[0].size = sizeof(int32_t);
    self->options[0].type = map_uri(self, LV2_ATOM__Int);
    self->options[0].value = &self->min_block;
    self->options[1].context = LV2_OPTIONS_INSTANCE;
    self->options[1].subject = 0;
    self->options[1].key = map_uri(self, LV2_BUF_SIZE__maxBlockLength);
    self->options[1].size = sizeof(int32_t);
    self->options[1].type = map_uri(self, LV2_ATOM__Int);
    self->options[1].value = &self->max_block;
    self->options[2].context = LV2_OPTIONS_INSTANCE;
    self->options[2].subject = 0;
    self->options[2].key = map_uri(self, LV2_BUF_SIZE__sequenceSize);
    self->options[2].size = sizeof(int32_t);
    self->options[2].type = map_uri(self, LV2_ATOM__Int);
    self->options[2].value = &self->sequence_size;
    std::memset(&self->options[3], 0, sizeof self->options[3]);
    self->options_feature.URI = LV2_OPTIONS__options;
    self->options_feature.data = self->options;
    self->features[0] = &self->map_feature;
    self->features[1] = &self->bounded_feature;
    self->features[2] = &self->options_feature;
    self->features[3] = nullptr;

    self->instance = lilv_plugin_instantiate(plugin, static_cast<double>(sample_rate), self->features);
    if (self->instance == nullptr) {
        delete self;
        return nullptr;
    }

    const uint32_t ports = lilv_plugin_get_num_ports(plugin);
    std::vector<float> minimum(ports, 0.0f);
    std::vector<float> maximum(ports, 1.0f);
    std::vector<float> defaults(ports, 0.0f);
    lilv_plugin_get_port_ranges_float(plugin, minimum.data(), maximum.data(), defaults.data());
    LilvNode* audio_uri = lilv_new_uri(self->world, LILV_URI_AUDIO_PORT);
    LilvNode* control_uri = lilv_new_uri(self->world, LILV_URI_CONTROL_PORT);
    LilvNode* input_uri = lilv_new_uri(self->world, LILV_URI_INPUT_PORT);
    LilvNode* output_uri = lilv_new_uri(self->world, LILV_URI_OUTPUT_PORT);
    LilvNode* atom_uri = lilv_new_uri(self->world, LV2_ATOM__AtomPort);
    self->controls.assign(ports, 0.0f);
    const int use_channels = std::clamp(channels, 1, 2);
    self->scratch_in.assign(static_cast<size_t>(use_channels), std::vector<float>(static_cast<size_t>(self->max_block), 0.0f));
    self->scratch_out.assign(2, std::vector<float>(static_cast<size_t>(self->max_block), 0.0f));
    self->silence.assign(static_cast<size_t>(self->max_block), 0.0f);
    self->atom_in.assign(kAtomWords, 0);
    self->atom_out.assign(kAtomWords, 0);

    for (uint32_t index = 0; index < ports; ++index) {
        const LilvPort* port = lilv_plugin_get_port_by_index(plugin, index);
        const bool is_input = lilv_port_is_a(plugin, port, input_uri);
        const bool is_output = lilv_port_is_a(plugin, port, output_uri);
        const char* symbol = lilv_node_as_string(lilv_port_get_symbol(plugin, port));
        if (lilv_port_is_a(plugin, port, audio_uri) && is_input) {
            self->audio_in.push_back(static_cast<int>(index));
            float* buffer = self->audio_in.size() <= self->scratch_in.size()
                ? self->scratch_in[self->audio_in.size() - 1].data()
                : self->silence.data();
            lilv_instance_connect_port(self->instance, index, buffer);
        } else if (lilv_port_is_a(plugin, port, audio_uri) && is_output) {
            self->audio_out.push_back(static_cast<int>(index));
            float* buffer = self->audio_out.size() <= self->scratch_out.size()
                ? self->scratch_out[self->audio_out.size() - 1].data()
                : self->silence.data();
            lilv_instance_connect_port(self->instance, index, buffer);
        } else if (lilv_port_is_a(plugin, port, atom_uri) && is_input && self->atom_in_port < 0) {
            self->atom_in_port = static_cast<int>(index);
            lilv_instance_connect_port(self->instance, index, self->atom_in.data());
        } else if (lilv_port_is_a(plugin, port, atom_uri) && is_output && self->atom_out_port < 0) {
            self->atom_out_port = static_cast<int>(index);
            lilv_instance_connect_port(self->instance, index, self->atom_out.data());
        } else if (lilv_port_is_a(plugin, port, control_uri)) {
            float value = defaults[index];
            if (!std::isfinite(value)) {
                value = 0.0f;
            }
            if (symbol != nullptr && std::strcmp(symbol, "enabled") == 0) {
                value = 1.0f;
            }
            if (symbol != nullptr && std::strcmp(symbol, "freeWheeling") == 0) {
                value = 0.0f;
            }
            self->controls[index] = value;
            lilv_instance_connect_port(self->instance, index, &self->controls[index]);
            if (is_output && symbol != nullptr && std::strcmp(symbol, "latency") == 0) {
                self->control_ports.push_back(static_cast<int>(index));
            }
        }
    }
    lilv_node_free(audio_uri);
    lilv_node_free(control_uri);
    lilv_node_free(input_uri);
    lilv_node_free(output_uri);
    lilv_node_free(atom_uri);

    lv2_atom_forge_init(&self->forge, &self->urid_map);
    self->patch_set = map_uri(self, LV2_PATCH__Set);
    self->patch_property = map_uri(self, LV2_PATCH__property);
    self->patch_value = map_uri(self, LV2_PATCH__value);

    LilvNode* writable = lilv_new_uri(self->world, LV2_PATCH__writable);
    LilvNodes* params = lilv_plugin_get_value(plugin, writable);
    lilv_node_free(writable);
    if (params != nullptr) {
        LILV_FOREACH (nodes, iter, params) {
            const LilvNode* param = lilv_nodes_get(params, iter);
            if (param == nullptr || !lilv_node_is_uri(param)) {
                continue;
            }
            Lv2Param described;
            const char* param_uri = lilv_node_as_uri(param);
            described.urid = map_uri(self, param_uri);
            described.minimum = node_float(self->world, param, "http://lv2plug.in/ns/lv2core#minimum", 0.0f);
            described.maximum = node_float(self->world, param, "http://lv2plug.in/ns/lv2core#maximum", 1.0f);
            if (!(described.maximum > described.minimum)) {
                described.minimum = 0.0f;
                described.maximum = 1.0f;
            }
            described.current = node_float(self->world, param, "http://lv2plug.in/ns/lv2core#default", described.minimum);
            described.name = node_string(self->world, param, "http://www.w3.org/2000/01/rdf-schema#label");
            if (described.name.empty() && param_uri != nullptr) {
                described.name = param_uri;
            }
            self->params.push_back(std::move(described));
        }
        lilv_nodes_free(params);
    }

    lilv_instance_activate(self->instance);
    return self;
}

void forge_lv2_sets(Lv2Plugin* self) {
    const size_t bytes = self->atom_in.size() * sizeof(uint64_t);
    lv2_atom_forge_set_buffer(&self->forge, reinterpret_cast<uint8_t*>(self->atom_in.data()), bytes);
    LV2_Atom_Forge_Frame frame;
    lv2_atom_forge_sequence_head(&self->forge, &frame, 0);
    for (const Lv2Param& param : self->params) {
        if (!param.touched) {
            continue;
        }
        lv2_atom_forge_frame_time(&self->forge, 0);
        LV2_Atom_Forge_Frame object;
        lv2_atom_forge_object(&self->forge, &object, 0, self->patch_set);
        lv2_atom_forge_key(&self->forge, self->patch_property);
        lv2_atom_forge_urid(&self->forge, param.urid);
        lv2_atom_forge_key(&self->forge, self->patch_value);
        lv2_atom_forge_float(&self->forge, param.current);
        lv2_atom_forge_pop(&self->forge, &object);
    }
    lv2_atom_forge_pop(&self->forge, &frame);

    auto* sequence = reinterpret_cast<LV2_Atom_Sequence*>(self->atom_out.data());
    sequence->atom.type = self->forge.Sequence;
    sequence->atom.size = static_cast<uint32_t>(self->atom_out.size() * sizeof(uint64_t) - sizeof(LV2_Atom));
}

void process_lv2(Lv2Plugin* self, float** audio, int channels, int frames) {
    frames = std::max(0, std::min(frames, self->max_block));
    const int count = std::min(channels, static_cast<int>(self->scratch_in.size()));
    for (int channel = 0; channel < count; ++channel) {
        std::memcpy(self->scratch_in[static_cast<size_t>(channel)].data(), audio[channel], static_cast<size_t>(frames) * sizeof(float));
    }
    for (auto& channel : self->scratch_out) {
        std::fill(channel.begin(), channel.begin() + frames, 0.0f);
    }
    forge_lv2_sets(self);
    lilv_instance_run(self->instance, static_cast<uint32_t>(frames));
    const int outs = std::min(count, static_cast<int>(self->scratch_out.size()));
    for (int channel = 0; channel < outs; ++channel) {
        std::memcpy(audio[channel], self->scratch_out[static_cast<size_t>(channel)].data(), static_cast<size_t>(frames) * sizeof(float));
    }
    for (int port : self->control_ports) {
        if (port >= 0 && static_cast<size_t>(port) < self->controls.size()) {
            self->latency = static_cast<int>(self->controls[static_cast<size_t>(port)]);
        }
    }
}

// --- VST3 -----------------------------------------------------------------------

class HostApp : public Steinberg::Vst::IHostApplication, public Steinberg::Vst::IComponentHandler {
public:
    HostApp() { FUNKNOWN_CTOR }
    virtual ~HostApp() { FUNKNOWN_DTOR }
    DECLARE_FUNKNOWN_METHODS

    Steinberg::tresult PLUGIN_API getName(Steinberg::Vst::String128 name) override {
        static const char text[] = "AudioGridder";
        for (int index = 0; index < 128; ++index) {
            name[index] = text[index];
            if (text[index] == 0) {
                break;
            }
        }
        return Steinberg::kResultOk;
    }

    Steinberg::tresult PLUGIN_API createInstance(Steinberg::TUID, Steinberg::TUID, void** obj) override {
        if (obj != nullptr) {
            *obj = nullptr;
        }
        return Steinberg::kNotImplemented;
    }

    Steinberg::tresult PLUGIN_API beginEdit(Steinberg::Vst::ParamID) override { return Steinberg::kResultOk; }
    Steinberg::tresult PLUGIN_API performEdit(Steinberg::Vst::ParamID, Steinberg::Vst::ParamValue) override {
        return Steinberg::kResultOk;
    }
    Steinberg::tresult PLUGIN_API endEdit(Steinberg::Vst::ParamID) override { return Steinberg::kResultOk; }
    Steinberg::tresult PLUGIN_API restartComponent(Steinberg::int32) override { return Steinberg::kResultOk; }
};

Steinberg::uint32 PLUGIN_API HostApp::addRef() {
    return static_cast<Steinberg::uint32>(__atomic_add_fetch(&__funknownRefCount, 1, __ATOMIC_ACQ_REL));
}

Steinberg::uint32 PLUGIN_API HostApp::release() {
    const int next = __atomic_sub_fetch(&__funknownRefCount, 1, __ATOMIC_ACQ_REL);
    if (next == 0) {
        delete this;
        return 0;
    }
    return static_cast<Steinberg::uint32>(next);
}

Steinberg::tresult PLUGIN_API HostApp::queryInterface(const Steinberg::TUID iid, void** obj) {
    QUERY_INTERFACE(iid, obj, Steinberg::FUnknown::iid, Steinberg::Vst::IHostApplication)
    QUERY_INTERFACE(iid, obj, Steinberg::Vst::IHostApplication::iid, Steinberg::Vst::IHostApplication)
    QUERY_INTERFACE(iid, obj, Steinberg::Vst::IComponentHandler::iid, Steinberg::Vst::IComponentHandler)
    if (obj != nullptr) {
        *obj = nullptr;
    }
    return Steinberg::kNoInterface;
}

// IPlugFrame for one open VST3 editor: plugin-initiated resizes resize the window.
class PlugFrame : public Steinberg::IPlugFrame {
public:
    explicit PlugFrame(void* parent) : parent_(parent) { FUNKNOWN_CTOR }
    virtual ~PlugFrame() { FUNKNOWN_DTOR }
    DECLARE_FUNKNOWN_METHODS

    Steinberg::tresult PLUGIN_API resizeView(Steinberg::IPlugView* view, Steinberg::ViewRect* rect) override {
        if (view == nullptr || rect == nullptr) {
            return Steinberg::kInvalidArgument;
        }
        ag_ui_view_resized(parent_, rect->getWidth(), rect->getHeight());
        view->onSize(rect);
        return Steinberg::kResultOk;
    }

private:
    void* parent_;
};

Steinberg::uint32 PLUGIN_API PlugFrame::addRef() {
    return static_cast<Steinberg::uint32>(__atomic_add_fetch(&__funknownRefCount, 1, __ATOMIC_ACQ_REL));
}

Steinberg::uint32 PLUGIN_API PlugFrame::release() {
    const int next = __atomic_sub_fetch(&__funknownRefCount, 1, __ATOMIC_ACQ_REL);
    if (next == 0) {
        delete this;
        return 0;
    }
    return static_cast<Steinberg::uint32>(next);
}

Steinberg::tresult PLUGIN_API PlugFrame::queryInterface(const Steinberg::TUID iid, void** obj) {
    QUERY_INTERFACE(iid, obj, Steinberg::FUnknown::iid, Steinberg::IPlugFrame)
    QUERY_INTERFACE(iid, obj, Steinberg::IPlugFrame::iid, Steinberg::IPlugFrame)
    if (obj != nullptr) {
        *obj = nullptr;
    }
    return Steinberg::kNoInterface;
}

struct Vst3Param {
    Steinberg::Vst::ParamID id = 0;
    std::string name;
    double current = 0.0;
};

struct Vst3Plugin {
    void* library = nullptr;
    HostApp* host = nullptr;
    Steinberg::IPluginFactory* factory = nullptr;
    Steinberg::Vst::IComponent* component = nullptr;
    Steinberg::Vst::IAudioProcessor* processor = nullptr;
    Steinberg::Vst::IEditController* controller = nullptr;
    Steinberg::Vst::IEditControllerHostEditing* controller2 = nullptr;
    Steinberg::Vst::IConnectionPoint* component_point = nullptr;
    Steinberg::Vst::IConnectionPoint* controller_point = nullptr;
    bool separate_controller = false;
    Steinberg::IPlugView* view = nullptr;   // open editor, main thread only
    PlugFrame* frame = nullptr;
    int max_block = 256;
    int channels = 2;
    int latency = 0;
    std::vector<Vst3Param> params;
    std::vector<std::vector<float>> scratch_in;
    std::vector<std::vector<float>> scratch_out;

    void close_editor() {
        if (view != nullptr) {
            view->setFrame(nullptr);
            view->removed();
            view->release();
            view = nullptr;
        }
        if (frame != nullptr) {
            frame->release();
            frame = nullptr;
        }
    }

    ~Vst3Plugin() {
        close_editor();
        if (processor != nullptr) {
            processor->setProcessing(0);
        }
        if (component != nullptr) {
            component->setActive(0);
        }
        if (component_point != nullptr && controller_point != nullptr) {
            component_point->disconnect(controller_point);
            controller_point->disconnect(component_point);
        }
        if (component_point != nullptr) {
            component_point->release();
        }
        if (controller_point != nullptr) {
            controller_point->release();
        }
        if (controller2 != nullptr) {
            controller2->release();
        }
        if (controller != nullptr) {
            controller->setComponentHandler(nullptr);
            if (separate_controller) {
                controller->terminate();
            }
            controller->release();
        }
        if (processor != nullptr) {
            processor->release();
        }
        if (component != nullptr) {
            component->terminate();
            component->release();
        }
        if (factory != nullptr) {
            factory->release();
        }
        if (host != nullptr) {
            host->release();
        }
        using Exit = bool (*)();
        if (library != nullptr) {
            auto exit_module = reinterpret_cast<Exit>(dlsym(library, "ModuleExit"));
            if (exit_module == nullptr) {
                exit_module = reinterpret_cast<Exit>(dlsym(library, "bundleExit"));
            }
            if (exit_module != nullptr) {
                exit_module();
            }
            dlclose(library);
        }
    }
};

std::string vst3_binary(const std::filesystem::path& bundle) {
    const std::filesystem::path macos = bundle / "Contents" / "MacOS";
    if (!std::filesystem::is_directory(macos)) {
        return {};
    }
    for (const auto& entry : std::filesystem::directory_iterator(macos)) {
        if (entry.is_regular_file()) {
            return entry.path().string();
        }
    }
    return {};
}

void utf16_to_utf8(const Steinberg::Vst::String128 text, char* dest, size_t cap) {
    size_t written = 0;
    for (int index = 0; index < 128 && text[index] != 0 && written + 1 < cap; ++index) {
        const unsigned int code = text[index];
        if (code < 128) {
            dest[written++] = static_cast<char>(code);
        } else {
            dest[written++] = '?';
        }
    }
    if (cap > 0) {
        dest[written] = 0;
    }
}

struct LoadedFactory {
    void* library = nullptr;
    Steinberg::IPluginFactory* factory = nullptr;
};

LoadedFactory load_factory(const std::string& binary) {
    LoadedFactory loaded;
    loaded.library = dlopen(binary.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (loaded.library == nullptr) {
        loaded.library = dlopen(binary.c_str(), RTLD_NOW | RTLD_GLOBAL);
    }
    if (loaded.library == nullptr) {
        return loaded;
    }
    using Entry = bool (*)(void*);
    auto entry = reinterpret_cast<Entry>(dlsym(loaded.library, "ModuleEntry"));
    if (entry == nullptr) {
        entry = reinterpret_cast<Entry>(dlsym(loaded.library, "bundleEntry"));
    }
    if (entry != nullptr) {
        entry(nullptr);
    }
    using FactoryFn = Steinberg::IPluginFactory* (*)();
    auto get_factory = reinterpret_cast<FactoryFn>(dlsym(loaded.library, "GetPluginFactory"));
    if (get_factory == nullptr) {
        dlclose(loaded.library);
        loaded.library = nullptr;
        return loaded;
    }
    loaded.factory = get_factory();
    if (loaded.factory == nullptr) {
        dlclose(loaded.library);
        loaded.library = nullptr;
    }
    return loaded;
}

bool collect_vst3_bundle(const std::filesystem::path& bundle, std::vector<Catalog>& out) {
    if (skipped_name(bundle.filename().string())) {
        return false;
    }
    const std::string binary = vst3_binary(bundle);
    if (binary.empty()) {
        return false;
    }
    LoadedFactory loaded = load_factory(binary);
    if (loaded.factory == nullptr) {
        return false;
    }
    bool stored = false;
    Steinberg::IPluginFactory2* factory2 = nullptr;
    loaded.factory->queryInterface(Steinberg::IPluginFactory2::iid, reinterpret_cast<void**>(&factory2));
    const Steinberg::int32 count = loaded.factory->countClasses();
    for (Steinberg::int32 index = 0; index < count; ++index) {
        Steinberg::PClassInfo info{};
        if (loaded.factory->getClassInfo(index, &info) != Steinberg::kResultOk) {
            continue;
        }
        if (std::strcmp(info.category, kVstAudioEffectClass) != 0) {
            continue;
        }
        bool instrument = false;
        char vendor[64] = "VST3";
        if (factory2 != nullptr) {
            Steinberg::PClassInfo2 info2{};
            if (factory2->getClassInfo2(index, &info2) == Steinberg::kResultOk) {
                instrument = std::strstr(info2.subCategories, "Instrument") != nullptr;
                if (info2.vendor[0] != 0) {
                    copy_cstr(vendor, sizeof vendor, info2.vendor);
                }
            }
        }
        Catalog item{};
        copy_cstr(item.name, sizeof item.name, info.name);
        copy_cstr(item.company, sizeof item.company, vendor);
        const std::string id = std::string("vst3:") + bundle.string();
        if (id.size() >= sizeof item.id) {
            continue;
        }
        copy_cstr(item.id, sizeof item.id, id.c_str());
        copy_cstr(item.format, sizeof item.format, "VST3");
        item.is_instrument = instrument ? 1 : 0;
        out.push_back(item);
        stored = true;
        break;
    }
    if (factory2 != nullptr) {
        factory2->release();
    }
    loaded.factory->release();
    // Keep the module mapped. Some plug-ins crash inside dlclose.
    return stored;
}

void collect_vst3(std::vector<Catalog>& out, int cap) {
    const std::string root = home_dir().empty() ? std::string() : home_dir() + "/Library/Audio/Plug-Ins/VST3";
    if (root.empty() || !std::filesystem::is_directory(root)) {
        return;
    }
    const char* preferred[] = {"ADAA Tester.vst3", "Blue Channel.vst3", "Harmonic Compressor.vst3", "HybridEQ.vst3", "OpenPreamp.vst3"};
    for (const char* name : preferred) {
        if (static_cast<int>(out.size()) >= cap) {
            return;
        }
        const std::filesystem::path bundle = std::filesystem::path(root) / name;
        if (std::filesystem::is_directory(bundle)) {
            collect_vst3_bundle(bundle, out);
        }
    }
}

Vst3Plugin* open_vst3(const char* id, float sample_rate, int max_block, int channels) {
    if (id == nullptr || std::strncmp(id, "vst3:", 5) != 0) {
        return nullptr;
    }
    const std::filesystem::path bundle = id + 5;
    const std::string binary = vst3_binary(bundle);
    if (binary.empty()) {
        return nullptr;
    }
    auto* self = new Vst3Plugin();
    self->host = new HostApp();
    LoadedFactory loaded = load_factory(binary);
    self->library = loaded.library;
    self->factory = loaded.factory;
    if (self->factory == nullptr) {
        delete self;
        return nullptr;
    }
    Steinberg::FUnknown* host_unknown = static_cast<Steinberg::Vst::IHostApplication*>(self->host);
    Steinberg::IPluginFactory3* factory3 = nullptr;
    if (self->factory->queryInterface(Steinberg::IPluginFactory3::iid, reinterpret_cast<void**>(&factory3)) == Steinberg::kResultOk && factory3 != nullptr) {
        factory3->setHostContext(host_unknown);
        factory3->release();
    }

    Steinberg::TUID cid{};
    bool found = false;
    const Steinberg::int32 count = self->factory->countClasses();
    for (Steinberg::int32 index = 0; index < count; ++index) {
        Steinberg::PClassInfo info{};
        if (self->factory->getClassInfo(index, &info) != Steinberg::kResultOk) {
            continue;
        }
        if (std::strcmp(info.category, kVstAudioEffectClass) != 0) {
            continue;
        }
        std::memcpy(cid, info.cid, sizeof cid);
        found = true;
        break;
    }
    if (!found || self->factory->createInstance(cid, Steinberg::Vst::IComponent::iid, reinterpret_cast<void**>(&self->component)) != Steinberg::kResultOk || self->component == nullptr) {
        delete self;
        return nullptr;
    }
    if (self->component->initialize(host_unknown) != Steinberg::kResultOk) {
        delete self;
        return nullptr;
    }
    if (self->component->queryInterface(Steinberg::Vst::IAudioProcessor::iid, reinterpret_cast<void**>(&self->processor)) != Steinberg::kResultOk || self->processor == nullptr) {
        delete self;
        return nullptr;
    }
    if (self->component->queryInterface(Steinberg::Vst::IEditController::iid, reinterpret_cast<void**>(&self->controller)) != Steinberg::kResultOk) {
        self->controller = nullptr;
    }
    if (self->controller == nullptr) {
        Steinberg::TUID controller_id{};
        if (self->component->getControllerClassId(controller_id) == Steinberg::kResultOk) {
            if (self->factory->createInstance(controller_id, Steinberg::Vst::IEditController::iid, reinterpret_cast<void**>(&self->controller)) == Steinberg::kResultOk && self->controller != nullptr) {
                self->separate_controller = true;
                if (self->controller->initialize(host_unknown) != Steinberg::kResultOk) {
                    delete self;
                    return nullptr;
                }
            }
        }
    }
    if (self->controller != nullptr) {
        self->controller->setComponentHandler(static_cast<Steinberg::Vst::IComponentHandler*>(self->host));
        self->controller2 = nullptr;
        if (self->controller->queryInterface(Steinberg::Vst::IEditControllerHostEditing::iid, reinterpret_cast<void**>(&self->controller2)) != Steinberg::kResultOk) {
            self->controller2 = nullptr;
        }
        if (self->separate_controller) {
            self->component->queryInterface(Steinberg::Vst::IConnectionPoint::iid, reinterpret_cast<void**>(&self->component_point));
            self->controller->queryInterface(Steinberg::Vst::IConnectionPoint::iid, reinterpret_cast<void**>(&self->controller_point));
            if (self->component_point != nullptr && self->controller_point != nullptr) {
                self->component_point->connect(self->controller_point);
                self->controller_point->connect(self->component_point);
            }
        }
        const Steinberg::int32 param_count = self->controller->getParameterCount();
        for (Steinberg::int32 index = 0; index < param_count && self->params.size() < 32; ++index) {
            Steinberg::Vst::ParameterInfo info{};
            if (self->controller->getParameterInfo(index, info) != Steinberg::kResultOk) {
                continue;
            }
            if ((info.flags & Steinberg::Vst::ParameterInfo::kIsReadOnly) != 0) {
                continue;
            }
            if ((info.flags & Steinberg::Vst::ParameterInfo::kIsHidden) != 0) {
                continue;
            }
            Vst3Param param;
            param.id = info.id;
            char title[128];
            utf16_to_utf8(info.title, title, sizeof title);
            param.name = title[0] != 0 ? title : "Parameter";
            param.current = self->controller->getParamNormalized(info.id);
            self->params.push_back(param);
        }
    }

    Steinberg::Vst::SpeakerArrangement stereo = Steinberg::Vst::SpeakerArr::kStereo;
    const Steinberg::int32 inputs = self->component->getBusCount(Steinberg::Vst::kAudio, Steinberg::Vst::kInput);
    const Steinberg::int32 outputs = self->component->getBusCount(Steinberg::Vst::kAudio, Steinberg::Vst::kOutput);
    if (inputs > 0 || outputs > 0) {
        self->processor->setBusArrangements(inputs > 0 ? &stereo : nullptr, inputs > 0 ? 1 : 0, outputs > 0 ? &stereo : nullptr, outputs > 0 ? 1 : 0);
    }
    if (inputs > 0) {
        self->component->activateBus(Steinberg::Vst::kAudio, Steinberg::Vst::kInput, 0, 1);
    }
    if (outputs > 0) {
        self->component->activateBus(Steinberg::Vst::kAudio, Steinberg::Vst::kOutput, 0, 1);
    }
    self->max_block = std::max(1, max_block);
    self->channels = std::clamp(channels, 1, 2);
    Steinberg::Vst::ProcessSetup setup{};
    setup.processMode = Steinberg::Vst::kRealtime;
    setup.symbolicSampleSize = Steinberg::Vst::kSample32;
    setup.maxSamplesPerBlock = self->max_block;
    setup.sampleRate = sample_rate;
    if (self->processor->setupProcessing(setup) != Steinberg::kResultOk) {
        delete self;
        return nullptr;
    }
    if (self->component->setActive(1) != Steinberg::kResultOk) {
        delete self;
        return nullptr;
    }
    self->processor->setProcessing(1);
    self->latency = static_cast<int>(self->processor->getLatencySamples());
    self->scratch_in.assign(static_cast<size_t>(self->channels), std::vector<float>(static_cast<size_t>(self->max_block), 0.0f));
    self->scratch_out.assign(static_cast<size_t>(self->channels), std::vector<float>(static_cast<size_t>(self->max_block), 0.0f));
    return self;
}

void process_vst3(Vst3Plugin* self, float** audio, int channels, int frames) {
    frames = std::max(0, std::min(frames, self->max_block));
    const int count = std::min(channels, self->channels);
    float* inputs[2] = {};
    float* outputs[2] = {};
    for (int channel = 0; channel < count; ++channel) {
        std::memcpy(self->scratch_in[static_cast<size_t>(channel)].data(), audio[channel], static_cast<size_t>(frames) * sizeof(float));
        inputs[channel] = self->scratch_in[static_cast<size_t>(channel)].data();
        outputs[channel] = self->scratch_out[static_cast<size_t>(channel)].data();
        std::fill(self->scratch_out[static_cast<size_t>(channel)].begin(), self->scratch_out[static_cast<size_t>(channel)].begin() + frames, 0.0f);
    }
    const Steinberg::int32 input_buses = self->component->getBusCount(Steinberg::Vst::kAudio, Steinberg::Vst::kInput);
    Steinberg::Vst::AudioBusBuffers input{};
    input.numChannels = count;
    input.silenceFlags = 0;
    input.channelBuffers32 = inputs;
    Steinberg::Vst::AudioBusBuffers output{};
    output.numChannels = count;
    output.silenceFlags = 0;
    output.channelBuffers32 = outputs;
    Steinberg::Vst::ProcessData data{};
    data.processMode = Steinberg::Vst::kRealtime;
    data.symbolicSampleSize = Steinberg::Vst::kSample32;
    data.numSamples = frames;
    data.numInputs = input_buses > 0 ? 1 : 0;
    data.numOutputs = 1;
    data.inputs = input_buses > 0 ? &input : nullptr;
    data.outputs = &output;
    self->processor->process(data);
    for (int channel = 0; channel < count; ++channel) {
        std::memcpy(audio[channel], outputs[channel], static_cast<size_t>(frames) * sizeof(float));
    }
}

#include "clap/clap.h"

// --- CLAP -----------------------------------------------------------------------

struct ClapParam {
    clap_id id = 0;
    std::string name;
    float minimum = 0.0f;
    float maximum = 1.0f;
    float current = 0.0f;
};

struct ClapPlugin {
    void* library = nullptr;
    const clap_plugin_entry* entry = nullptr;
    const clap_plugin_factory* factory = nullptr;
    const clap_plugin* plugin = nullptr;
    const clap_plugin_params* params = nullptr;
    const clap_plugin_gui* gui = nullptr;
    clap_host* host = nullptr;          // per instance, so host callbacks know the plugin
    void* editor_parent = nullptr;      // NSView of the open editor, main thread only
    int latency = 0;
    std::vector<ClapParam> param_list;

    void close_editor() {
        if (gui != nullptr && editor_parent != nullptr) {
            gui->hide(plugin);
            gui->destroy(plugin);
        }
        editor_parent = nullptr;
    }

    ~ClapPlugin() {
        close_editor();
        if (plugin != nullptr) {
            plugin->deactivate(plugin);
            plugin->destroy(plugin);
            plugin = nullptr;
        }
        if (entry != nullptr && entry->deinit != nullptr) {
            entry->deinit();
            entry = nullptr;
        }
        delete host;
        host = nullptr;
        if (library != nullptr) {
            dlclose(library);
            library = nullptr;
        }
    }
};

static void clap_host_cb_request_restart(const clap_host*) {}
static void clap_host_cb_request_process(const clap_host*) {}
static void clap_host_cb_request_callback(const clap_host*) {}
static void clap_gui_resize_hints_changed(const clap_host*) {}
static bool clap_gui_request_resize(const clap_host* host, uint32_t width, uint32_t height) {
    auto* self = host != nullptr ? static_cast<ClapPlugin*>(host->host_data) : nullptr;
    if (self == nullptr || self->editor_parent == nullptr) {
        return false;
    }
    ag_ui_view_resized(self->editor_parent, static_cast<int>(width), static_cast<int>(height));
    if (self->gui != nullptr) {
        self->gui->set_size(self->plugin, width, height);
    }
    return true;
}
static bool clap_gui_request_show(const clap_host*) { return true; }
static bool clap_gui_request_hide(const clap_host*) { return true; }
static void clap_gui_closed(const clap_host*, bool) {}
static const clap_host_gui g_ag_clap_host_gui = {
    clap_gui_resize_hints_changed, clap_gui_request_resize, clap_gui_request_show, clap_gui_request_hide, clap_gui_closed,
};
static const void* clap_host_cb_get_extension(const clap_host*, const char* id) {
    if (id != nullptr && std::strcmp(id, CLAP_EXT_GUI) == 0) {
        return &g_ag_clap_host_gui;
    }
    return nullptr;
}

static clap_host g_ag_clap_host = {
    CLAP_VERSION,
    nullptr,
    "AudioGridder",
    "OpenGrid",
    "https://audiogridder.com",
    "0.1.1-alpha",
    clap_host_cb_get_extension,
    clap_host_cb_request_restart,
    clap_host_cb_request_process,
    clap_host_cb_request_callback,
};

static uint32_t clap_in_ev_size(const clap_input_events*) { return 0; }
static const clap_event_header* clap_in_ev_get(const clap_input_events*, uint32_t) { return nullptr; }
static bool clap_out_ev_push(const clap_output_events*, const clap_event_header*) { return true; }

std::string clap_binary(const std::filesystem::path& bundle) {
    const std::filesystem::path macos = bundle / "Contents" / "MacOS";
    if (!std::filesystem::is_directory(macos)) {
        return {};
    }
    for (const auto& entry : std::filesystem::directory_iterator(macos)) {
        if (entry.is_regular_file()) {
            return entry.path().string();
        }
    }
    return {};
}

bool collect_clap_bundle(const std::filesystem::path& bundle, std::vector<Catalog>& out) {
    if (skipped_name(bundle.filename().string())) {
        return false;
    }
    const std::string binary = clap_binary(bundle);
    if (binary.empty()) {
        return false;
    }
    void* lib = dlopen(binary.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (lib == nullptr) {
        lib = dlopen(binary.c_str(), RTLD_NOW | RTLD_GLOBAL);
    }
    if (lib == nullptr) {
        return false;
    }
    auto* entry = static_cast<const clap_plugin_entry*>(dlsym(lib, "clap_entry"));
    if (entry == nullptr || !entry->init(binary.c_str())) {
        dlclose(lib);
        return false;
    }
    auto* factory = static_cast<const clap_plugin_factory*>(entry->get_factory(CLAP_PLUGIN_FACTORY_ID));
    if (factory == nullptr) {
        dlclose(lib);
        return false;
    }
    const uint32_t count = factory->get_plugin_count(factory);
    bool stored = false;
    for (uint32_t i = 0; i < count; ++i) {
        const clap_plugin_descriptor* desc = factory->get_plugin_descriptor(factory, i);
        if (desc == nullptr) {
            continue;
        }
        Catalog item{};
        copy_cstr(item.name, sizeof item.name, desc->name);
        copy_cstr(item.company, sizeof item.company, desc->vendor != nullptr ? desc->vendor : "CLAP");
        const std::string id = std::string("clap:") + bundle.string() + "#" + (desc->id != nullptr ? desc->id : "");
        if (id.size() >= sizeof item.id) {
            continue;
        }
        copy_cstr(item.id, sizeof item.id, id.c_str());
        copy_cstr(item.format, sizeof item.format, "CLAP");
        item.is_instrument = 0;
        if (desc->features != nullptr) {
            for (const char* const* f = desc->features; *f != nullptr; ++f) {
                if (std::strstr(*f, "instrument") != nullptr) {
                    item.is_instrument = 1;
                    break;
                }
            }
        }
        out.push_back(item);
        stored = true;
    }
    return stored;
}

void collect_clap(std::vector<Catalog>& out, int cap) {
    const std::string roots[] = {
        "/Library/Audio/Plug-Ins/CLAP",
        home_dir().empty() ? std::string() : home_dir() + "/Library/Audio/Plug-Ins/CLAP"
    };
    for (const auto& root : roots) {
        if (root.empty() || !std::filesystem::is_directory(root)) {
            continue;
        }
        for (const auto& entry : std::filesystem::directory_iterator(root)) {
            if (static_cast<int>(out.size()) >= cap) {
                return;
            }
            if (entry.path().extension() == ".clap" && std::filesystem::is_directory(entry.path())) {
                collect_clap_bundle(entry.path(), out);
            }
        }
    }
}

ClapPlugin* open_clap(const char* id, float sample_rate, int max_block, int channels) {
    if (id == nullptr || std::strncmp(id, "clap:", 5) != 0) {
        return nullptr;
    }
    std::string id_str = id + 5;
    size_t hash_pos = id_str.find('#');
    std::string bundle_str = (hash_pos != std::string::npos) ? id_str.substr(0, hash_pos) : id_str;
    std::string plugin_id = (hash_pos != std::string::npos) ? id_str.substr(hash_pos + 1) : "";

    std::filesystem::path bundle(bundle_str);
    const std::string binary = clap_binary(bundle);
    if (binary.empty()) {
        return nullptr;
    }
    void* lib = dlopen(binary.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (lib == nullptr) {
        lib = dlopen(binary.c_str(), RTLD_NOW | RTLD_GLOBAL);
    }
    if (lib == nullptr) {
        return nullptr;
    }
    auto* entry = static_cast<const clap_plugin_entry*>(dlsym(lib, "clap_entry"));
    if (entry == nullptr || !entry->init(binary.c_str())) {
        dlclose(lib);
        return nullptr;
    }
    auto* factory = static_cast<const clap_plugin_factory*>(entry->get_factory(CLAP_PLUGIN_FACTORY_ID));
    if (factory == nullptr) {
        dlclose(lib);
        return nullptr;
    }

    auto* host = new clap_host(g_ag_clap_host);
    const clap_plugin* plugin = nullptr;
    if (!plugin_id.empty()) {
        plugin = factory->create_plugin(factory, host, plugin_id.c_str());
    } else {
        const uint32_t count = factory->get_plugin_count(factory);
        if (count > 0) {
            const clap_plugin_descriptor* desc = factory->get_plugin_descriptor(factory, 0);
            if (desc != nullptr) {
                plugin = factory->create_plugin(factory, host, desc->id);
            }
        }
    }
    if (plugin == nullptr || !plugin->init(plugin)) {
        if (plugin != nullptr) {
            plugin->destroy(plugin);
        }
        delete host;
        dlclose(lib);
        return nullptr;
    }

    auto* self = new ClapPlugin();
    self->host = host;
    host->host_data = self;
    self->library = lib;
    self->entry = entry;
    self->factory = factory;
    self->plugin = plugin;
    self->params = static_cast<const clap_plugin_params*>(plugin->get_extension(plugin, CLAP_EXT_PARAMS));
    self->gui = static_cast<const clap_plugin_gui*>(plugin->get_extension(plugin, CLAP_EXT_GUI));
    if (self->params != nullptr) {
        uint32_t count = self->params->count(plugin);
        for (uint32_t i = 0; i < count; ++i) {
            clap_param_info info{};
            if (self->params->get_info(plugin, i, &info)) {
                ClapParam p{};
                p.id = info.id;
                p.name = info.name;
                p.minimum = static_cast<float>(info.min_value);
                p.maximum = static_cast<float>(info.max_value);
                double cur = 0;
                self->params->get_value(plugin, info.id, &cur);
                p.current = static_cast<float>(cur);
                self->param_list.push_back(p);
            }
        }
    }
    if (const auto* lat_ext = static_cast<const clap_plugin_latency*>(plugin->get_extension(plugin, CLAP_EXT_LATENCY))) {
        self->latency = static_cast<int>(lat_ext->get(plugin));
    }
    plugin->activate(plugin, sample_rate, 1, max_block);
    plugin->start_processing(plugin);
    return self;
}

void process_clap(ClapPlugin* self, float** audio, int channels, int frames) {
    if (self == nullptr || self->plugin == nullptr || audio == nullptr || channels <= 0 || frames <= 0) {
        return;
    }
    float* channel_ptrs[2] = { audio[0], channels > 1 ? audio[1] : audio[0] };
    clap_audio_buffer in_buf = { channel_ptrs, nullptr, static_cast<uint32_t>(channels), 0, 0 };
    clap_audio_buffer out_buf = { channel_ptrs, nullptr, static_cast<uint32_t>(channels), 0, 0 };

    clap_input_events in_ev = { nullptr, clap_in_ev_size, clap_in_ev_get };
    clap_output_events out_ev = { nullptr, clap_out_ev_push };

    clap_process process_data{};
    process_data.steady_time = 0;
    process_data.frames_count = static_cast<uint32_t>(frames);
    process_data.transport = nullptr;
    process_data.audio_inputs = &in_buf;
    process_data.audio_outputs = &out_buf;
    process_data.audio_inputs_count = 1;
    process_data.audio_outputs_count = 1;
    process_data.in_events = &in_ev;
    process_data.out_events = &out_ev;

    self->plugin->process(self->plugin, &process_data);
}

enum class Kind { None, Lv2, Vst3, Clap };

struct Instance {
    Kind kind = Kind::None;
    Lv2Plugin* lv2 = nullptr;
    Vst3Plugin* vst3 = nullptr;
    ClapPlugin* clap = nullptr;
};

std::mutex& gate() {
    static std::mutex mutex;
    return mutex;
}

std::vector<Catalog>& catalog_cache() {
    static std::vector<Catalog> items;
    return items;
}

bool& catalog_ready() {
    static bool ready = false;
    return ready;
}

}  // namespace

struct AgNative;

extern "C" {

struct AgCatalogItem {
    char name[128];
    char company[128];
    char id[768];
    char format[16];
    int is_instrument;
};

int ag_native_scan(AgCatalogItem* out, int cap) {
    std::lock_guard<std::mutex> lock(gate());
    if (!catalog_ready()) {
        std::vector<Catalog> items;
        collect_lv2(items, std::numeric_limits<int>::max());  // lilv metadata only, no instances
        collect_vst3(items, static_cast<int>(items.size()) + 8);
        collect_clap(items, static_cast<int>(items.size()) + 8);
        catalog_cache() = std::move(items);
        catalog_ready() = true;
    }
    // out == nullptr asks for the catalog size, so the caller can size its buffer.
    if (out == nullptr) {
        return static_cast<int>(catalog_cache().size());
    }
    const int count = std::min(cap, static_cast<int>(catalog_cache().size()));
    for (int index = 0; index < count; ++index) {
        std::memcpy(&out[index], &catalog_cache()[static_cast<size_t>(index)], sizeof(Catalog));
    }
    return count;
}

AgNative* ag_native_open(const char* id, float sample_rate, int max_block, int channels) {
    std::lock_guard<std::mutex> lock(gate());
    auto* instance = new Instance();
    if (id != nullptr && std::strncmp(id, "lv2:", 4) == 0) {
        instance->lv2 = open_lv2(id, sample_rate, max_block, channels);
        instance->kind = instance->lv2 != nullptr ? Kind::Lv2 : Kind::None;
    } else if (id != nullptr && std::strncmp(id, "vst3:", 5) == 0) {
        instance->vst3 = open_vst3(id, sample_rate, max_block, channels);
        instance->kind = instance->vst3 != nullptr ? Kind::Vst3 : Kind::None;
    } else if (id != nullptr && std::strncmp(id, "clap:", 5) == 0) {
        instance->clap = open_clap(id, sample_rate, max_block, channels);
        instance->kind = instance->clap != nullptr ? Kind::Clap : Kind::None;
    }
    if (instance->kind == Kind::None) {
        delete instance;
        return nullptr;
    }
    return reinterpret_cast<AgNative*>(instance);
}

void ag_native_close(AgNative* raw) {
    std::lock_guard<std::mutex> lock(gate());
    auto* instance = reinterpret_cast<Instance*>(raw);
    if (instance == nullptr) {
        return;
    }
    delete instance->lv2;
    delete instance->vst3;
    delete instance->clap;
    delete instance;
}

int ag_native_latency(AgNative* raw) {
    auto* instance = reinterpret_cast<Instance*>(raw);
    if (instance == nullptr) {
        return 0;
    }
    if (instance->kind == Kind::Lv2) {
        return instance->lv2->latency;
    }
    if (instance->kind == Kind::Vst3) {
        return instance->vst3->latency;
    }
    if (instance->kind == Kind::Clap) {
        return instance->clap->latency;
    }
    return 0;
}

int ag_native_param_count(AgNative* raw) {
    auto* instance = reinterpret_cast<Instance*>(raw);
    if (instance == nullptr) {
        return 0;
    }
    if (instance->kind == Kind::Lv2) {
        return static_cast<int>(instance->lv2->params.size());
    }
    if (instance->kind == Kind::Vst3) {
        return static_cast<int>(instance->vst3->params.size());
    }
    if (instance->kind == Kind::Clap) {
        return static_cast<int>(instance->clap->param_list.size());
    }
    return 0;
}

int ag_native_param(AgNative* raw, int index, char* name, int name_cap, float* minimum, float* maximum, float* current) {
    auto* instance = reinterpret_cast<Instance*>(raw);
    if (instance == nullptr || index < 0) {
        return 0;
    }
    if (instance->kind == Kind::Lv2) {
        if (index >= static_cast<int>(instance->lv2->params.size())) {
            return 0;
        }
        const Lv2Param& param = instance->lv2->params[static_cast<size_t>(index)];
        copy_cstr(name, static_cast<size_t>(name_cap), param.name.c_str());
        const float span = param.maximum - param.minimum;
        *minimum = param.minimum;
        *maximum = param.maximum;
        *current = span != 0.0f ? (param.current - param.minimum) / span : 0.0f;
        return 1;
    }
    if (instance->kind == Kind::Vst3) {
        if (index >= static_cast<int>(instance->vst3->params.size())) {
            return 0;
        }
        const Vst3Param& param = instance->vst3->params[static_cast<size_t>(index)];
        copy_cstr(name, static_cast<size_t>(name_cap), param.name.c_str());
        *minimum = 0.0f;
        *maximum = 1.0f;
        *current = static_cast<float>(param.current);
        return 1;
    }
    if (instance->kind == Kind::Clap) {
        if (index >= static_cast<int>(instance->clap->param_list.size())) {
            return 0;
        }
        const ClapParam& param = instance->clap->param_list[static_cast<size_t>(index)];
        copy_cstr(name, static_cast<size_t>(name_cap), param.name.c_str());
        const float span = param.maximum - param.minimum;
        *minimum = param.minimum;
        *maximum = param.maximum;
        *current = span != 0.0f ? (param.current - param.minimum) / span : 0.0f;
        return 1;
    }
    return 0;
}

int ag_native_set_param(AgNative* raw, int index, float normalized) {
    auto* instance = reinterpret_cast<Instance*>(raw);
    if (instance == nullptr || index < 0) {
        return 0;
    }
    normalized = clampf(normalized, 0.0f, 1.0f);
    if (instance->kind == Kind::Lv2) {
        if (index >= static_cast<int>(instance->lv2->params.size())) {
            return 0;
        }
        Lv2Param& param = instance->lv2->params[static_cast<size_t>(index)];
        param.current = param.minimum + normalized * (param.maximum - param.minimum);
        param.touched = true;
        return 1;
    }
    if (instance->kind == Kind::Vst3) {
        if (index >= static_cast<int>(instance->vst3->params.size()) || instance->vst3->controller == nullptr) {
            return 0;
        }
        Vst3Param& param = instance->vst3->params[static_cast<size_t>(index)];
        if (instance->vst3->controller2 != nullptr) {
            instance->vst3->controller2->beginEditFromHost(param.id);
        }
        const Steinberg::tresult status = instance->vst3->controller->setParamNormalized(param.id, normalized);
        if (instance->vst3->controller2 != nullptr) {
            instance->vst3->controller2->endEditFromHost(param.id);
        }
        if (status != Steinberg::kResultOk) {
            return 0;
        }
        param.current = instance->vst3->controller->getParamNormalized(param.id);
        return 1;
    }
    if (instance->kind == Kind::Clap) {
        if (index >= static_cast<int>(instance->clap->param_list.size()) || instance->clap->params == nullptr) {
            return 0;
        }
        ClapParam& param = instance->clap->param_list[static_cast<size_t>(index)];
        param.current = param.minimum + normalized * (param.maximum - param.minimum);
        return 1;
    }
    return 0;
}

float ag_native_get_param(AgNative* raw, int index) {
    auto* instance = reinterpret_cast<Instance*>(raw);
    if (instance == nullptr || index < 0) {
        return 0.0f;
    }
    if (instance->kind == Kind::Lv2) {
        if (index >= static_cast<int>(instance->lv2->params.size())) {
            return 0.0f;
        }
        const Lv2Param& param = instance->lv2->params[static_cast<size_t>(index)];
        const float span = param.maximum - param.minimum;
        return span != 0.0f ? (param.current - param.minimum) / span : 0.0f;
    }
    if (instance->kind == Kind::Vst3) {
        if (index >= static_cast<int>(instance->vst3->params.size())) {
            return 0.0f;
        }
        return static_cast<float>(instance->vst3->params[static_cast<size_t>(index)].current);
    }
    if (instance->kind == Kind::Clap) {
        if (index >= static_cast<int>(instance->clap->param_list.size())) {
            return 0.0f;
        }
        const ClapParam& param = instance->clap->param_list[static_cast<size_t>(index)];
        const float span = param.maximum - param.minimum;
        return span != 0.0f ? (param.current - param.minimum) / span : 0.0f;
    }
    return 0.0f;
}

void ag_native_process(AgNative* raw, float** audio, int channels, int frames) {
    auto* instance = reinterpret_cast<Instance*>(raw);
    if (instance == nullptr || audio == nullptr || channels <= 0 || frames <= 0) {
        return;
    }
    if (instance->kind == Kind::Lv2) {
        process_lv2(instance->lv2, audio, channels, frames);
    } else if (instance->kind == Kind::Vst3) {
        process_vst3(instance->vst3, audio, channels, frames);
    } else if (instance->kind == Kind::Clap) {
        process_clap(instance->clap, audio, channels, frames);
    }
}


// Attach the plugin's own editor to nsview (main thread). Returns 1 with the editor size,
// 0 when the plugin has no editor (LV2 UIs are not hosted).
int ag_native_editor_open(AgNative* raw, void* nsview, int* width, int* height) {
    auto* instance = reinterpret_cast<Instance*>(raw);
    if (instance == nullptr || nsview == nullptr) {
        return 0;
    }
    if (instance->kind == Kind::Vst3 && instance->vst3 != nullptr && instance->vst3->controller != nullptr) {
        Vst3Plugin* self = instance->vst3;
        self->close_editor();
        Steinberg::IPlugView* view = self->controller->createView(Steinberg::Vst::ViewType::kEditor);
        if (view == nullptr) {
            return 0;
        }
        if (view->isPlatformTypeSupported(Steinberg::kPlatformTypeNSView) != Steinberg::kResultTrue) {
            view->release();
            return 0;
        }
        self->view = view;
        self->frame = new PlugFrame(nsview);
        view->setFrame(self->frame);
        Steinberg::ViewRect rect;
        if (view->getSize(&rect) == Steinberg::kResultOk && rect.getWidth() > 0 && rect.getHeight() > 0) {
            ag_ui_view_resized(nsview, rect.getWidth(), rect.getHeight());
        }
        if (view->attached(nsview, Steinberg::kPlatformTypeNSView) != Steinberg::kResultOk) {
            self->close_editor();
            return 0;
        }
        if (view->getSize(&rect) == Steinberg::kResultOk) {
            *width = rect.getWidth();
            *height = rect.getHeight();
        }
        return 1;
    }
    if (instance->kind == Kind::Clap && instance->clap != nullptr && instance->clap->gui != nullptr) {
        ClapPlugin* self = instance->clap;
        const clap_plugin_gui* gui = self->gui;
        self->close_editor();
        if (!gui->is_api_supported(self->plugin, CLAP_WINDOW_API_COCOA, false) ||
            !gui->create(self->plugin, CLAP_WINDOW_API_COCOA, false)) {
            return 0;
        }
        self->editor_parent = nsview;
        uint32_t w = 0, h = 0;
        if (gui->get_size(self->plugin, &w, &h) && w > 0 && h > 0) {
            ag_ui_view_resized(nsview, static_cast<int>(w), static_cast<int>(h));
        }
        clap_window window{};
        window.api = CLAP_WINDOW_API_COCOA;
        window.cocoa = nsview;
        if (!gui->set_parent(self->plugin, &window)) {
            gui->destroy(self->plugin);
            self->editor_parent = nullptr;
            return 0;
        }
        gui->show(self->plugin);
        if (gui->get_size(self->plugin, &w, &h)) {
            *width = static_cast<int>(w);
            *height = static_cast<int>(h);
        }
        return 1;
    }
    return 0;
}

void ag_native_editor_close(AgNative* raw) {
    auto* instance = reinterpret_cast<Instance*>(raw);
    if (instance == nullptr) {
        return;
    }
    if (instance->vst3 != nullptr) {
        instance->vst3->close_editor();
    }
    if (instance->clap != nullptr) {
        instance->clap->close_editor();
    }
}

}  // extern "C"

