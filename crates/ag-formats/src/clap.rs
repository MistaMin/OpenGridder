//! Use OpenPluginSDK's CLAP bridge, including parameter automation, MIDI,
//! audio ports, latency, and state. Keep one ABI implementation in the SDK.
openplugin::targets::export_clap!(ag_plugin::AudioGridder);

#[cfg(test)]
mod tests {
    use super::clap_entry;
    use openplugin::targets::clap_abi::*;
    use std::{ffi::c_void, ptr};

    #[test]
    fn clap_factory_exposes_console_automation_and_restores_state() {
        unsafe {
            assert!(clap_entry.init.unwrap()(c"/tmp/AudioGridder.clap".as_ptr()));
            let factory=(clap_entry.get_factory.unwrap()(c"clap.plugin-factory".as_ptr())) as *const clap_plugin_factory;
            assert!(!factory.is_null());
            let desc=(*factory).get_plugin_descriptor.unwrap()(factory,0);
            let plugin=(*factory).create_plugin.unwrap()(factory,ptr::null(),(*desc).id);
            assert!(!plugin.is_null());assert!((*plugin).init.unwrap()(plugin));
            let params=(*plugin).get_extension.unwrap()(plugin,c"clap.params".as_ptr()) as *const clap_plugin_params;
            assert!(!params.is_null());assert_eq!((*params).count.unwrap()(plugin),25);
            let mut info=std::mem::zeroed::<clap_param_info>();
            assert!((*params).get_info.unwrap()(plugin,3,&mut info));
            assert_eq!((info.min_value,info.max_value,info.default_value),(-24.0,24.0,0.0));
            let event=clap_event_param_value {
                header:clap_event_header{size:std::mem::size_of::<clap_event_param_value>()as u32,time:0,space_id:0,type_:EVENT_PARAM_VALUE,flags:0},
                param_id:3,cookie:ptr::null_mut(),note_id:-1,port_index:-1,channel:-1,key:-1,value:6.0,
            };
            let list=clap_input_events{ctx:&event as *const _ as *mut c_void,size:Some(size),get:Some(get)};
            (*params).flush.unwrap()(plugin,&list,ptr::null());
            let mut value=0.;assert!((*params).get_value.unwrap()(plugin,3,&mut value));assert_eq!(value,6.0);
            let state=(*plugin).get_extension.unwrap()(plugin,c"clap.state".as_ptr())as *const clap_plugin_state;
            assert!(!state.is_null());
            let mut bytes=Vec::<u8>::new();let out=clap_ostream{ctx:&mut bytes as *mut _ as *mut c_void,write:Some(write)};
            assert!((*state).save.unwrap()(plugin,&out));assert!(bytes.len()>20);
            let event=clap_event_param_value{value:-12.0,..event};let list=clap_input_events{ctx:&event as *const _ as *mut c_void,..list};
            (*params).flush.unwrap()(plugin,&list,ptr::null());
            let mut input=(&bytes[..],0usize);let stream=clap_istream{ctx:&mut input as *mut _ as *mut c_void,read:Some(read)};
            assert!((*state).load.unwrap()(plugin,&stream));assert!((*params).get_value.unwrap()(plugin,3,&mut value));assert_eq!(value,6.0);
            (*plugin).destroy.unwrap()(plugin);
        }
    }
    unsafe extern "C" fn size(_: *const clap_input_events)->u32{1}
    unsafe extern "C" fn get(list:*const clap_input_events,i:u32)->*const clap_event_header{if i==0{(*list).ctx as *const clap_event_header}else{ptr::null()}}
    unsafe extern "C" fn write(s:*const clap_ostream,data:*const c_void,n:u64)->i64{let out=&mut *((*s).ctx as *mut Vec<u8>);out.extend_from_slice(std::slice::from_raw_parts(data as *const u8,n as usize));n as i64}
    unsafe extern "C" fn read(s:*const clap_istream,data:*mut c_void,n:u64)->i64{let input=&mut *((*s).ctx as *mut (&[u8],usize));let count=(n as usize).min(input.0.len()-input.1).min(7);ptr::copy_nonoverlapping(input.0.as_ptr().add(input.1),data as *mut u8,count);input.1+=count;count as i64}
}
