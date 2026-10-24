//! Screen capture, delta diffing, and image reader for OpenGridder.
//!
//! Ports `Server/Source/Screen.h`, `Server/Source/ScreenWorker.hpp`/`.cpp`, and
//! `Plugin/Source/ImageReader.hpp`/`.cpp` without JUCE or FFmpeg dependencies.

use image::{ImageBuffer, ImageFormat, Rgba};
use og_common::image_diff;
use og_common::message::{RawMessage, ScreenHeader};
use std::io::Cursor;

/// Encodes captured RGBA frames (either full PNG or delta-diffed RGBA) for the `ScreenCapture` wire message.
pub struct ScreenEncoder {
    last_frame: Vec<u8>,
    last_width: u32,
    last_height: u32,
    diff_buf: Vec<u8>,
    pub enable_diff_detection: bool,
    pub use_png: bool,
}

impl Default for ScreenEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenEncoder {
    pub fn new() -> Self {
        Self {
            last_frame: Vec::new(),
            last_width: 0,
            last_height: 0,
            diff_buf: Vec::new(),
            enable_diff_detection: true,
            use_png: true,
        }
    }

    pub fn reset(&mut self) {
        self.last_frame.clear();
        self.last_width = 0;
        self.last_height = 0;
    }

    /// Encode an RGBA frame into a `ScreenCapture` `RawMessage`.
    /// Returns `None` if diff detection is enabled and zero pixels changed since the last frame.
    pub fn encode_frame(
        &mut self,
        width: u32,
        height: u32,
        scale: f64,
        rgba: &[u8],
    ) -> Option<RawMessage> {
        let expected_len = (width as usize) * (height as usize) * 4;
        if rgba.len() < expected_len {
            return None;
        }
        let rgba = &rgba[..expected_len];

        let size_changed = width != self.last_width || height != self.last_height;
        if size_changed {
            self.last_frame = vec![0u8; expected_len];
            self.diff_buf = vec![0u8; expected_len];
            self.last_width = width;
            self.last_height = height;
        }

        if self.enable_diff_detection && !size_changed {
            let changed_px = image_diff::get_delta(
                &self.last_frame,
                rgba,
                &mut self.diff_buf,
                width as usize,
                height as usize,
            );
            if changed_px == 0 {
                return None;
            }
        }

        self.last_frame.copy_from_slice(rgba);

        let payload_bytes = if self.use_png {
            let img: ImageBuffer<Rgba<u8>, &[u8]> =
                ImageBuffer::from_raw(width, height, rgba)?;
            let mut cursor = Cursor::new(Vec::new());
            img.write_to(&mut cursor, ImageFormat::Png).ok()?;
            cursor.into_inner()
        } else {
            rgba.to_vec()
        };

        let hdr = ScreenHeader {
            width,
            height,
            scale,
            size: payload_bytes.len() as u32,
        };
        let mut msg_data = hdr.to_bytes().to_vec();
        msg_data.extend_from_slice(&payload_bytes);
        Some(RawMessage::new(ScreenHeader::TYPE, msg_data))
    }
}

/// Decodes incoming `ScreenCapture` messages on the client (`ImageReader` in `Plugin/Source/ImageReader.cpp`).
pub struct ImageReader {
    current_rgba: Vec<u8>,
    width: u32,
    height: u32,
    scale: f64,
}

impl Default for ImageReader {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageReader {
    pub fn new() -> Self {
        Self {
            current_rgba: Vec::new(),
            width: 0,
            height: 0,
            scale: 1.0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn current_rgba(&self) -> &[u8] {
        &self.current_rgba
    }

    /// Decode a `ScreenCapture` `RawMessage` into an RGBA pixel buffer.
    pub fn read_message(&mut self, msg: &RawMessage) -> Result<(u32, u32, f64, &[u8]), String> {
        let hdr = ScreenHeader::from_bytes(&msg.payload)
            .ok_or_else(|| "screen message too short for ScreenHeader".to_string())?;
        if hdr.width == 0 || hdr.height == 0 || hdr.size == 0 {
            return Ok((self.width, self.height, self.scale, &self.current_rgba));
        }
        let body = &msg.payload[ScreenHeader::SIZE..];
        if body.len() < hdr.size as usize {
            return Err("screen message payload truncated".to_string());
        }
        let img_bytes = &body[..hdr.size as usize];

        let expected_raw = (hdr.width as usize) * (hdr.height as usize) * 4;
        if img_bytes.len() >= 8 && &img_bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
            let decoded = image::load_from_memory_with_format(img_bytes, ImageFormat::Png)
                .map_err(|e| format!("PNG decode error: {}", e))?
                .to_rgba8();
            self.current_rgba = decoded.into_raw();
        } else if img_bytes.len() == expected_raw {
            self.current_rgba = img_bytes.to_vec();
        } else {
            return Err("unrecognized screen payload format".to_string());
        }

        self.width = hdr.width;
        self.height = hdr.height;
        self.scale = hdr.scale;
        Ok((self.width, self.height, self.scale, &self.current_rgba))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screen_encode_decode_and_diff_skip() {
        let mut enc = ScreenEncoder::new();
        let mut dec = ImageReader::new();

        let mut rgba = vec![120u8; 64 * 48 * 4];
        let msg1 = enc
            .encode_frame(64, 48, 2.0, &rgba)
            .expect("first frame must encode");

        // Second identical frame should be skipped by diff detection
        assert!(enc.encode_frame(64, 48, 2.0, &rgba).is_none());

        // Modify 1 pixel -> should encode again
        rgba[0] = 250;
        let msg2 = enc
            .encode_frame(64, 48, 2.0, &rgba)
            .expect("modified frame must encode");

        let (w1, h1, s1, px1) = dec.read_message(&msg1).expect("decode 1");
        assert_eq!((w1, h1, s1), (64, 48, 2.0));
        assert_eq!(px1[0], 120);

        let (_w2, _h2, _s2, px2) = dec.read_message(&msg2).expect("decode 2");
        assert_eq!(px2[0], 250);
    }
}
