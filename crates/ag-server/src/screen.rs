//! `ScreenWorker::runNative` with diff detection on.
//!
//! Capture 0 and every 20th capture are a full PNG. Other captures are a PNG
//! of the changed pixels: unchanged pixels are 0,0,0,0 and changed pixels keep
//! their color with alpha 255. An unchanged capture is not sent. A capture
//! whose brightness is at least 99 percent white, or at most 0.1, is not sent.
//! The editor baseline is dropped so the next capture is a full image.
//! Bytes are R, G, B, A. JPEG quality stays unused while diff detection is on.

/// One editor image kept so the next capture can be a delta.
struct Image {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
}

pub struct ScreenDiff {
    last: Option<Image>,
    /// `ScreenWorker::runNative` increments this for every capture, including
    /// ones that are skipped or that reset the editor.
    captures: u32,
}

impl ScreenDiff {
    pub fn new() -> Self {
        Self { last: None, captures: 0 }
    }

    /// Hide, show, and a recreated editor drop the previous image.
    /// `captures` keeps counting.
    pub fn clear_baseline(&mut self) {
        self.last = None;
    }

    /// Returns PNG bytes to write on the screen socket, or `None` when the
    /// capture is identical or the editor must be reset instead.
    pub fn capture(&mut self, width: i32, height: i32, pixels: &[u8]) -> Option<Vec<u8>> {
        let count = self.captures;
        self.captures = self.captures.wrapping_add(1);
        let expected = (width.max(0) as usize).saturating_mul(height.max(0) as usize).saturating_mul(4);
        if width <= 0 || height <= 0 || pixels.len() < expected {
            return None;
        }
        let pixels = &pixels[..expected];
        let refresh = count % 20 == 0;
        let force_full = refresh;
        let area = (width as f32) * (height as f32);
        let mostly_white = area * 0.99;
        let mostly_black = 0.1f32;
        let mut brightness = mostly_white / 2.0;
        let mut diff_count = (width as u64) * (height as u64);
        let mut send = pixels.to_vec();
        let same = self.last.as_ref().is_some_and(|last| last.width == width && last.height == height && last.pixels.len() == pixels.len());
        if !force_full && same {
            let last = self.last.as_ref().expect("baseline");
            brightness = 0.0;
            diff_count = 0;
            let mut delta = vec![0u8; pixels.len()];
            for (index, (before, after)) in last.pixels.chunks_exact(4).zip(pixels.chunks_exact(4)).enumerate() {
                brightness += (f32::from(after[0]) + f32::from(after[1]) + f32::from(after[2])) / (255.0 * 3.0);
                if before != after {
                    diff_count += 1;
                    let at = index * 4;
                    delta[at..at + 3].copy_from_slice(&after[..3]);
                    delta[at + 3] = 255;
                }
            }
            send = delta;
        }
        self.last = Some(Image { width, height, pixels: pixels.to_vec() });
        if brightness >= mostly_white || brightness <= mostly_black {
            eprintln!("resetting editor window");
            self.last = None;
            return None;
        }
        if diff_count == 0 {
            return None;
        }
        Some(encode_png(width as u32, height as u32, &send))
    }
}

/// Gray byte for a normalized parameter. 0 is black and 1 is white.
pub fn gray_level(normalized: f32) -> u8 {
    (normalized.clamp(0.0, 1.0) * 255.0) as u8
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    // Compressed with ImageIO in the server; the tests read the stored-block encoding below.
    #[cfg(not(test))]
    if let Some(png) = crate::ui::encode_png(width, height, rgba) {
        return png;
    }
    encode_png_stored(width, height, rgba)
}

fn encode_png_stored(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut filtered = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    let stride = width as usize * 4;
    for row in 0..height as usize {
        filtered.push(0);
        let start = row * stride;
        filtered.extend_from_slice(&rgba[start..start + stride]);
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"IDAT", &zlib_store(&filtered)));
    png.extend(chunk(b"IEND", &[]));
    png
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_bytes = Vec::with_capacity(4 + data.len());
    crc_bytes.extend_from_slice(kind);
    crc_bytes.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_bytes).to_be_bytes());
    out
}

/// zlib wrapper around stored deflate blocks. The wire test inflates only
/// this encoding: CMF/FLG `78 01`, then length and one's complement.
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut rest = data;
    if rest.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while !rest.is_empty() {
        let n = rest.len().min(65535);
        let last = n == rest.len();
        out.push(if last { 1 } else { 0 });
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&(!(n as u16)).to_le_bytes());
        out.extend_from_slice(&rest[..n]);
        rest = &rest[n..];
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
