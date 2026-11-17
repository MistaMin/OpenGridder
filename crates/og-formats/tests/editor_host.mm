#import <Cocoa/Cocoa.h>
#include "pluginterfaces/base/ipluginbase.h"
#include "pluginterfaces/vst/ivstcomponent.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/gui/iplugview.h"
#include <dlfcn.h>
#include <cstdio>
#include <cstdlib>
#include <cmath>
using namespace Steinberg;using namespace Steinberg::Vst;
void check(bool v,const char* message){if(!v){std::fprintf(stderr,"FAIL %s\n",message);std::exit(1);}}
struct Handler:IComponentHandler{
 int begin=0,perform=0,end=0,restart=0,restartFlags=0;
 uint32 addRef()override{return 1;}uint32 release()override{return 1;}
 tresult queryInterface(const TUID,void** out)override{*out=nullptr;return kNoInterface;}
 tresult beginEdit(ParamID)override{begin++;return kResultOk;}
 tresult performEdit(ParamID,ParamValue)override{perform++;return kResultOk;}
 tresult endEdit(ParamID)override{end++;return kResultOk;}
 tresult restartComponent(int32 flags)override{restart++;restartFlags|=flags;return kResultOk;}
};
void snapshot(NSView* view,NSString* path){
 [view setNeedsDisplay:YES];NSBitmapImageRep* image=[view bitmapImageRepForCachingDisplayInRect:view.bounds];
 [view cacheDisplayInRect:view.bounds toBitmapImageRep:image];
 NSData* png=[image representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
 check([png writeToFile:path atomically:YES],"write screenshot");
}
NSEvent* mouse(NSEventType type,double x,double y,NSWindow* window,NSInteger clicks=1){
 return[NSEvent mouseEventWithType:type location:NSMakePoint(x,y) modifierFlags:0 timestamp:0 windowNumber:window.windowNumber context:nil eventNumber:1 clickCount:clicks pressure:1];
}
int main(int argc,char**argv){@autoreleasepool{
 check(argc==3,"module and output folder");[NSApplication sharedApplication];[NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
 void* module=dlopen(argv[1],RTLD_NOW|RTLD_LOCAL);check(module,"load module");
 auto factory=((IPluginFactory*(*)())dlsym(module,"GetPluginFactory"))();PClassInfo info{};factory->getClassInfo(0,&info);
 IComponent* component=nullptr;check(factory->createInstance(info.cid,IComponent::iid,(void**)&component)==0,"create component");
 component->initialize(nullptr);IEditController* control=nullptr;component->queryInterface(IEditController::iid,(void**)&control);
 Handler handler;control->setComponentHandler(&handler);
 IPlugView* plugview=control->createView("editor");check(plugview,"native editor");
 ViewRect rect{};check(plugview->getSize(&rect)==0&&rect.right==864&&rect.bottom==420,"editor size 864x420");
 NSWindow* window=[[NSWindow alloc]initWithContentRect:NSMakeRect(0,0,864,420) styleMask:NSWindowStyleMaskBorderless backing:NSBackingStoreBuffered defer:NO];
 window.releasedWhenClosed=NO;[window orderOut:nil];
 check(plugview->attached((__bridge void*)window.contentView,kPlatformTypeNSView)==0,"attach Cocoa view");
 NSView* view=window.contentView.subviews.firstObject;check(view,"child exists");
 snapshot(view,[[NSString stringWithUTF8String:argv[2]]stringByAppendingPathComponent:@"editor-before.png"]);
 view=view.subviews.firstObject;check(view,"interactive editor exists");
 const double controls[][3]={{351,285,3},{481,285,4},{351,122,5},{481,122,6},{224,360,0},{226,284,2},{323,83,23},{379,83,24}};
 int failures=0;
 for(const auto& hit:controls){
  auto id=(uint32)hit[2];double before=control->getParamNormalized(id);
  bool knob=id>=3&&id<=6;double dy=knob?(before<0.5?30:-30):0;
  [view mouseDown:mouse(NSEventTypeLeftMouseDown,hit[0],hit[1],window)];
  if(knob)[view mouseDragged:mouse(NSEventTypeLeftMouseDragged,hit[0],hit[1]+dy,window)];
  [view mouseUp:mouse(NSEventTypeLeftMouseUp,hit[0],hit[1]+dy,window)];
  double after=control->getParamNormalized(id);
  if(std::abs(after-before)<=1e-6){std::fprintf(stderr,"FAIL UI parameter %u did not change host value (%.6f)\n",id,before);failures++;}
  else std::printf("PASS UI parameter %u %.6f -> %.6f\n",id,before,after);
 }
 if(handler.begin!=8||handler.end!=8||handler.perform<8){std::fprintf(stderr,"FAIL gestures begin=%d perform=%d end=%d, expected 8 paired edits\n",handler.begin,handler.perform,handler.end);failures++;}
 if(handler.restart<1||(handler.restartFlags&kLatencyChanged)==0){std::fprintf(stderr,"FAIL latency restart count=%d flags=%d\n",handler.restart,handler.restartFlags);failures++;}
 snapshot(view,[[NSString stringWithUTF8String:argv[2]]stringByAppendingPathComponent:@"editor-after.png"]);
 plugview->removed();check(window.contentView.subviews.count==0,"editor removed");
 check(plugview->attached((__bridge void*)window.contentView,kPlatformTypeNSView)==0,"editor reopens");
 plugview->removed();plugview->release();control->setComponentHandler(nullptr);control->release();component->terminate();component->release();factory->release();[window close];
 check(failures==0,"native editor control and host notification sweep");
 std::printf("PASS native editor attach, 8 bindings, gestures, latency restart, remove/reopen; hidden window only\n");
 }return 0;}
