//! Audio Unit v2 hosting through Apple's C ABI. No SDK sources or C++ runtime.
//!
//! Instances are exclusively owned by the server's serialized processor. They are
//! Send (the command/audio workers take turns) but deliberately not Sync.
use crate::au::{AudioComponentDescription, K_CF_STRING_ENCODING_UTF8};
use crate::{HostedPlugin, HostedPluginInstance};
use og_common::{
    message::{MidiEvent, PositionInfo},
    server_plugin::{PluginParameter, ServerPlugin},
};
use std::{
    ffi::{c_void, CStr},
    ptr,
};

type Unit = *mut c_void;
type CF = *const c_void;
const MAX_CHANNELS: usize = 32;
const GLOBAL: u32 = 0;
const INPUT: u32 = 1;
const OUTPUT: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Format {
    rate: f64,
    id: u32,
    flags: u32,
    packet_bytes: u32,
    packet_frames: u32,
    frame_bytes: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct AudioBuffer {
    channels: u32,
    bytes: u32,
    data: *mut c_void,
}
#[repr(C)]
struct BufferList {
    count: u32,
    buffers: [AudioBuffer; MAX_CHANNELS],
}
impl BufferList {
    fn new(channels: usize) -> Self {
        Self {
            count: channels as u32,
            buffers: [AudioBuffer {
                channels: 1,
                bytes: 0,
                data: ptr::null_mut(),
            }; MAX_CHANNELS],
        }
    }
}
#[repr(C)]
#[derive(Default)]
struct SmpteTime {
    subframes: i16,
    divisor: i16,
    counter: u32,
    kind: u32,
    flags: u32,
    hours: i16,
    minutes: i16,
    seconds: i16,
    frames: i16,
}
#[repr(C)]
#[derive(Default)]
struct TimeStamp {
    sample: f64,
    host: u64,
    scalar: f64,
    word: u64,
    smpte: SmpteTime,
    flags: u32,
    reserved: u32,
}
type RenderFn =
    unsafe extern "C" fn(*mut c_void, *mut u32, *const TimeStamp, u32, u32, *mut BufferList) -> i32;
#[repr(C)]
struct RenderCallback {
    callback: RenderFn,
    context: *mut c_void,
}
#[repr(C)]
struct ParameterInfo {
    name: [i8; 52],
    unit_name: CF,
    clump: u32,
    cf_name: CF,
    unit: u32,
    min: f32,
    max: f32,
    default: f32,
    flags: u32,
}

#[link(name = "AudioToolbox", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn AudioComponentFindNext(
        previous: Unit,
        description: *const AudioComponentDescription,
    ) -> Unit;
    fn AudioComponentInstanceNew(component: Unit, instance: *mut Unit) -> i32;
    fn AudioComponentInstanceDispose(unit: Unit) -> i32;
    fn AudioUnitInitialize(unit: Unit) -> i32;
    fn AudioUnitUninitialize(unit: Unit) -> i32;
    fn AudioUnitGetPropertyInfo(
        unit: Unit,
        property: u32,
        scope: u32,
        element: u32,
        size: *mut u32,
        writable: *mut u8,
    ) -> i32;
    fn AudioUnitGetProperty(
        unit: Unit,
        property: u32,
        scope: u32,
        element: u32,
        value: *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn AudioUnitSetProperty(
        unit: Unit,
        property: u32,
        scope: u32,
        element: u32,
        value: *const c_void,
        size: u32,
    ) -> i32;
    fn AudioUnitGetParameter(
        unit: Unit,
        parameter: u32,
        scope: u32,
        element: u32,
        value: *mut f32,
    ) -> i32;
    fn AudioUnitSetParameter(
        unit: Unit,
        parameter: u32,
        scope: u32,
        element: u32,
        value: f32,
        offset: u32,
    ) -> i32;
    fn AudioUnitRender(
        unit: Unit,
        flags: *mut u32,
        time: *const TimeStamp,
        bus: u32,
        frames: u32,
        buffers: *mut BufferList,
    ) -> i32;
    fn MusicDeviceMIDIEvent(
        unit: Unit,
        status: u32,
        data1: u32,
        data2: u32,
        offset_sample_frame: u32,
    ) -> i32;
    fn CFRelease(value: CF);
    fn CFStringGetLength(value: CF) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(value: CF, bytes: *mut i8, size: isize, encoding: u32) -> u8;
    fn CFPropertyListCreateData(
        allocator: CF,
        value: CF,
        format: isize,
        options: usize,
        error: *mut CF,
    ) -> CF;
    fn CFPropertyListCreateWithData(
        allocator: CF,
        data: CF,
        options: usize,
        format: *mut isize,
        error: *mut CF,
    ) -> CF;
    fn CFDataCreate(allocator: CF, bytes: *const u8, size: isize) -> CF;
    fn CFDataGetLength(data: CF) -> isize;
    fn CFDataGetBytePtr(data: CF) -> *const u8;
}

