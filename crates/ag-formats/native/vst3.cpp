// VST3 interfaces over an OpenPluginSDK processor. No JUCE dependency.
#include "pluginterfaces/base/ipluginbase.h"
#include "pluginterfaces/base/ibstream.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivstcomponent.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/ivstparameterchanges.h"
#include "pluginterfaces/vst/ivstevents.h"
#include "pluginterfaces/vst/ivstprocesscontext.h"
#include <array>
#include "pluginterfaces/vst/vstspeaker.h"
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>
#include "bridge.h"
#include "editor.h"
#include "pluginterfaces/gui/iplugview.h"

using namespace Steinberg;
using namespace Steinberg::Vst;
namespace {
const TUID CID = {(char)0xA1,0x1D,0x0A,0x6D,0x6E,0x72,0,1,(char)0x8F,0x3C,0x4B,0x21,(char)0xC0,(char)0xFF,(char)0xEE,1};
void text16(const char* src, TChar* dst, size_t cap=128){size_t i=0; for(;i+1<cap && src[i];++i)dst[i]=(unsigned char)src[i];dst[i]=0;}
bool same(const TUID a,const FUID& b){return FUnknownPrivate::iidEqual(a,b);}
bool stream_read(IBStream* s,void* dst,int32 count){auto* p=(char*)dst;while(count>0){int32 n=0;s->read(p,count,&n);if(n<=0||n>count)return false;p+=n;count-=n;}return true;}
bool stream_write(IBStream* s,const void* src,int32 count){auto* p=(const char*)src;while(count>0){int32 n=0;s->write((void*)p,count,&n);if(n<=0||n>count)return false;p+=n;count-=n;}return true;}
class View final:public IPlugView {
 std::atomic<uint32> refs{1};FUnknown* owner;void* processor;void* view=nullptr;AgEdit edit;AgGesture gesture;void* context;
public:
 View(FUnknown* o,void* p,void* c,AgEdit e,AgGesture g):owner(o),processor(p),edit(e),gesture(g),context(c){owner->addRef();}
 ~View(){if(view)ag_editor_release(view);owner->release();}
 tresult PLUGIN_API queryInterface(const TUID id,void** out)override{if(!out)return kInvalidArgument;*out=nullptr;if(same(id,FUnknown::iid)||same(id,IPlugView::iid)){*out=this;addRef();return kResultOk;}return kNoInterface;}
 uint32 PLUGIN_API addRef()override{return refs.fetch_add(1)+1;}
 uint32 PLUGIN_API release()override{uint32 n=refs.fetch_sub(1)-1;if(!n)delete this;return n;}
 tresult PLUGIN_API isPlatformTypeSupported(FIDString type)override{return type&&std::strcmp(type,kPlatformTypeNSView)==0?kResultOk:kResultFalse;}
 tresult PLUGIN_API attached(void* parent,FIDString type)override{if(view||!parent||isPlatformTypeSupported(type)!=kResultOk)return kResultFalse;view=ag_editor_create(processor,context,edit,gesture);return ag_editor_attach(view,parent)?kResultOk:kResultFalse;}
 tresult PLUGIN_API removed()override{if(view){ag_editor_release(view);view=nullptr;}return kResultOk;}
 tresult PLUGIN_API onWheel(float)override{return kResultFalse;}
 tresult PLUGIN_API onKeyDown(char16,int16,int16)override{return kResultFalse;}
 tresult PLUGIN_API onKeyUp(char16,int16,int16)override{return kResultFalse;}
 tresult PLUGIN_API getSize(ViewRect* rect)override{if(!rect)return kInvalidArgument;*rect={0,0,720,420};return kResultOk;}
 tresult PLUGIN_API onSize(ViewRect* rect)override{return rect&&rect->right-rect->left==720&&rect->bottom-rect->top==420?kResultOk:kResultFalse;}
 tresult PLUGIN_API onFocus(TBool)override{return kResultOk;}
 tresult PLUGIN_API setFrame(IPlugFrame*)override{return kResultOk;}
 tresult PLUGIN_API canResize()override{return kResultFalse;}
 tresult PLUGIN_API checkSizeConstraint(ViewRect* rect)override{if(!rect)return kInvalidArgument;rect->right=rect->left+720;rect->bottom=rect->top+420;return kResultOk;}
};
class Insert final: public IComponent, public IAudioProcessor, public IEditController {
    std::atomic<uint32> refs{1};
    void* processor=ag_insert_create();
    IComponentHandler* handler=nullptr;
    SpeakerArrangement arrangement=SpeakerArr::kStereo;
    double rate=48000;
    int32 maximum=512;
    int32 format=kSample32;
    bool active=false,processing=false;
    int channels()const{return arrangement==SpeakerArr::kMono?1:2;}
public:
    ~Insert(){if(handler)handler->release();ag_insert_destroy(processor);}
    tresult PLUGIN_API queryInterface(const TUID id,void** out)override{
        if(!out)return kInvalidArgument;*out=nullptr;
        if(same(id,FUnknown::iid)||same(id,IPluginBase::iid)||same(id,IComponent::iid))*out=static_cast<IComponent*>(this);
        else if(same(id,IAudioProcessor::iid))*out=static_cast<IAudioProcessor*>(this);
        else if(same(id,IEditController::iid))*out=static_cast<IEditController*>(this);
        else return kNoInterface;
        addRef();return kResultOk;
    }
    uint32 PLUGIN_API addRef()override{return refs.fetch_add(1)+1;}
    uint32 PLUGIN_API release()override{uint32 n=refs.fetch_sub(1)-1;if(!n)delete this;return n;}
    tresult PLUGIN_API initialize(FUnknown*)override{return kResultOk;}
    tresult PLUGIN_API terminate()override{return setComponentHandler(nullptr);}
    tresult PLUGIN_API getControllerClassId(TUID)override{return kResultFalse;}
    tresult PLUGIN_API setIoMode(IoMode)override{return kResultOk;}
    int32 PLUGIN_API getBusCount(MediaType t,BusDirection d)override{return ((d==kInput||d==kOutput)&&t==kAudio)||(d==kInput&&t==kEvent)?1:0;}
    tresult PLUGIN_API getBusInfo(MediaType t,BusDirection d,int32 i,BusInfo& b)override{
        if(t==kEvent&&d==kInput&&i==0){std::memset(&b,0,sizeof(b));b.mediaType=t;b.direction=d;b.channelCount=16;b.busType=kMain;b.flags=BusInfo::kDefaultActive;text16("MIDI Input",b.name);return kResultOk;}
        if(t!=kAudio||i!=0||(d!=kInput&&d!=kOutput))return kInvalidArgument;
        std::memset(&b,0,sizeof(b));b.mediaType=t;b.direction=d;b.channelCount=channels();b.busType=kMain;b.flags=BusInfo::kDefaultActive;text16(d==kInput?"Input":"Output",b.name);return kResultOk;
    }
    tresult PLUGIN_API getRoutingInfo(RoutingInfo& in,RoutingInfo& out)override{if(in.mediaType!=kAudio||in.busIndex!=0)return kResultFalse;out=in;return kResultOk;}
    tresult PLUGIN_API activateBus(MediaType t,BusDirection d,int32 i,TBool)override{return i==0&&((t==kAudio&&(d==kInput||d==kOutput))||(t==kEvent&&d==kInput))?kResultOk:kResultFalse;}
    tresult PLUGIN_API setActive(TBool v)override{if(v&&!ag_insert_prepare(processor,rate,maximum,channels()))return kResultFalse;active=v!=0;return kResultOk;}
    tresult PLUGIN_API setState(IBStream* s)override{
        if(!s)return kInvalidArgument;
        uint8_t header[8];if(!stream_read(s,header,8)||std::memcmp(header,"AGO1",4))return kResultFalse;
        uint32_t n=uint32_t(header[4])|(uint32_t(header[5])<<8)|(uint32_t(header[6])<<16)|(uint32_t(header[7])<<24);
        if(!n||n>16*1024*1024)return kResultFalse;
        std::vector<uint8_t> bytes(n);if(!stream_read(s,bytes.data(),n))return kResultFalse;
        return ag_insert_load(processor,bytes.data(),n)?kResultOk:kResultFalse;
    }
    tresult PLUGIN_API getState(IBStream* s)override{
        if(!s)return kInvalidArgument;
        uint32_t n=ag_insert_save(processor,nullptr,0);if(!n||n>16*1024*1024)return kResultFalse;
        std::vector<uint8_t> bytes(n);uint32_t actual=ag_insert_save(processor,bytes.data(),n);if(!actual||actual>n)return kResultFalse;
        uint8_t header[8]={'A','G','O','1',uint8_t(actual),uint8_t(actual>>8),uint8_t(actual>>16),uint8_t(actual>>24)};
        return stream_write(s,header,8)&&stream_write(s,bytes.data(),actual)?kResultOk:kResultFalse;
    }
    tresult PLUGIN_API setBusArrangements(SpeakerArrangement* in,int32 ni,SpeakerArrangement* out,int32 no)override{
        if(active||!in||!out||ni!=1||no!=1||in[0]!=out[0]||(in[0]!=SpeakerArr::kMono&&in[0]!=SpeakerArr::kStereo))return kResultFalse;
        arrangement=in[0];return kResultOk;
    }
    tresult PLUGIN_API getBusArrangement(BusDirection d,int32 i,SpeakerArrangement& out)override{if(i!=0||(d!=kInput&&d!=kOutput))return kInvalidArgument;out=arrangement;return kResultOk;}
    tresult PLUGIN_API canProcessSampleSize(int32 s)override{return s==kSample32||s==kSample64?kResultOk:kResultFalse;}
    uint32 PLUGIN_API getLatencySamples()override{return ag_insert_latency(processor);}
    tresult PLUGIN_API setupProcessing(ProcessSetup& s)override{
        if(active||!std::isfinite(s.sampleRate)||s.sampleRate<8000||s.sampleRate>768000||s.maxSamplesPerBlock<1||s.maxSamplesPerBlock>8192||canProcessSampleSize(s.symbolicSampleSize)!=kResultOk)return kInvalidArgument;
        rate=s.sampleRate;maximum=s.maxSamplesPerBlock;format=s.symbolicSampleSize;return ag_insert_prepare(processor,rate,maximum,channels())?kResultOk:kResultFalse;
    }
    tresult PLUGIN_API setProcessing(TBool v)override{processing=v!=0;if(!v)ag_insert_reset(processor);return kResultOk;}
    tresult PLUGIN_API process(ProcessData& d)override{
        constexpr size_t parameterLimit=25;
        std::array<IParamValueQueue*,parameterLimit> queues{};
        std::array<int32,parameterLimit> cursors{};
        if(d.inputParameterChanges)for(int32 q=0;q<d.inputParameterChanges->getParameterCount();++q){
            auto* queue=d.inputParameterChanges->getParameterData(q);
            if(queue&&queue->getParameterId()<parameterLimit)queues[queue->getParameterId()]=queue;
        }
        if(d.numSamples==0){
            for(auto* queue:queues)if(queue&&queue->getPointCount()>0){int32 offset=0;ParamValue value=0;if(queue->getPoint(queue->getPointCount()-1,offset,value)==kResultOk)ag_insert_set(processor,queue->getParameterId(),value);}
            return kResultOk;
        }
        if(!active||!processing||d.numSamples<0||d.numSamples>maximum||d.symbolicSampleSize!=format||d.numInputs!=1||d.numOutputs!=1||!d.inputs||!d.outputs||d.inputs[0].numChannels!=channels()||d.outputs[0].numChannels!=channels())return kInvalidArgument;
        if((format==kSample32&&(!d.inputs[0].channelBuffers32||!d.outputs[0].channelBuffers32))||(format==kSample64&&(!d.inputs[0].channelBuffers64||!d.outputs[0].channelBuffers64)))return kInvalidArgument;
        int32 cursor=0;
        while(cursor<d.numSamples){
            int32 next=d.numSamples;
            for(size_t i=0;i<queues.size();++i){auto* queue=queues[i];if(!queue)continue;
                while(cursors[i]<queue->getPointCount()){
                    int32 offset=0;ParamValue value=0;
                    if(queue->getPoint(cursors[i],offset,value)!=kResultOk){++cursors[i];continue;}
                    if(offset>cursor){next=std::min(next,offset);break;}
                    ag_insert_set(processor,queue->getParameterId(),value);++cursors[i];
                }
            }
            next=std::clamp(next,cursor+1,d.numSamples);
            if(d.processContext){const auto& c=*d.processContext;const double tempo=(c.state&ProcessContext::kTempoValid)&&std::isfinite(c.tempo)&&c.tempo>0?c.tempo:120.0;
                const int32 num=(c.state&ProcessContext::kTimeSigValid)?c.timeSigNumerator:4,den=(c.state&ProcessContext::kTimeSigValid)?c.timeSigDenominator:4;
                const int64 sample=c.projectTimeSamples+cursor;
                const double ppq=(c.state&ProcessContext::kProjectTimeMusicValid)?c.projectTimeMusic+cursor*tempo/(60.0*rate):0;
                ag_insert_position(processor,tempo,num,den,sample,sample/rate,ppq,c.state&ProcessContext::kPlaying,c.state&ProcessContext::kRecording,c.state&ProcessContext::kCycleActive);
            }else ag_insert_position(processor,120,4,4,cursor,cursor/rate,0,false,false,false);
            if(d.inputEvents)for(int32 e=0;e<d.inputEvents->getEventCount();++e){Event event{};if(d.inputEvents->getEvent(e,event)!=kResultOk||event.sampleOffset<cursor||event.sampleOffset>=next)continue;
                if(event.type==Event::kNoteOnEvent)ag_insert_midi(processor,event.sampleOffset-cursor,0x90|(event.noteOn.channel&15),std::clamp<int>(event.noteOn.pitch,0,127),std::clamp<int>(std::lround(event.noteOn.velocity*127),0,127));
                else if(event.type==Event::kNoteOffEvent)ag_insert_midi(processor,event.sampleOffset-cursor,0x80|(event.noteOff.channel&15),std::clamp<int>(event.noteOff.pitch,0,127),std::clamp<int>(std::lround(event.noteOff.velocity*127),0,127));
            }
            bool ok=false;
            if(format==kSample32){const float* in[2]{};float* out[2]{};for(int ch=0;ch<channels();++ch){in[ch]=d.inputs[0].channelBuffers32[ch]?d.inputs[0].channelBuffers32[ch]+cursor:nullptr;out[ch]=d.outputs[0].channelBuffers32[ch]?d.outputs[0].channelBuffers32[ch]+cursor:nullptr;}ok=ag_insert_process(processor,in,out,channels(),next-cursor,d.processMode==kOffline);}
            else{const double* in[2]{};double* out[2]{};for(int ch=0;ch<channels();++ch){in[ch]=d.inputs[0].channelBuffers64[ch]?d.inputs[0].channelBuffers64[ch]+cursor:nullptr;out[ch]=d.outputs[0].channelBuffers64[ch]?d.outputs[0].channelBuffers64[ch]+cursor:nullptr;}ok=ag_insert_process64(processor,in,out,channels(),next-cursor,d.processMode==kOffline);}
            if(!ok)return kResultFalse;cursor=next;
        }
        d.outputs[0].silenceFlags=0;return kResultOk;
    }

