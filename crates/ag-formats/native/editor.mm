#import <Cocoa/Cocoa.h>
#import <AudioUnit/AUCocoaUIView.h>
#import <AudioToolbox/AudioToolbox.h>
#include "bridge.h"
#include "editor.h"
#include <vector>
#include <cmath>
@interface AGOpenEditor:NSView {
    void* _processor;
    void* _owner;
    AgEdit _edit;
    AgGesture _gesture;
    NSTimer* _timer;
    uint64_t _revision;
    int _drag;
    double _start;
    CGFloat _startY;
    std::vector<uint32_t> _pixels;
}
-(id)initWithProcessor:(void*)processor owner:(void*)owner edit:(AgEdit)edit gesture:(AgGesture)gesture;
@end
@implementation AGOpenEditor
-(id)initWithProcessor:(void*)processor owner:(void*)owner edit:(AgEdit)edit gesture:(AgGesture)gesture{
 if((self=[super initWithFrame:NSMakeRect(0,0,720,420)])){
  if(!ag_insert_retain(processor))return nil;
  _processor=processor;_owner=owner;_edit=edit;_gesture=gesture;_drag=-1;_revision=0;
  [self setAccessibilityLabel:@"AudioGridder console"];
  __weak AGOpenEditor* weak=self;
  _timer=[NSTimer scheduledTimerWithTimeInterval:1.0/15.0 repeats:YES block:^(NSTimer*){AGOpenEditor* view=weak;if(view&&view->_processor&&ag_insert_alive(view->_processor)&&ag_insert_revision(view->_processor)!=view->_revision)[view setNeedsDisplay:YES];}];
 }
 return self;
}
-(void)shutdown{[_timer invalidate];_timer=nil;if(_processor&&ag_insert_alive(_processor)&&_drag>=0&&_gesture)_gesture(_owner,_drag,false);_drag=-1;if(_processor)ag_insert_release(_processor);_processor=nullptr;_owner=nullptr;_edit=nullptr;_gesture=nullptr;}
-(void)dealloc{[self shutdown];}
-(BOOL)isFlipped{return YES;}
-(BOOL)isOpaque{return YES;}
-(BOOL)acceptsFirstResponder{return YES;}
-(void)drawRect:(NSRect)dirty{
 (void)dirty;if(!_processor||!ag_insert_alive(_processor))return;CGFloat scale=self.window.backingScaleFactor;if(scale<1)scale=1;if(scale>3)scale=3;
 uint32_t width=720*scale,height=420*scale;_pixels.resize((size_t)width*height);
 if(!ag_insert_render(_processor,_pixels.data(),width,height))return;
 CGColorSpaceRef space=CGColorSpaceCreateDeviceRGB();CGDataProviderRef provider=CGDataProviderCreateWithData(nullptr,_pixels.data(),_pixels.size()*4,nullptr);
 CGImageRef image=CGImageCreate(width,height,8,32,width*4,space,kCGBitmapByteOrder32Little|kCGImageAlphaNoneSkipFirst,provider,nullptr,false,kCGRenderingIntentDefault);
 if(image){NSImage* picture=[[NSImage alloc]initWithCGImage:image size:NSMakeSize(720,420)];[picture drawInRect:self.bounds fromRect:NSZeroRect operation:NSCompositingOperationCopy fraction:1 respectFlipped:YES hints:nil];CGImageRelease(image);}
 CGDataProviderRelease(provider);CGColorSpaceRelease(space);_revision=ag_insert_revision(_processor);
}
-(int)parameterAt:(NSPoint)point{
 double x=point.x*720/self.bounds.size.width,y=point.y*420/self.bounds.size.height;
 const double knobs[][3]={{100,250,3},{208.333,250,5},{325,280,4},{583.333,250,6}};
 for(const auto& k:knobs)if(std::hypot(x-k[0],y-k[1])<45)return k[2];
 if(y>30&&y<62){if(x>35&&x<155)return 0;if(x>=155&&x<292)return 1;if(x>=292&&x<455)return 2;}
 if(y>340&&y<380){if(x>35&&x<200)return 7;if(x>=200&&x<360)return 8;}
 return -1;
}
-(void)setParameter:(int)index value:(double)value{
 if(!_processor||!ag_insert_alive(_processor))return;
 value=std::max(0.0,std::min(1.0,value));if(_edit)_edit(_owner,index,value);else ag_insert_set(_processor,index,value);[self setNeedsDisplay:YES];
}
-(void)mouseDown:(NSEvent*)event{
 if(!_processor||!ag_insert_alive(_processor))return;
 int i=[self parameterAt:[self convertPoint:event.locationInWindow fromView:nil]];if(i<0)return;
 _drag=i;_start=ag_insert_get(_processor,i);_startY=event.locationInWindow.y;
 if(_gesture)_gesture(_owner,i,true);
 if(event.clickCount==2){double value;ag_insert_param_info(i,nullptr,nullptr,nullptr,&value,nullptr);[self setParameter:i value:value];}
 else if(i==0)[self setParameter:i value:_start>=0.5?0:1];
 else if(i==1||i==2||i==7||i==8){double lo,hi;ag_insert_param_info(i,nullptr,&lo,&hi,nullptr,nullptr);double step=1.0/(hi-lo);double next=_start+((event.modifierFlags&NSEventModifierFlagShift)?-step:step);if(next>1.00001)next=0;if(next<0)next=1;[self setParameter:i value:next];}
}
-(void)mouseDragged:(NSEvent*)event{if(!_processor||!ag_insert_alive(_processor))return;if(_drag<0||_drag==0||_drag==1||_drag==2||_drag==7||_drag==8)return;double delta=(event.locationInWindow.y-_startY)/180.0;if(event.modifierFlags&NSEventModifierFlagShift)delta*=0.1;[self setParameter:_drag value:_start+delta];}
-(void)mouseUp:(NSEvent*)event{(void)event;if(_processor&&ag_insert_alive(_processor)&&_drag>=0&&_gesture)_gesture(_owner,_drag,false);_drag=-1;}
-(void)scrollWheel:(NSEvent*)event{if(!_processor||!ag_insert_alive(_processor))return;int i=[self parameterAt:[self convertPoint:event.locationInWindow fromView:nil]];if(i<0)return;double lo,hi;int32_t steps;ag_insert_param_info(i,nullptr,&lo,&hi,nullptr,&steps);double step=steps?1.0/steps:0.01;double value=ag_insert_get(_processor,i)+event.scrollingDeltaY*step;if(_gesture)_gesture(_owner,i,true);[self setParameter:i value:value];if(_gesture)_gesture(_owner,i,false);}
@end
extern "C" void* ag_editor_create(void* processor,void* owner,AgEdit edit,AgGesture gesture){AGOpenEditor* view=[[AGOpenEditor alloc]initWithProcessor:processor owner:owner edit:edit gesture:gesture];return (__bridge_retained void*)view;}
extern "C" void ag_editor_release(void* view){if(view){AGOpenEditor* owned=(__bridge_transfer AGOpenEditor*)view;[owned shutdown];[owned removeFromSuperview];}}
@interface AGOpenCocoaViewFactory:NSObject<AUCocoaUIBase>
@end
static void auEdit(void* owner,uint32_t i,double value){double lo,hi;ag_insert_param_info(i,nullptr,&lo,&hi,nullptr,nullptr);AudioUnitSetParameter((AudioUnit)owner,i,kAudioUnitScope_Global,0,lo+value*(hi-lo),0);AudioUnitParameter p{(AudioUnit)owner,i,kAudioUnitScope_Global,0};AUParameterListenerNotify(nullptr,nullptr,&p);}
static void auGesture(void* owner,uint32_t i,bool start){AudioUnitEvent event{};event.mEventType=start?kAudioUnitEvent_BeginParameterChangeGesture:kAudioUnitEvent_EndParameterChangeGesture;event.mArgument.mParameter={(AudioUnit)owner,i,kAudioUnitScope_Global,0};AUEventListenerNotify(nullptr,nullptr,&event);}
@implementation AGOpenCocoaViewFactory
-(unsigned)interfaceVersion{return 0;}
-(NSView*)uiViewForAudioUnit:(AudioUnit)unit withSize:(NSSize)size{(void)size;void* processor=nullptr;UInt32 bytes=sizeof(processor);if(AudioUnitGetProperty(unit,65536,0,0,&processor,&bytes)!=0||!processor)return nil;void* raw=ag_editor_create(processor,unit,auEdit,auGesture);return (__bridge_transfer NSView*)raw;}
@end

extern "C" bool ag_editor_attach(void* raw,void* parent){if(!raw||!parent)return false;[(__bridge NSView*)parent addSubview:(__bridge NSView*)raw];return true;}