fn status(code: i32, operation: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("Audio Unit {operation} failed (OSStatus {code})"))
    }
}
unsafe fn get<T: Copy>(unit: Unit, property: u32, scope: u32, element: u32) -> Result<T, String> {
    let mut value = std::mem::MaybeUninit::<T>::zeroed();
    let mut size = std::mem::size_of::<T>() as u32;
    status(
        AudioUnitGetProperty(
            unit,
            property,
            scope,
            element,
            value.as_mut_ptr().cast(),
            &mut size,
        ),
        &format!("get property {property}"),
    )?;
    if size != std::mem::size_of::<T>() as u32 {
        return Err("Audio Unit returned an invalid property size".into());
    }
    Ok(value.assume_init())
}
unsafe fn set<T>(
    unit: Unit,
    property: u32,
    scope: u32,
    element: u32,
    value: &T,
) -> Result<(), String> {
    status(
        AudioUnitSetProperty(
            unit,
            property,
            scope,
            element,
            (value as *const T).cast(),
            std::mem::size_of::<T>() as u32,
        ),
        &format!("set property {property}"),
    )
}
unsafe fn cf_string(value: CF) -> String {
    if value.is_null() {
        return String::new();
    }
    let size =
        CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), K_CF_STRING_ENCODING_UTF8) + 1;
    if !(1..=1_048_576).contains(&size) {
        return String::new();
    }
    let mut bytes = vec![0i8; size as usize];
    if CFStringGetCString(value, bytes.as_mut_ptr(), size, K_CF_STRING_ENCODING_UTF8) == 0 {
        return String::new();
    }
    CStr::from_ptr(bytes.as_ptr())
        .to_string_lossy()
        .into_owned()
}

struct InputContext {
    audio: Vec<Vec<f32>>,
    frames: usize,
    time: f64,
}
unsafe extern "C" fn pull_input(
    context: *mut c_void,
    _flags: *mut u32,
    timestamp: *const TimeStamp,
    bus: u32,
    frames: u32,
    list: *mut BufferList,
) -> i32 {
    if context.is_null() || list.is_null() || timestamp.is_null() || bus != 0 {
        return -50;
    }
    let context = &*(context as *const InputContext);
    let offset = (*timestamp).sample - context.time;
    if !offset.is_finite()
        || offset < 0.0
        || offset.fract() != 0.0
        || offset + frames as f64 > context.frames as f64
    {
        return -10874;
    }
    let offset = offset as usize;
    let count = (*list).count as usize;
    if count != context.audio.len() || count > MAX_CHANNELS {
        return -10879;
    }
    // A plugin may pass a variable-length AudioBufferList, so access only its declared buffers.
    let buffers = std::slice::from_raw_parts_mut(
        ptr::addr_of_mut!((*list).buffers).cast::<AudioBuffer>(),
        count,
    );
    for (buffer, source) in buffers.iter_mut().zip(&context.audio) {
        let bytes = frames * 4;
        if buffer.channels != 1 {
            return -10879;
        }
        if buffer.data.is_null() {
            buffer.data = source.as_ptr().add(offset) as *mut c_void;
        } else {
            if buffer.bytes < bytes {
                return -10874;
            }
            let input = source.as_ptr().add(offset);
            if buffer.data.cast::<f32>() != input as *mut f32 {
                ptr::copy(input, buffer.data.cast::<f32>(), frames as usize);
            }
        }
        buffer.bytes = bytes;
    }
    0
}

