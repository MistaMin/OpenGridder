// Exercise the built module through the same public COM interfaces a DAW uses.
#include "pluginterfaces/base/ipluginbase.h"
#include "pluginterfaces/base/ibstream.h"
#include "pluginterfaces/vst/ivstcomponent.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/vstspeaker.h"
#include "pluginterfaces/vst/ivstparameterchanges.h"
#include <dlfcn.h>
#include <vector>
#include <cstring>
#include <cstdio>
#include <cstdlib>
#include <cmath>
using namespace Steinberg;using namespace Steinberg::Vst;
void check(bool condition,const char* message){if(!condition){std::fprintf(stderr,"FAIL: %s\n",message);std::exit(1);}}
struct Stream:IBStream{
 std::vector<char> data;size_t pos=0;
 tresult PLUGIN_API queryInterface(const TUID,void** out)override{*out=nullptr;return kNoInterface;}
 uint32 PLUGIN_API addRef()override{return 1;}uint32 PLUGIN_API release()override{return 1;}
 tresult PLUGIN_API read(void* out,int32 bytes,int32* done)override{int32 n=std::min<int32>(std::min<int32>(bytes,7),data.size()-pos);if(done)*done=n;if(n>0)std::memcpy(out,data.data()+pos,n);pos+=n;return n?kResultOk:kResultFalse;}
 tresult PLUGIN_API write(void* in,int32 bytes,int32* done)override{int32 n=std::min<int32>(bytes,5);auto* p=(char*)in;data.insert(data.end(),p,p+n);if(done)*done=n;return kResultOk;}
 tresult PLUGIN_API seek(int64 p,int32 mode,int64* result)override{int64 next=mode==kIBSeekSet?p:mode==kIBSeekCur?int64(pos)+p:int64(data.size())+p;if(next<0||next>int64(data.size()))return kResultFalse;pos=next;if(result)*result=pos;return kResultOk;}
 tresult PLUGIN_API tell(int64* p)override{*p=pos;return kResultOk;}
};
struct Queue:IParamValueQueue {
 ParamID id=3;
 tresult PLUGIN_API queryInterface(const TUID,void** out)override{*out=nullptr;return kNoInterface;}
 uint32 PLUGIN_API addRef()override{return 1;}uint32 PLUGIN_API release()override{return 1;}
 ParamID PLUGIN_API getParameterId()override{return id;}
 int32 PLUGIN_API getPointCount()override{return 2;}
 tresult PLUGIN_API getPoint(int32 i,int32& offset,ParamValue& value)override{if(i<0||i>1)return kInvalidArgument;offset=i==0?256:384;value=i==0?0.5:0.75;return kResultOk;}
 tresult PLUGIN_API addPoint(int32,ParamValue,int32&)override{return kNotImplemented;}
};
struct Changes:IParameterChanges {
 Queue queue;
 tresult PLUGIN_API queryInterface(const TUID,void** out)override{*out=nullptr;return kNoInterface;}
 uint32 PLUGIN_API addRef()override{return 1;}uint32 PLUGIN_API release()override{return 1;}
 int32 PLUGIN_API getParameterCount()override{return 1;}
 IParamValueQueue* PLUGIN_API getParameterData(int32 i)override{return i==0?&queue:nullptr;}
 IParamValueQueue* PLUGIN_API addParameterData(const ParamID&,int32&)override{return nullptr;}
};
int main(int argc,char** argv){
 check(argc==2,"pass module binary");void* module=dlopen(argv[1],RTLD_NOW|RTLD_LOCAL);if(!module)std::fprintf(stderr,"%s\n",dlerror());check(module,"module loads");
 auto get=(IPluginFactory*(*)())dlsym(module,"GetPluginFactory");check(get,"factory symbol");auto* f=get();check(f->countClasses()==1,"one class");PClassInfo info{};check(f->getClassInfo(0,&info)==kResultOk,"class info");
 for(int count:{1,2})for(int precision:{kSample32,kSample64})for(double rate:{44100.,48000.,96000.}){
 IComponent* component=nullptr;check(f->createInstance(info.cid,IComponent::iid,(void**)&component)==kResultOk,"IComponent created");check(component->initialize(nullptr)==kResultOk,"initialize");
 IAudioProcessor* audio=nullptr;IEditController* control=nullptr;
 check(component->queryInterface(IAudioProcessor::iid,(void**)&audio)==kResultOk,"IAudioProcessor available");check(component->queryInterface(IEditController::iid,(void**)&control)==kResultOk,"IEditController available");
 check(component->getBusCount(kEvent,kInput)==1,"MIDI input bus");check(control->getParameterCount()==25,"all 25 parameters visible");ParameterInfo pi{};check(control->getParameterInfo(3,pi)==kResultOk,"input gain parameter");check(std::abs(control->normalizedParamToPlain(3,0.625)-6)<1e-6,"real-unit normalization");
 check(control->setParamNormalized(2,0)==kResultOk,"128 profile");check(control->setParamNormalized(3,0.625)==kResultOk,"+6 input gain");check(audio->getLatencySamples()==128,"reported delay");
 SpeakerArrangement arr=count==1?SpeakerArr::kMono:SpeakerArr::kStereo;check(audio->setBusArrangements(&arr,1,&arr,1)==kResultOk,"mono/stereo negotiation");
 ProcessSetup setup{};setup.sampleRate=rate;setup.maxSamplesPerBlock=512;setup.symbolicSampleSize=precision;setup.processMode=kRealtime;
 check(audio->setupProcessing(setup)==kResultOk,"prepare");check(component->setActive(1)==kResultOk,"activate");check(audio->setProcessing(1)==kResultOk,"start");
 std::vector<float> l(512,0),r(512,0),ol(512,-9),orr(512,-9);std::vector<double> dl(512,0),dr(512,0),dol(512,-9),dor(512,-9);l[0]=r[0]=0.25f;dl[0]=dr[0]=0.25;
 float* input32[2]={l.data(),r.data()};float* output32[2]={ol.data(),orr.data()};double* input64[2]={dl.data(),dr.data()};double* output64[2]={dol.data(),dor.data()};
 AudioBusBuffers in{},out{};in.numChannels=out.numChannels=count;if(precision==kSample32){in.channelBuffers32=input32;out.channelBuffers32=output32;}else{in.channelBuffers64=input64;out.channelBuffers64=output64;}
 ProcessData data{};data.numSamples=512;data.numInputs=data.numOutputs=1;data.inputs=&in;data.outputs=&out;data.symbolicSampleSize=precision;data.processMode=kRealtime;
 check(audio->process(data)==kResultOk,"render");double expected=0.25*std::pow(10.,6./20.);for(int ch=0;ch<count;ch++)for(int n=0;n<512;n++){double v=precision==kSample32?output32[ch][n]:output64[ch][n];check(std::isfinite(v),"finite render");check(std::abs(v-(n==128?expected:0))<1e-5,"rendered gain and impulse latency");}
 // An automation point at frame 256 must not change the first 256 input samples.
 audio->setProcessing(0);audio->setProcessing(1);
 std::fill(l.begin(),l.end(),1.f);std::fill(r.begin(),r.end(),1.f);std::fill(dl.begin(),dl.end(),1.);std::fill(dr.begin(),dr.end(),1.);
 Changes automation;data.inputParameterChanges=&automation;
 check(audio->process(data)==kResultOk,"sample-offset automation render");
 double first=precision==kSample32?output32[0][200]:output64[0][200];
 double beforePoint=precision==kSample32?output32[0][383]:output64[0][383];
 double afterPoint=precision==kSample32?output32[0][400]:output64[0][400];
 check(std::abs(first-std::pow(10.,6./20.))<1e-5,"late automation leaves early audio unchanged");
 check(std::abs(beforePoint-first)<1e-5,"automation starts at its exact sample offset plus delay");
 check(afterPoint<beforePoint,"first queue point was applied before second point");
 check(std::abs(control->getParamNormalized(3)-0.75)<1e-6,"last automation value retained");
 data.inputParameterChanges=nullptr;control->setParamNormalized(3,0.625);
 Stream state;check(component->getState(&state)==kResultOk,"partial-write state save");control->setParamNormalized(3,0.5);check(component->setState(&state)==kResultOk,"partial-read state load");check(std::abs(control->getParamNormalized(3)-0.625)<1e-6,"state restored gain");
 audio->setProcessing(0);component->setActive(0);control->release();audio->release();component->terminate();component->release();std::printf("PASS %d channels %s %.0f Hz latency 128 gain %.6f\n",count,precision==kSample32?"f32":"f64",rate,expected);
 }
 f->release();dlclose(module);return 0;
}
