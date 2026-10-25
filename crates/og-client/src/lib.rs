//! `og-client`: AudioGridder client networking and lock-free audio streaming.

pub mod audio_streamer;
pub mod client;

pub use audio_streamer::{send_and_receive_f32, send_and_receive_f64, AudioChunk, AudioStreamer};
pub use client::{Client, ClientConfig, ParamChangeCallback, ScreenFrame, ScreenUpdateCallback};