struct Parameter {
    id: u32,
    min: f32,
    max: f32,
    writable: bool,
}
impl Parameter {
    fn normalized(&self, plain: f32) -> f32 {
        if self.max > self.min {
            ((plain - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
    fn plain(&self, normalized: f32) -> f32 {
        self.min + normalized.clamp(0.0, 1.0) * (self.max - self.min)
    }
}

pub struct AudioUnitPlugin {
    unit: Unit,
    desc: ServerPlugin,
    parameters: Vec<Parameter>,
    input: Box<InputContext>,
    output: Vec<Vec<f32>>,
    initialized: bool,
    suspended: bool,
    rate: f64,
    max_frames: usize,
    sample_time: f64,
    render_error: i32,
}
// Access is exclusive: HostedPlugin is Send, not Sync. The unit and callback context
// move together; input is boxed and remains alive until AudioComponentInstanceDispose.
unsafe impl Send for AudioUnitPlugin {}

impl AudioUnitPlugin {
    pub fn new(desc: &ServerPlugin) -> Result<Self, String> {
        let fields: Vec<_> = desc.id.split(':').collect();
        if fields.len() != 4
            || fields[0] != "AudioUnit"
            || fields[1..].iter().any(|v| v.len() != 4 || !v.is_ascii())
        {
            return Err(format!("Invalid Audio Unit identifier: {}", desc.id));
        }
        let fourcc = |s: &str| u32::from_be_bytes(s.as_bytes().try_into().unwrap());
        let description = AudioComponentDescription {
            component_type: fourcc(fields[1]),
            component_sub_type: fourcc(fields[2]),
            component_manufacturer: fourcc(fields[3]),
            ..Default::default()
        };
        if fields[1] != "aufx" && fields[1] != "aumf" && fields[1] != "augn" {
            return Err("This host currently supports Audio Unit audio and music effects; instrument hosting is not implemented".into());
        }
        let channels = desc.num_output_ch;
        if channels < 1 || channels as usize > MAX_CHANNELS {
            return Err("Audio Unit requires valid channel counts (1–32)".into());
        }
        let component = unsafe { AudioComponentFindNext(ptr::null_mut(), &description) };
        if component.is_null() {
            return Err(format!(
                "Audio Unit is not installed for this architecture: {}",
                desc.id
            ));
        }
        let mut unit = ptr::null_mut();
        status(
            unsafe { AudioComponentInstanceNew(component, &mut unit) },
            "instantiate",
        )?;
        if unit.is_null() {
            return Err("Audio Unit returned a null instance".into());
        }
        let mut plugin = Self {
            unit,
            desc: desc.clone(),
            parameters: Vec::new(),
            input: Box::new(InputContext {
                audio: Vec::new(),
                frames: 0,
                time: 0.0,
            }),
            output: Vec::new(),
            initialized: false,
            suspended: false,
            rate: 48000.0,
            max_frames: 0,
            sample_time: 0.0,
            render_error: 0,
        };
        plugin.desc.has_editor = true;
        plugin.desc.presets.clear(); // Do not advertise fictitious factory presets.
        plugin.read_parameters()?;
        Ok(plugin)
    }

    fn read_parameters(&mut self) -> Result<(), String> {
        let mut size = 0;
        status(
            unsafe {
                AudioUnitGetPropertyInfo(self.unit, 3, GLOBAL, 0, &mut size, ptr::null_mut())
            },
            "parameter list size",
        )?;
        if size % 4 != 0 || size > 4 * 65536 {
            return Err("Invalid Audio Unit parameter list size".into());
        }
        let mut ids = vec![0u32; size as usize / 4];
        if size > 0 {
            status(
                unsafe {
                    AudioUnitGetProperty(
                        self.unit,
                        3,
                        GLOBAL,
                        0,
                        ids.as_mut_ptr().cast(),
                        &mut size,
                    )
                },
                "parameter list",
            )?;
        }
        ids.truncate(size as usize / 4);
        self.desc.parameters.clear();
        self.parameters.clear();
        for id in ids {
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            let mut size = std::mem::size_of::<ParameterInfo>() as u32;
            status(
                unsafe {
                    AudioUnitGetProperty(
                        self.unit,
                        4,
                        GLOBAL,
                        id,
                        (&mut info as *mut ParameterInfo).cast(),
                        &mut size,
                    )
                },
                "parameter info",
            )?;
            let name = if info.flags & (1 << 27) != 0 && !info.cf_name.is_null() {
                unsafe { cf_string(info.cf_name) }
            } else {
                let bytes: Vec<u8> = info
                    .name
                    .iter()
                    .take_while(|&&c| c != 0)
                    .map(|&c| c as u8)
                    .collect();
                String::from_utf8_lossy(&bytes).into_owned()
            };
            if info.flags & (1 << 4) != 0 {
                unsafe {
                    if !info.cf_name.is_null() && info.flags & (1 << 27) != 0 {
                        CFRelease(info.cf_name);
                    }
                    if info.unit == 26 && !info.unit_name.is_null() {
                        CFRelease(info.unit_name);
                    }
                }
            }
            if !info.min.is_finite() || !info.max.is_finite() || info.max < info.min {
                continue;
            }
            let param = Parameter {
                id,
                min: info.min,
                max: info.max,
                writable: info.flags & (1 << 31) != 0,
            };
            let mut plain = info.default;
            unsafe {
                AudioUnitGetParameter(self.unit, id, GLOBAL, 0, &mut plain);
            }
            let label = match info.unit {
                3 => "%",
                4 => "s",
                5 => "samples",
                8 => "Hz",
                13 => "dB",
                14 => "linear",
                21 => "BPM",
                24 => "ms",
                _ => "",
            }
            .to_string();
            let discrete = info.unit == 1 || info.unit == 2;
            self.desc.parameters.push(PluginParameter {
                idx: self.parameters.len() as i32,
                name,
                label,
                default_value: param.normalized(info.default),
                current_value: param.normalized(plain),
                category: 0,
                num_steps: if discrete {
                    (info.max - info.min + 1.0).min(i32::MAX as f32) as i32
                } else {
                    i32::MAX
                },
                is_boolean: info.unit == 2,
                is_discrete: discrete,
                is_meta: info.flags & ((1 << 28) | (1 << 29)) != 0,
                is_orient_inv: false,
                all_value_strings: Vec::new(),
            });
            self.parameters.push(param);
        }
        Ok(())
    }

    fn configure(&mut self, rate: f64, frames: usize) -> Result<(), String> {
        if !rate.is_finite()
            || !(8000.0..=768000.0).contains(&rate)
            || frames == 0
            || frames > 32768
        {
            return Err("Unsupported Audio Unit sample rate/block size".into());
        }
        if self.initialized && self.rate == rate && self.max_frames == frames {
            return Ok(());
        }
        self.release_resources();
        let channels = self.desc.num_output_ch as usize;
        let format = Format {
            rate,
            id: u32::from_be_bytes(*b"lpcm"),
            flags: 1 | (1 << 3) | (1 << 5),
            packet_bytes: 4,
            packet_frames: 1,
            frame_bytes: 4,
            channels: channels as u32,
            bits: 32,
            reserved: 0,
        };
        unsafe {
            set(self.unit, 8, INPUT, 0, &format)?;
            set(self.unit, 8, OUTPUT, 0, &format)?;
            set(self.unit, 14, GLOBAL, 0, &(frames as u32))?;
        }
        self.input.audio = vec![vec![0.0; frames]; channels];
        self.input.frames = 0;
        self.output = vec![vec![0.0; frames]; channels];
        let callback = RenderCallback {
            callback: pull_input,
            context: (&mut *self.input as *mut InputContext).cast(),
        };
        unsafe {
            set(self.unit, 23, INPUT, 0, &callback)?;
        }
        status(unsafe { AudioUnitInitialize(self.unit) }, "initialize")?;
        self.initialized = true;
        self.rate = rate;
        self.max_frames = frames;
        self.sample_time = 0.0;
        self.render_error = 0;
        Ok(())
    }

    fn render(&mut self, frames: usize) -> Result<(), i32> {
        if !self.initialized || frames > self.max_frames {
            return Err(-10874);
        }
        self.input.frames = frames;
        self.input.time = self.sample_time;
        let mut list = BufferList::new(self.output.len());
        for (buffer, channel) in list.buffers.iter_mut().zip(&mut self.output) {
            buffer.bytes = (frames * 4) as u32;
            buffer.data = channel.as_mut_ptr().cast();
        }
        let time = TimeStamp {
            sample: self.sample_time,
            flags: 1,
            ..Default::default()
        };
        let mut flags = 0;
        let result =
            unsafe { AudioUnitRender(self.unit, &mut flags, &time, 0, frames as u32, &mut list) };
        self.sample_time += frames as f64;
        if result != 0 {
            return Err(result);
        }
        if list.count as usize != self.output.len() {
            return Err(-10879);
        }
        for (buffer, destination) in list.buffers.iter().zip(&mut self.output) {
            if buffer.channels != 1 || buffer.data.is_null() || buffer.bytes < (frames * 4) as u32 {
                return Err(-10879);
            }
            if buffer.data != destination.as_mut_ptr().cast() {
                unsafe {
                    ptr::copy(buffer.data.cast::<f32>(), destination.as_mut_ptr(), frames);
                }
            }
        }
        Ok(())
    }
    fn failed_render(&mut self, error: i32) {
        // Log once per failure code, then return silence rather than pass corrupted plugin output.
        if self.render_error != error {
            log::error!("{} render failed (OSStatus {error})", self.desc.name);
        }
        self.render_error = error;
    }
}

impl Drop for AudioUnitPlugin {
    fn drop(&mut self) {
        self.release_resources();
        #[cfg(target_os = "macos")]
        unsafe {
            extern "C" {
                fn og_au_destroy_window(unit: Unit);
            }
            og_au_destroy_window(self.unit);
        }
        unsafe {
            AudioComponentInstanceDispose(self.unit);
        }
    }
}
impl HostedPlugin for AudioUnitPlugin {
    fn descriptor(&self) -> &ServerPlugin {
        &self.desc
    }
    fn name(&self) -> &str {
        &self.desc.name
    }
    fn prepare_to_play(&mut self, rate: f64, frames: usize) {
        if let Err(error) = self.configure(rate, frames) {
            log::error!("{}: {error}", self.name());
        }
    }
    fn try_prepare_to_play(&mut self, rate: f64, frames: usize) -> Result<(), String> {
        self.configure(rate, frames)
    }
    fn release_resources(&mut self) {
        if self.initialized {
            unsafe {
                AudioUnitUninitialize(self.unit);
            }
            self.initialized = false;
        }
    }
    fn is_suspended(&self) -> bool {
        self.suspended
    }
    fn suspend_processing(&mut self, suspended: bool) {
        self.suspended = suspended;
    }
    fn supports_double_precision(&self) -> bool {
        false
    }
    fn latency_samples(&self) -> i32 {
        let seconds = unsafe { get::<f64>(self.unit, 12, GLOBAL, 0) }.unwrap_or(0.0);
        if seconds.is_finite() {
            (seconds.max(0.0) * self.rate).ceil().min(i32::MAX as f64) as i32
        } else {
            0
        }
    }
    fn set_latency_samples(&mut self, _latency: i32) {} // Native plugin owns its latency.
    fn tail_seconds(&self) -> f64 {
        unsafe { get::<f64>(self.unit, 20, GLOBAL, 0) }
            .unwrap_or(0.0)
            .max(0.0)
    }
    fn num_input_channels(&self) -> usize {
        self.desc.num_input_ch as usize
    }
    fn num_output_channels(&self) -> usize {
        self.desc.num_output_ch as usize
    }
    fn enable_all_buses(&mut self) {}
    fn check_buses_layout_supported(&self, inputs: usize, outputs: usize) -> bool {
        inputs == self.num_input_channels() && outputs == self.num_output_channels()
    }
    fn process_block_f32(
        &mut self,
        audio: &mut [Vec<f32>],
        midi: &mut Vec<MidiEvent>,
        _pos: &PositionInfo,
    ) {
        if self.suspended {
            return;
        }
        for ev in midi.iter() {
            if !ev.data.is_empty() {
                let s = ev.data[0] as u32;
                let d1 = *ev.data.get(1).unwrap_or(&0) as u32;
                let d2 = *ev.data.get(2).unwrap_or(&0) as u32;
                let offset = ev.sample_number.max(0) as u32;
                unsafe {
                    let _ = MusicDeviceMIDIEvent(self.unit, s, d1, d2, offset);
                }
            }
        }
        let frames = audio.first().map_or(0, Vec::len);
        if frames == 0 {
            return;
        }
        if audio.len() != self.input.audio.len()
            || frames > self.max_frames
            || audio.iter().any(|ch| ch.len() != frames)
        {
            self.failed_render(-10874);
            for ch in audio {
                ch.fill(0.0);
            }
            return;
        }
        for (source, destination) in audio.iter().zip(&mut self.input.audio) {
            destination[..frames].copy_from_slice(source);
        }
        match self.render(frames) {
            Ok(()) => {
                self.render_error = 0;
                for (destination, source) in audio.iter_mut().zip(&self.output) {
                    destination.copy_from_slice(&source[..frames]);
                }
            }
            Err(error) => {
                self.failed_render(error);
                for ch in audio {
                    ch.fill(0.0);
                }
            }
        }
    }
    fn process_block_f64(
        &mut self,
        audio: &mut [Vec<f64>],
        midi: &mut Vec<MidiEvent>,
        _pos: &PositionInfo,
    ) {
        if self.suspended {
            return;
        }
        for ev in midi.iter() {
            if !ev.data.is_empty() {
                let s = ev.data[0] as u32;
                let d1 = *ev.data.get(1).unwrap_or(&0) as u32;
                let d2 = *ev.data.get(2).unwrap_or(&0) as u32;
                let offset = ev.sample_number.max(0) as u32;
                unsafe {
                    let _ = MusicDeviceMIDIEvent(self.unit, s, d1, d2, offset);
                }
            }
        }
        let frames = audio.first().map_or(0, Vec::len);
        if frames == 0 {
            return;
        }
        if audio.len() != self.input.audio.len()
            || frames > self.max_frames
            || audio.iter().any(|ch| ch.len() != frames)
        {
            self.failed_render(-10874);
            for ch in audio {
                ch.fill(0.0);
            }
            return;
        }
        for (source, destination) in audio.iter().zip(&mut self.input.audio) {
            for (s, d) in source.iter().zip(destination) {
                *d = *s as f32;
            }
        }
        match self.render(frames) {
            Ok(()) => {
                self.render_error = 0;
                for (destination, source) in audio.iter_mut().zip(&self.output) {
                    for (d, s) in destination.iter_mut().zip(source) {
                        *d = *s as f64;
                    }
                }
            }
            Err(error) => {
                self.failed_render(error);
                for ch in audio {
                    ch.fill(0.0);
                }
            }
        }
    }
    fn parameters(&self) -> &[PluginParameter] {
        &self.desc.parameters
    }
    fn get_parameter(&self, index: usize) -> f32 {
        let Some(parameter) = self.parameters.get(index) else {
            return 0.0;
        };
        let mut plain = 0.0;
        if unsafe { AudioUnitGetParameter(self.unit, parameter.id, GLOBAL, 0, &mut plain) } == 0 {
            parameter.normalized(plain)
        } else {
            0.0
        }
    }
    fn set_parameter(&mut self, index: usize, value: f32) {
        if !value.is_finite() {
            return;
        }
        if let Some(parameter) = self.parameters.get(index).filter(|p| p.writable) {
            let mut plain = parameter.plain(value);
            if self.desc.parameters[index].is_discrete {
                plain = plain.round();
            }
            let result =
                unsafe { AudioUnitSetParameter(self.unit, parameter.id, GLOBAL, 0, plain, 0) };
            if result == 0 {
                self.desc.parameters[index].current_value = parameter.normalized(plain);
            }
        }
    }
    fn get_parameter_text(&self, index: usize) -> String {
        self.parameters
            .get(index)
            .map(|p| {
                format!(
                    "{:.3} {}",
                    p.plain(self.get_parameter(index)),
                    self.desc.parameters[index].label
                )
            })
            .unwrap_or_default()
    }
    fn num_programs(&self) -> usize {
        0
    }
    fn get_program_name(&self, _index: usize) -> String {
        String::new()
    }
    fn current_program(&self) -> usize {
        0
    }
    fn set_current_program(&mut self, _index: usize) {}
    fn get_state_information(&self) -> Vec<u8> {
        unsafe {
            let Ok(value) = get::<CF>(self.unit, 0, GLOBAL, 0) else {
                return Vec::new();
            };
            if value.is_null() {
                return Vec::new();
            }
            let mut error = ptr::null();
            let data = CFPropertyListCreateData(ptr::null(), value, 200, 0, &mut error);
            CFRelease(value);
            if !error.is_null() {
                CFRelease(error);
            }
            if data.is_null() {
                return Vec::new();
            }
            let length = CFDataGetLength(data);
            let bytes = if length > 0 && length <= 64 * 1024 * 1024 {
                std::slice::from_raw_parts(CFDataGetBytePtr(data), length as usize).to_vec()
            } else {
                Vec::new()
            };
            CFRelease(data);
            bytes
        }
    }
    fn set_state_information(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() || bytes.len() > 64 * 1024 * 1024 {
            return Err("Audio Unit state is empty or too large".into());
        }
        unsafe {
            let data = CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize);
            if data.is_null() {
                return Err("Could not allocate Audio Unit state data".into());
            }
            let mut error = ptr::null();
            let value =
                CFPropertyListCreateWithData(ptr::null(), data, 0, ptr::null_mut(), &mut error);
            CFRelease(data);
            if !error.is_null() {
                CFRelease(error);
            }
            if value.is_null() {
                return Err("Invalid Audio Unit property-list state".into());
            }
            let result = set(self.unit, 0, GLOBAL, 0, &value);
            CFRelease(value);
            result?;
        }
        for index in 0..self.parameters.len() {
            self.desc.parameters[index].current_value = self.get_parameter(index);
        }
        Ok(())
    }
    fn has_editor(&self) -> bool {
        true
    }
    fn editor_size(&self) -> (u32, u32) {
        (640, 360)
    }
    fn show_editor(&mut self, x: i32, y: i32, on_top: bool) -> bool {
        #[cfg(target_os = "macos")]
        unsafe {
            extern "C" {
                fn og_au_show_window(unit: Unit, title: *const std::os::raw::c_char, x: i32, y: i32, on_top: bool) -> bool;
            }
            let title = std::ffi::CString::new(self.desc.name.clone()).unwrap_or_default();
            og_au_show_window(self.unit, title.as_ptr(), x, y, on_top)
        }
        #[cfg(not(target_os = "macos"))]
        false
    }
    fn hide_editor(&mut self) {
        #[cfg(target_os = "macos")]
        unsafe {
            extern "C" {
                fn og_au_hide_window(unit: Unit);
            }
            og_au_hide_window(self.unit);
        }
    }
    fn is_editor_open(&self) -> bool {
        true
    }
    fn render_editor_rgba(&self) -> (u32, u32, Vec<u8>) {
        let mut desc = self.desc.clone();
        for (index, parameter) in desc.parameters.iter_mut().enumerate() {
            parameter.current_value = self.get_parameter(index);
        }
        HostedPluginInstance::new(desc).render_editor_rgba()
    }
}
