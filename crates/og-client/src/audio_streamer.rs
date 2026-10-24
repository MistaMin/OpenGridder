//! Lock-free buffered audio/MIDI network streamer.
//!
//! Ports `Plugin/Source/AudioStreamer.hpp` (`AudioStreamer<T>`, `write_q`, `read_q`,
//! `NUM_OF_BUFFERS`, `FIXED_OUTBOUND_BUFFER`, `LIVE_MODE`, and synchronous/buffered streaming).

use crossbeam_channel::{bounded, Receiver, Sender};
use og_common::message::{AudioMessage, MidiEvent, PositionInfo};
use og_common::metrics::TimeStatistic;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AudioChunk<T: Copy> {
    pub audio: Vec<Vec<T>>,
    pub midi: Vec<MidiEvent>,
    pub pos: PositionInfo,
}

/// Manages buffered or direct synchronous audio/MIDI streaming over the dedicated `audio` socket,
/// matching `AudioStreamer<T>` in `Plugin/Source/AudioStreamer.hpp`.
pub struct AudioStreamer<T: Copy + Default + Send + 'static> {
    pub num_of_buffers: usize,
    pub fixed_outbound_buffer: bool,
    pub requested_channels: usize,
    pub requested_samples: usize,
    write_tx: Sender<AudioChunk<T>>,
    write_rx: Receiver<AudioChunk<T>>,
    read_tx: Sender<AudioChunk<T>>,
    read_rx: Receiver<AudioChunk<T>>,
    error: Arc<AtomicBool>,
    last_latency_samples: Arc<AtomicI32>,
}

impl<T: Copy + Default + Send + 'static> AudioStreamer<T> {
    pub fn new(num_of_buffers: usize, requested_channels: usize, requested_samples: usize) -> Self {
        let cap = num_of_buffers.max(1) * 4;
        let (write_tx, write_rx) = bounded(cap);
        let (read_tx, read_rx) = bounded(cap);
        Self {
            num_of_buffers,
            fixed_outbound_buffer: true,
            requested_channels,
            requested_samples,
            write_tx,
            write_rx,
            read_tx,
            read_rx,
            error: Arc::new(AtomicBool::new(false)),
            last_latency_samples: Arc::new(AtomicI32::new(0)),
        }
    }

    pub fn has_error(&self) -> bool {
        self.error.load(Ordering::Relaxed)
    }

    pub fn last_latency_samples(&self) -> i32 {
        self.last_latency_samples.load(Ordering::Relaxed)
    }

    pub fn write_rx(&self) -> Receiver<AudioChunk<T>> {
        self.write_rx.clone()
    }

    pub fn read_tx(&self) -> Sender<AudioChunk<T>> {
        self.read_tx.clone()
    }

    pub fn enqueue(&self, chunk: AudioChunk<T>) -> bool {
        self.write_tx.try_send(chunk).is_ok()
    }

    pub fn try_dequeue(&self) -> Option<AudioChunk<T>> {
        self.read_rx.try_recv().ok()
    }
}

/// Perform a single synchronous `f32` audio + MIDI roundtrip over `stream`,
/// matching `AudioMessage::sendToServer` + `AudioMessage::readFromServer` in `Message.hpp`.
pub fn send_and_receive_f32<S: Read + Write>(
    stream: &mut S,
    audio: &mut [Vec<f32>],
    midi: &mut Vec<MidiEvent>,
    pos: &PositionInfo,
    channels_requested: i32,
    samples_requested: i32,
) -> std::io::Result<i32> {
    let _timer = TimeStatistic::get_duration_guard("audio_stream");
    let mut msg = AudioMessage::new();
    msg.send_to_server_f32(
        stream,
        audio,
        midi,
        pos,
        channels_requested,
        samples_requested,
    )
    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let max_ch = audio.len().max(channels_requested.max(0) as usize);
    let max_samples = audio
        .first()
        .map(|c| c.len())
        .unwrap_or(0)
        .max(samples_requested.max(0) as usize);
    let mut resp_audio = vec![vec![0.0f32; max_samples]; max_ch];
    let mut resp_midi = Vec::new();
    msg.read_from_server_f32(stream, &mut resp_audio, &mut resp_midi)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    for (dst, src) in audio.iter_mut().zip(resp_audio.iter()) {
        let n = dst.len().min(src.len());
        dst[..n].copy_from_slice(&src[..n]);
    }
    *midi = resp_midi;
    Ok(msg.res_header.latency_samples)
}

/// Perform a single synchronous `f64` audio + MIDI roundtrip over `stream`.
pub fn send_and_receive_f64<S: Read + Write>(
    stream: &mut S,
    audio: &mut [Vec<f64>],
    midi: &mut Vec<MidiEvent>,
    pos: &PositionInfo,
    channels_requested: i32,
    samples_requested: i32,
) -> std::io::Result<i32> {
    let _timer = TimeStatistic::get_duration_guard("audio_stream");
    let mut msg = AudioMessage::new();
    msg.send_to_server_f64(
        stream,
        audio,
        midi,
        pos,
        channels_requested,
        samples_requested,
    )
    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let max_ch = audio.len().max(channels_requested.max(0) as usize);
    let max_samples = audio
        .first()
        .map(|c| c.len())
        .unwrap_or(0)
        .max(samples_requested.max(0) as usize);
    let mut resp_audio = vec![vec![0.0f64; max_samples]; max_ch];
    let mut resp_midi = Vec::new();
    msg.read_from_server_f64(stream, &mut resp_audio, &mut resp_midi)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    for (dst, src) in audio.iter_mut().zip(resp_audio.iter()) {
        let n = dst.len().min(src.len());
        dst[..n].copy_from_slice(&src[..n]);
    }
    *midi = resp_midi;
    Ok(msg.res_header.latency_samples)
}
