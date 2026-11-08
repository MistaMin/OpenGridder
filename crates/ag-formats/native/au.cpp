#include "bridge.h"
#include "editor.h"
#include <dlfcn.h>
#include <filesystem>
#include <AudioToolbox/AudioToolbox.h>
#include <AudioUnit/AudioUnit.h>
#include <CoreFoundation/CoreFoundation.h>
#include <algorithm>
#include <cmath>
#include <cstring>
#include <vector>
namespace {
constexpr OSStatus OK=0;
struct Listener{AudioUnitPropertyID id;AudioUnitPropertyListenerProc proc;void* data;};
struct Notify{AURenderCallback proc;void* data;};
struct Au {
 AudioComponentPlugInInterface api{};
 AudioComponentInstance component=nullptr;
 void* processor=ag_insert_create();
 AudioStreamBasicDescription format{};
 UInt32 maximum=512;
 bool initialized=false;
 bool offline=false;
 AURenderCallbackStruct callback{};
 AudioUnitConnection connection{};
 OSStatus last_error=0;
 std::vector<Listener> listeners;
 std::vector<Notify> notifies;
 float input[2][8192]{};float output[2][8192]{};
 Au(){format.mSampleRate=48000;format.mFormatID=kAudioFormatLinearPCM;format.mFormatFlags=kAudioFormatFlagsNativeFloatPacked|kAudioFormatFlagIsNonInterleaved;format.mBytesPerPacket=format.mBytesPerFrame=4;format.mFramesPerPacket=1;format.mChannelsPerFrame=2;format.mBitsPerChannel=32;}
 ~Au(){ag_insert_destroy(processor);}
 void changed(UInt32 id,UInt32 scope=0,UInt32 element=0){for(const auto& l:listeners)if(l.id==id&&l.proc)l.proc(l.data,component,id,scope,element);}
};
Au* au(void* self){return static_cast<Au*>(self);}
OSStatus open(void* self,AudioComponentInstance instance){au(self)->component=instance;return OK;}
OSStatus close(void* self){delete au(self);return OK;}
OSStatus initialize(void* self){auto* a=au(self);if(a->initialized)return OK;if(!ag_insert_prepare(a->processor,a->format.mSampleRate,a->maximum,a->format.mChannelsPerFrame))return kAudioUnitErr_FormatNotSupported;a->initialized=true;return OK;}
OSStatus uninitialize(void* self){au(self)->initialized=false;ag_insert_reset(au(self)->processor);return OK;}
OSStatus reset(void* self,AudioUnitScope,AudioUnitElement){ag_insert_reset(au(self)->processor);return OK;}
OSStatus property_info(void*,AudioUnitPropertyID id,AudioUnitScope scope,AudioUnitElement element,UInt32* size,Boolean* writable){
 UInt32 n=0;Boolean write=false;
 switch(id){
 case 65536:n=sizeof(void*);break;
 case kAudioUnitProperty_CocoaUI:n=sizeof(AudioUnitCocoaViewInfo);break;
 case kAudioUnitProperty_ClassInfo:n=sizeof(CFPropertyListRef);write=true;break;
 case kAudioUnitProperty_SampleRate:n=sizeof(Float64);write=true;break;
 case kAudioUnitProperty_StreamFormat:n=sizeof(AudioStreamBasicDescription);write=true;break;
 case kAudioUnitProperty_ParameterList:if(scope!=kAudioUnitScope_Global)return kAudioUnitErr_InvalidScope;n=ag_insert_param_count()*sizeof(AudioUnitParameterID);break;
 case kAudioUnitProperty_ParameterInfo:if(scope!=kAudioUnitScope_Global)return kAudioUnitErr_InvalidScope;if(element>=ag_insert_param_count())return kAudioUnitErr_InvalidParameter;n=sizeof(AudioUnitParameterInfo);break;
 case kAudioUnitProperty_SupportedNumChannels:n=2*sizeof(AUChannelInfo);break;
 case kAudioUnitProperty_ElementCount:n=sizeof(UInt32);break;
 case kAudioUnitProperty_OfflineRender:case kAudioUnitProperty_MaximumFramesPerSlice:case kAudioUnitProperty_BypassEffect:if(scope!=kAudioUnitScope_Global)return kAudioUnitErr_InvalidScope;n=sizeof(UInt32);write=true;break;
 case kAudioUnitProperty_ShouldAllocateBuffer:case kAudioUnitProperty_RenderQuality:n=sizeof(UInt32);write=true;break;
 case kAudioUnitProperty_LastRenderError:n=sizeof(OSStatus);break;
 case kAudioUnitProperty_Latency:case kAudioUnitProperty_TailTime:if(scope!=kAudioUnitScope_Global)return kAudioUnitErr_InvalidScope;n=sizeof(Float64);break;
 case kAudioUnitProperty_PresentPreset:if(scope!=kAudioUnitScope_Global)return kAudioUnitErr_InvalidScope;n=sizeof(AUPreset);write=true;break;
 case kAudioUnitProperty_SetRenderCallback:n=sizeof(AURenderCallbackStruct);write=true;break;
 case kAudioUnitProperty_MakeConnection:n=sizeof(AudioUnitConnection);write=true;break;
 default:return kAudioUnitErr_InvalidProperty;
 }
 if(size)*size=n;if(writable)*writable=write;return OK;
}
OSStatus get_property(void* self,AudioUnitPropertyID id,AudioUnitScope scope,AudioUnitElement element,void* out,UInt32* size){
 if(!out||!size)return kAudioUnitErr_InvalidPropertyValue;
 UInt32 needed=0;OSStatus status=property_info(self,id,scope,element,&needed,nullptr);if(status)return status;if(*size<needed){*size=needed;return kAudioUnitErr_InvalidPropertyValue;}*size=needed;auto* a=au(self);
 switch(id){
 case 65536:*(void**)out=a->processor;break;
 case kAudioUnitProperty_CocoaUI:{
  Dl_info location{};if(!dladdr((void*)&ag_editor_create,&location)||!location.dli_fname)return kAudioUnitErr_InvalidPropertyValue;
  auto path=std::filesystem::path(location.dli_fname).parent_path().parent_path().parent_path().string();
  auto* info=(AudioUnitCocoaViewInfo*)out;info->mCocoaAUViewBundleLocation=CFURLCreateFromFileSystemRepresentation(nullptr,(const UInt8*)path.data(),path.size(),true);
  info->mCocoaAUViewClass[0]=(CFStringRef)CFRetain(CFSTR("AGOpenCocoaViewFactory"));break;
 }
 case kAudioUnitProperty_ClassInfo:{
  UInt32 length=ag_insert_save(a->processor,nullptr,0);if(!length||length>16*1024*1024)return kAudioUnitErr_InvalidPropertyValue;
  std::vector<uint8_t> bytes(length);UInt32 got=ag_insert_save(a->processor,bytes.data(),length);if(!got||got>length)return kAudioUnitErr_InvalidPropertyValue;
  auto dict=CFDictionaryCreateMutable(nullptr,0,&kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks);auto data=CFDataCreate(nullptr,bytes.data(),got);CFDictionarySetValue(dict,CFSTR("data"),data);CFRelease(data);
  for(auto item: {std::pair<CFStringRef,int>{CFSTR("version"),0}, {CFSTR("type"),'aufx'},{CFSTR("subtype"),'AGrd'},{CFSTR("manufacturer"),'OpGd'}}){auto value=CFNumberCreate(nullptr,kCFNumberIntType,&item.second);CFDictionarySetValue(dict,item.first,value);CFRelease(value);}
  CFDictionarySetValue(dict,CFSTR("name"),CFSTR("OpenGridder"));*(CFPropertyListRef*)out=dict;break;
 }
 case kAudioUnitProperty_SampleRate:*(Float64*)out=a->format.mSampleRate;break;
 case kAudioUnitProperty_StreamFormat:*(AudioStreamBasicDescription*)out=a->format;break;
 case kAudioUnitProperty_OfflineRender:*(UInt32*)out=a->offline?1:0;break;
 case kAudioUnitProperty_ElementCount:*(UInt32*)out=1;break;
 case kAudioUnitProperty_MaximumFramesPerSlice:*(UInt32*)out=a->maximum;break;
 case kAudioUnitProperty_BypassEffect:*(UInt32*)out=ag_insert_get(a->processor,0)>=0.5;break;
 case kAudioUnitProperty_ShouldAllocateBuffer:*(UInt32*)out=1;break;
 case kAudioUnitProperty_RenderQuality:*(UInt32*)out=127;break;
 case kAudioUnitProperty_LastRenderError:*(OSStatus*)out=a->last_error;break;
 case kAudioUnitProperty_Latency:*(Float64*)out=ag_insert_latency(a->processor)/a->format.mSampleRate;break;
 case kAudioUnitProperty_TailTime:*(Float64*)out=0;break;
 case kAudioUnitProperty_PresentPreset:{if(scope!=kAudioUnitScope_Global)return kAudioUnitErr_InvalidScope;auto* pr=(AUPreset*)out;pr->presetNumber=0;pr->presetName=CFSTR("Default");break;}
 case kAudioUnitProperty_SetRenderCallback:*(AURenderCallbackStruct*)out=a->callback;break;
 case kAudioUnitProperty_MakeConnection:*(AudioUnitConnection*)out=a->connection;break;
 case kAudioUnitProperty_SupportedNumChannels:{auto* list=(AUChannelInfo*)out;list[0]={1,1};list[1]={2,2};break;}
 case kAudioUnitProperty_ParameterList:{auto* list=(AudioUnitParameterID*)out;for(UInt32 i=0;i<ag_insert_param_count();++i)list[i]=i;break;}
 case kAudioUnitProperty_ParameterInfo:{auto* p=(AudioUnitParameterInfo*)out;std::memset(p,0,sizeof(*p));char name[128];double lo,hi,def;int32_t steps;ag_insert_param_info(element,name,&lo,&hi,&def,&steps);std::strncpy(p->name,name,sizeof(p->name)-1);p->cfNameString=CFStringCreateWithCString(nullptr,name,kCFStringEncodingUTF8);p->flags=kAudioUnitParameterFlag_IsReadable|kAudioUnitParameterFlag_IsWritable|kAudioUnitParameterFlag_HasCFNameString|kAudioUnitParameterFlag_CFNameRelease;p->minValue=lo;p->maxValue=hi;p->defaultValue=lo+def*(hi-lo);p->unit=steps?kAudioUnitParameterUnit_Indexed:kAudioUnitParameterUnit_Generic;if(element==3||element==6)p->unit=kAudioUnitParameterUnit_Decibels;if(element==4)p->unit=kAudioUnitParameterUnit_Percent;if(element==0)p->unit=kAudioUnitParameterUnit_Boolean;break;}
 default:return kAudioUnitErr_InvalidProperty;
 }return OK;
}
OSStatus set_property(void* self,AudioUnitPropertyID id,AudioUnitScope scope,AudioUnitElement element,const void* input,UInt32 size){
 if(!input)return kAudioUnitErr_InvalidPropertyValue;auto* a=au(self);
 switch(id){
 case kAudioUnitProperty_ClassInfo:{if(size!=sizeof(CFPropertyListRef))break;auto dict=*(CFPropertyListRef*)input;if(!dict||CFGetTypeID(dict)!=CFDictionaryGetTypeID())break;auto data=(CFDataRef)CFDictionaryGetValue((CFDictionaryRef)dict,CFSTR("data"));if(!data||CFGetTypeID(data)!=CFDataGetTypeID())break;return ag_insert_load(a->processor,CFDataGetBytePtr(data),CFDataGetLength(data))?OK:kAudioUnitErr_InvalidPropertyValue;}
 case kAudioUnitProperty_StreamFormat:{
  if(size!=sizeof(AudioStreamBasicDescription)||a->initialized)return kAudioUnitErr_FormatNotSupported;auto f=*(const AudioStreamBasicDescription*)input;
  auto flags=kAudioFormatFlagsNativeFloatPacked|kAudioFormatFlagIsNonInterleaved;
  if(!std::isfinite(f.mSampleRate)||f.mSampleRate<8000||f.mSampleRate>768000||f.mFormatID!=kAudioFormatLinearPCM||f.mFormatFlags!=flags||f.mBitsPerChannel!=32||f.mBytesPerPacket!=4||f.mBytesPerFrame!=4||f.mFramesPerPacket!=1||f.mChannelsPerFrame<1||f.mChannelsPerFrame>2)return kAudioUnitErr_FormatNotSupported;
  a->format=f;a->changed(id,scope,element);return OK;
 }
 case kAudioUnitProperty_SampleRate:{if(size!=sizeof(Float64)||a->initialized||scope!=kAudioUnitScope_Global)break;double rate=*(const Float64*)input;if(!std::isfinite(rate)||rate<8000||rate>768000)break;a->format.mSampleRate=rate;a->changed(id,scope,element);return OK;}
 case kAudioUnitProperty_MaximumFramesPerSlice:{if(size!=4||a->initialized||scope!=kAudioUnitScope_Global)break;UInt32 n=*(const UInt32*)input;if(n<1||n>8192)break;a->maximum=n;a->changed(id,scope,element);return OK;}
 case kAudioUnitProperty_SetRenderCallback:if(size!=sizeof(AURenderCallbackStruct)||scope!=kAudioUnitScope_Input||element!=0)break;a->callback=*(const AURenderCallbackStruct*)input;a->connection={};return OK;
 case kAudioUnitProperty_MakeConnection:if(size!=sizeof(AudioUnitConnection))break;a->connection=*(const AudioUnitConnection*)input;a->callback={};return OK;
 case kAudioUnitProperty_BypassEffect:if(size!=4||scope!=kAudioUnitScope_Global)break;ag_insert_set(a->processor,0,*(const UInt32*)input?1:0);a->changed(id,scope,element);return OK;
 case kAudioUnitProperty_OfflineRender:if(size!=4||scope!=kAudioUnitScope_Global)break;a->offline=(*(const UInt32*)input!=0);a->changed(id,scope,element);return OK;
 case kAudioUnitProperty_PresentPreset:if(size!=sizeof(AUPreset)||scope!=kAudioUnitScope_Global)break;a->changed(id,scope,element);return OK;
 case kAudioUnitProperty_ShouldAllocateBuffer:case kAudioUnitProperty_RenderQuality:if(size==4)return OK;break;
 default:return kAudioUnitErr_InvalidProperty;
 }return kAudioUnitErr_InvalidPropertyValue;
}
OSStatus get_parameter(void* self,AudioUnitParameterID id,AudioUnitScope scope,AudioUnitElement element,AudioUnitParameterValue* value){if(scope!=0)return kAudioUnitErr_InvalidScope;if(element!=0)return kAudioUnitErr_InvalidElement;if(!value||id>=ag_insert_param_count())return kAudioUnitErr_InvalidParameter;double lo,hi;ag_insert_param_info(id,nullptr,&lo,&hi,nullptr,nullptr);*value=lo+ag_insert_get(au(self)->processor,id)*(hi-lo);return OK;}
OSStatus set_parameter(void* self,AudioUnitParameterID id,AudioUnitScope scope,AudioUnitElement element,AudioUnitParameterValue value,UInt32){if(scope!=0)return kAudioUnitErr_InvalidScope;if(element!=0)return kAudioUnitErr_InvalidElement;if(id>=ag_insert_param_count()||!std::isfinite(value))return kAudioUnitErr_InvalidParameter;auto* a=au(self);double lo,hi;ag_insert_param_info(id,nullptr,&lo,&hi,nullptr,nullptr);auto old=ag_insert_latency(a->processor);ag_insert_set(a->processor,id,hi>lo?(value-lo)/(hi-lo):0);if(old!=ag_insert_latency(a->processor))a->changed(kAudioUnitProperty_Latency);return OK;}
OSStatus render(void* self,AudioUnitRenderActionFlags* flags,const AudioTimeStamp* time,UInt32 bus,UInt32 frames,AudioBufferList* data){
 auto* a=au(self);if(!a->initialized)return kAudioUnitErr_Uninitialized;if(bus!=0)return kAudioUnitErr_InvalidElement;if(!data||!time||!flags)return kAudioUnitErr_InvalidPropertyValue;if(frames>a->maximum)return kAudioUnitErr_TooManyFramesToProcess;
 UInt32 channels=a->format.mChannelsPerFrame;if(data->mNumberBuffers!=channels)return kAudioUnitErr_FormatNotSupported;
 for(UInt32 ch=0;ch<channels;ch++)if(data->mBuffers[ch].mData&&data->mBuffers[ch].mDataByteSize<frames*4)return kAudioUnitErr_InvalidPropertyValue;
 struct Buffers{UInt32 count;AudioBuffer buffers[2];} in{};in.count=channels;
 for(UInt32 ch=0;ch<channels;ch++){std::fill(a->input[ch],a->input[ch]+frames,0);in.buffers[ch]={1,frames*4,a->input[ch]};}
 for(const auto& n:a->notifies){auto pre=*flags|kAudioUnitRenderAction_PreRender;n.proc(n.data,&pre,time,bus,frames,data);}
 OSStatus status=OK;
 if(a->callback.inputProc)status=a->callback.inputProc(a->callback.inputProcRefCon,flags,time,0,frames,(AudioBufferList*)&in);
 else if(a->connection.sourceAudioUnit)status=AudioUnitRender(a->connection.sourceAudioUnit,flags,time,a->connection.sourceOutputNumber,frames,(AudioBufferList*)&in);
 if(status){a->last_error=status;return status;}
 const float* inputs[2]{};float* outputs[2]{};
 for(UInt32 ch=0;ch<channels;ch++){inputs[ch]=(const float*)in.buffers[ch].mData;outputs[ch]=a->output[ch];}
 if(!ag_insert_process(a->processor,inputs,outputs,channels,frames,a->offline))return kAudioUnitErr_CannotDoInCurrentContext;
 bool silence=true;for(UInt32 ch=0;ch<channels;ch++){auto& b=data->mBuffers[ch];if(!b.mData)b.mData=a->output[ch];else std::memcpy(b.mData,a->output[ch],frames*4);b.mDataByteSize=frames*4;b.mNumberChannels=1;for(UInt32 i=0;i<frames;i++)silence=silence&&a->output[ch][i]==0;}
 *flags&=~kAudioUnitRenderAction_OutputIsSilence;if(silence)*flags|=kAudioUnitRenderAction_OutputIsSilence;
 for(const auto& n:a->notifies){auto post=*flags|kAudioUnitRenderAction_PostRender;n.proc(n.data,&post,time,bus,frames,data);}a->last_error=OK;return OK;
}
OSStatus add_listener(void* self,AudioUnitPropertyID id,AudioUnitPropertyListenerProc proc,void* data){if(!proc)return kAudioUnitErr_InvalidPropertyValue;au(self)->listeners.push_back({id,proc,data});return OK;}
OSStatus remove_listener_data(void* self,AudioUnitPropertyID id,AudioUnitPropertyListenerProc proc,void* data){auto& ls=au(self)->listeners;ls.erase(std::remove_if(ls.begin(),ls.end(),[=](const Listener& l){return l.id==id&&l.proc==proc&&l.data==data;}),ls.end());return OK;}
OSStatus remove_listener(void* self,AudioUnitPropertyID id,AudioUnitPropertyListenerProc proc){auto& ls=au(self)->listeners;ls.erase(std::remove_if(ls.begin(),ls.end(),[=](const Listener& l){return l.id==id&&l.proc==proc;}),ls.end());return OK;}
OSStatus add_notify(void* self,AURenderCallback proc,void* data){if(!proc)return kAudioUnitErr_InvalidPropertyValue;au(self)->notifies.push_back({proc,data});return OK;}
OSStatus remove_notify(void* self,AURenderCallback proc,void* data){auto& ns=au(self)->notifies;ns.erase(std::remove_if(ns.begin(),ns.end(),[=](const Notify& n){return n.proc==proc&&n.data==data;}),ns.end());return OK;}
OSStatus schedule_parameters(void* self,const AudioUnitParameterEvent* events,UInt32 count){
 if(!events)return kAudioUnitErr_InvalidPropertyValue;
 for(UInt32 i=0;i<count;i++){
  const auto& ev=events[i];
  if(ev.eventType==kParameterEvent_Immediate)set_parameter(self,ev.parameter,ev.scope,ev.element,ev.eventValues.immediate.value,0);
 }
 return OK;
}
AudioComponentMethod lookup(SInt16 selector){switch(selector){
#define METHOD(s,f) case s:return reinterpret_cast<AudioComponentMethod>(f)
 METHOD(kAudioUnitInitializeSelect,initialize);METHOD(kAudioUnitUninitializeSelect,uninitialize);METHOD(kAudioUnitResetSelect,reset);METHOD(kAudioUnitGetPropertyInfoSelect,property_info);METHOD(kAudioUnitGetPropertySelect,get_property);METHOD(kAudioUnitSetPropertySelect,set_property);METHOD(kAudioUnitGetParameterSelect,get_parameter);METHOD(kAudioUnitSetParameterSelect,set_parameter);METHOD(kAudioUnitScheduleParametersSelect,schedule_parameters);METHOD(kAudioUnitRenderSelect,render);METHOD(kAudioUnitAddPropertyListenerSelect,add_listener);METHOD(kAudioUnitRemovePropertyListenerSelect,remove_listener);METHOD(kAudioUnitRemovePropertyListenerWithUserDataSelect,remove_listener_data);METHOD(kAudioUnitAddRenderNotifySelect,add_notify);METHOD(kAudioUnitRemoveRenderNotifySelect,remove_notify);
#undef METHOD
 default:return nullptr;}}
}
extern "C" void* ag_au_factory(const void*){auto* a=new Au();a->api.Open=open;a->api.Close=close;a->api.Lookup=lookup;a->api.reserved=nullptr;return &a->api;}
