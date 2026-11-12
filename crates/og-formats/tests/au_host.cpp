// Load through Apple's AudioComponent API, without changing plugin registrations on disk.
#include <AudioToolbox/AudioToolbox.h>
#include <AudioUnit/AudioUnit.h>
#include <dlfcn.h>
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <cstring>
#include <vector>
void check(bool v,const char* message){if(!v){std::fprintf(stderr,"FAIL: %s\n",message);std::exit(1);}}
OSStatus input(void*,AudioUnitRenderActionFlags*,const AudioTimeStamp* t,UInt32,UInt32 frames,AudioBufferList* data){for(UInt32 ch=0;ch<data->mNumberBuffers;ch++){float* p=(float*)data->mBuffers[ch].mData;for(UInt32 n=0;n<frames;n++)p[n]=(t->mSampleTime==0&&n==0)?0.25f:0;}return 0;}
int main(int argc,char** argv){check(argc==2,"pass module binary");void* lib=dlopen(argv[1],RTLD_NOW|RTLD_LOCAL);if(!lib)std::fprintf(stderr,"%s\n",dlerror());check(lib,"load module");auto factory=(AudioComponentFactoryFunction)dlsym(lib,"AudioComponentFactoryFunction");check(factory,"AU factory symbol");AudioComponentDescription desc{'aufx','agQA','OpGd',0,0};auto component=AudioComponentRegister(&desc,CFSTR("OpenGrid: AudioGridder Test"),0x20000,factory);check(component,"register test component");
 for(UInt32 channels:{1,2})for(double rate:{44100.,48000.,96000.}){
 AudioUnit unit=nullptr;OSStatus status=AudioComponentInstanceNew(component,&unit);if(status)std::printf("instance status %d\n",status);check(status==0&&unit,"create AudioUnit instance");
 AudioStreamBasicDescription format{};format.mSampleRate=rate;format.mFormatID=kAudioFormatLinearPCM;format.mFormatFlags=kAudioFormatFlagsNativeFloatPacked|kAudioFormatFlagIsNonInterleaved;format.mBytesPerPacket=format.mBytesPerFrame=4;format.mFramesPerPacket=1;format.mChannelsPerFrame=channels;format.mBitsPerChannel=32;
 check(AudioUnitSetProperty(unit,kAudioUnitProperty_StreamFormat,kAudioUnitScope_Input,0,&format,sizeof(format))==0,"input format");check(AudioUnitSetProperty(unit,kAudioUnitProperty_StreamFormat,kAudioUnitScope_Output,0,&format,sizeof(format))==0,"output format");
 UInt32 count=0;check(AudioUnitGetPropertyInfo(unit,kAudioUnitProperty_ParameterList,0,0,&count,nullptr)==0&&count==25*4,"all console and slot parameters available");
 check(AudioUnitSetParameter(unit,3,0,0,6.,0)==0,"+6 dB input");check(AudioUnitSetParameter(unit,2,0,0,0.,0)==0,"latency profile");AURenderCallbackStruct cb{input,nullptr};check(AudioUnitSetProperty(unit,kAudioUnitProperty_SetRenderCallback,kAudioUnitScope_Input,0,&cb,sizeof(cb))==0,"connect input callback");
 check(AudioUnitInitialize(unit)==0,"initialize");Float64 latency=0;count=sizeof(latency);check(AudioUnitGetProperty(unit,kAudioUnitProperty_Latency,0,0,&latency,&count)==0,"latency property");check(std::abs(latency*rate-128.)<1e-6,"128-sample delay");
 struct Buffers{UInt32 count;AudioBuffer b[2];} data{};data.count=channels;float left[512]={},right[512]={};data.b[0]={1,sizeof(left),left};data.b[1]={1,sizeof(right),right};AudioTimeStamp stamp{};stamp.mFlags=kAudioTimeStampSampleTimeValid;stamp.mSampleTime=0;AudioUnitRenderActionFlags flags=0;
 status=AudioUnitRender(unit,&flags,&stamp,0,512,(AudioBufferList*)&data);if(status)std::printf("render status %d\n",status);check(status==0,"render through AudioToolbox");
 for(UInt32 ch=0;ch<channels;ch++)for(int n=0;n<512;n++){float value=((float*)data.b[ch].mData)[n];check(std::isfinite(value),"finite output");check(std::abs(value-(n==128?0.25*std::pow(10.,6./20.):0))<1e-5,"impulse gain and delay");}
 CFPropertyListRef state=nullptr;count=sizeof(state);check(AudioUnitGetProperty(unit,kAudioUnitProperty_ClassInfo,0,0,&state,&count)==0,"save class state");check(AudioUnitSetParameter(unit,3,0,0,-12.,0)==0,"alter gain");check(AudioUnitSetProperty(unit,kAudioUnitProperty_ClassInfo,0,0,&state,sizeof(state))==0,"restore class state");AudioUnitParameterValue value=0;check(AudioUnitGetParameter(unit,3,0,0,&value)==0&&std::abs(value-6.)<1e-5,"restored gain");CFRelease(state);
 AudioUnitUninitialize(unit);AudioComponentInstanceDispose(unit);std::printf("PASS AU %u channels %.0f Hz gain +6 dB latency 128\n",channels,rate);
 }
 // The process-wide AudioComponent registry owns the factory until exit.
 return 0;
}