    uint32 PLUGIN_API getTailSamples()override{return kNoTail;}
    tresult PLUGIN_API setComponentState(IBStream* s)override{return setState(s);}
    int32 PLUGIN_API getParameterCount()override{return ag_insert_param_count();}
    tresult PLUGIN_API getParameterInfo(int32 i,ParameterInfo& p)override{
        char name[128]={};double lo,hi,def;int32 steps;
        if(i<0||!ag_insert_param_info(i,name,&lo,&hi,&def,&steps))return kInvalidArgument;
        std::memset(&p,0,sizeof(p));p.id=i;text16(name,p.title);text16(name,p.shortTitle);p.stepCount=steps;p.defaultNormalizedValue=def;p.flags=ParameterInfo::kCanAutomate|(i==0?ParameterInfo::kIsBypass:0);return kResultOk;
    }
    tresult PLUGIN_API getParamStringByValue(ParamID i,ParamValue v,String128 out)override{char text[128];std::snprintf(text,sizeof(text),"%.3g",normalizedParamToPlain(i,v));text16(text,out);return i<ag_insert_param_count()?kResultOk:kInvalidArgument;}
    tresult PLUGIN_API getParamValueByString(ParamID i,TChar* text,ParamValue& out)override{if(!text||i>=ag_insert_param_count())return kInvalidArgument;char bytes[128]={};int k=0;for(;k<127&&text[k];++k)bytes[k]=char(text[k]);char* end=nullptr;double v=std::strtod(bytes,&end);if(end==bytes||!std::isfinite(v))return kResultFalse;out=plainParamToNormalized(i,v);return kResultOk;}
    ParamValue PLUGIN_API normalizedParamToPlain(ParamID i,ParamValue v)override{double lo=0,hi=1;ag_insert_param_info(i,nullptr,&lo,&hi,nullptr,nullptr);return lo+std::clamp(v,0.0,1.0)*(hi-lo);}
    ParamValue PLUGIN_API plainParamToNormalized(ParamID i,ParamValue v)override{double lo=0,hi=1;ag_insert_param_info(i,nullptr,&lo,&hi,nullptr,nullptr);return hi>lo?std::clamp((v-lo)/(hi-lo),0.0,1.0):0;}
    ParamValue PLUGIN_API getParamNormalized(ParamID i)override{return ag_insert_get(processor,i);}
    tresult PLUGIN_API setParamNormalized(ParamID i,ParamValue v)override{uint32 old=ag_insert_latency(processor);if(!ag_insert_set(processor,i,v))return kInvalidArgument;if(handler&&old!=ag_insert_latency(processor))handler->restartComponent(kLatencyChanged);return kResultOk;}
    tresult PLUGIN_API setComponentHandler(IComponentHandler* h)override{if(h)h->addRef();if(handler)handler->release();handler=h;return kResultOk;}
    static void edit(void* self,uint32_t i,double value){auto* p=(Insert*)self;p->setParamNormalized(i,value);if(p->handler)p->handler->performEdit(i,value);}
    static void gesture(void* self,uint32_t i,bool start){auto* p=(Insert*)self;if(p->handler){if(start)p->handler->beginEdit(i);else p->handler->endEdit(i);}}
    IPlugView* PLUGIN_API createView(FIDString name)override{if(!name||std::strcmp(name,"editor"))return nullptr;return new View(static_cast<IComponent*>(this),processor,this,edit,gesture);}
};
class Factory final:public IPluginFactory {
public:
 tresult PLUGIN_API queryInterface(const TUID id,void** out)override{if(!out)return kInvalidArgument;*out=nullptr;if(same(id,FUnknown::iid)||same(id,IPluginFactory::iid)){*out=this;addRef();return kResultOk;}return kNoInterface;}
 uint32 PLUGIN_API addRef()override{return 1;}
 uint32 PLUGIN_API release()override{return 1;}
 tresult PLUGIN_API getFactoryInfo(PFactoryInfo* p)override{if(!p)return kInvalidArgument;std::memset(p,0,sizeof(*p));std::strcpy(p->vendor,"OpenGrid");std::strcpy(p->url,"https://opengrid.studio");p->flags=PFactoryInfo::kUnicode;return kResultOk;}
 int32 PLUGIN_API countClasses()override{return 1;}
 tresult PLUGIN_API getClassInfo(int32 i,PClassInfo* p)override{if(i||!p)return kInvalidArgument;std::memset(p,0,sizeof(*p));std::memcpy(p->cid,CID,16);p->cardinality=PClassInfo::kManyInstances;std::strcpy(p->category,kVstAudioEffectClass);std::strcpy(p->name,"AudioGridder");return kResultOk;}
 tresult PLUGIN_API createInstance(FIDString cid,FIDString iid,void** out)override{if(!out)return kInvalidArgument;*out=nullptr;if(!cid||std::memcmp(cid,CID,16))return kNoInterface;auto* instance=new Insert();auto result=instance->queryInterface(iid,out);instance->release();return result;}
};
Factory factory;
}
extern "C" void* ag_vst3_factory(){return &factory;}
