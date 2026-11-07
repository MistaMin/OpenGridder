// Retain the Cocoa editor across disposal of its AudioUnit, then exercise every
// callback that used to dereference the freed unit/processor. No visible window.
#import <Cocoa/Cocoa.h>
#import <AudioUnit/AUCocoaUIView.h>
#import <AudioToolbox/AudioToolbox.h>
#include <dlfcn.h>
#include <cstdio>
#include <cstdlib>
static void require(bool value,const char* text){if(!value){std::fprintf(stderr,"FAIL: %s\n",text);std::exit(1);}}
int main(int argc,char** argv){@autoreleasepool{
 require(argc==2,"module path");[NSApplication sharedApplication];[NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
 void* lib=dlopen(argv[1],RTLD_NOW|RTLD_LOCAL);require(lib,"load module");
 auto factory=(AudioComponentFactoryFunction)dlsym(lib,"AudioComponentFactoryFunction");require(factory,"factory");
 auto alive=(bool(*)(void*))dlsym(lib,"ag_insert_alive");require(alive,"host lifetime query");
 AudioComponentDescription desc{'aufx','agLF','OpGd',0,0};auto component=AudioComponentRegister(&desc,CFSTR("OpenGrid: lifetime fixture"),0x20000,factory);require(component,"register fixture");
 AudioUnit unit=nullptr;require(AudioComponentInstanceNew(component,&unit)==0,"instantiate");
 void* processor=nullptr;UInt32 size=sizeof(processor);require(AudioUnitGetProperty(unit,65536,0,0,&processor,&size)==0,"processor property");
 Class cls=NSClassFromString(@"AGOpenCocoaViewFactory");require(cls!=nil,"Cocoa factory class");id<AUCocoaUIBase> editorFactory=[[cls alloc]init];
 NSView* view=[editorFactory uiViewForAudioUnit:unit withSize:NSMakeSize(720,420)];require(view!=nil,"Cocoa editor");
 require(alive(processor),"host alive before disposal");
 require(AudioComponentInstanceDispose(unit)==0,"dispose unit while retaining view");require(!alive(processor),"view retains processor storage with dead host flag");
 // Drawing, timer, click, wheel and deallocation must not call the disposed AU.
 [view drawRect:view.bounds];
 NSEvent* down=[NSEvent mouseEventWithType:NSEventTypeLeftMouseDown location:NSMakePoint(100,170) modifierFlags:0 timestamp:0 windowNumber:0 context:nil eventNumber:1 clickCount:1 pressure:1];
 [view mouseDown:down];[view mouseDragged:down];[view mouseUp:down];[view scrollWheel:down];
 [[NSRunLoop currentRunLoop]runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.2]];
 view=nil;editorFactory=nil;
 std::puts("PASS AU editor retained across AudioComponentInstanceDispose; draw/input/timer/dealloc safe");
 }return 0;}
