//! Port of `Common/Source/ImageDiff.hpp`.

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PixelARGB {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

impl PixelARGB {
    pub fn brightness(&self) -> f32 {
        (self.r as f32 + self.g as f32 + self.b as f32) / (3.0 * 255.0)
    }
}

pub fn get_delta(
    img_from: &[u8],
    img_to: &[u8],
    img_delta: &mut [u8],
    width: usize,
    height: usize,
) -> u64 {
    let total_pixels = width * height;
    if img_from.len() < total_pixels * 4
        || img_to.len() < total_pixels * 4
        || img_delta.len() < total_pixels * 4
    {
        return 0;
    }

    let mut count = 0u64;
    for i in 0..total_pixels {
        let off = i * 4;
        let from_px = &img_from[off..off + 4];
        let to_px = &img_to[off..off + 4];
        if from_px != to_px {
            count += 1;
            img_delta[off] = to_px[0];
            img_delta[off + 1] = to_px[1];
            img_delta[off + 2] = to_px[2];
            img_delta[off + 3] = 255;
        } else {
            img_delta[off..off + 4].fill(0);
        }
    }
    count
}

pub fn apply_delta(img_dst: &mut [u8], img_delta: &[u8], width: usize, height: usize) -> u64 {
    let total_pixels = width * height;
    if img_dst.len() < total_pixels * 4 || img_delta.len() < total_pixels * 4 {
        return 0;
    }

    let mut count = 0u64;
    for i in 0..total_pixels {
        let off = i * 4;
        if img_delta[off + 3] == 255 {
            img_dst[off..off + 4].copy_from_slice(&img_delta[off..off + 4]);
            count += 1;
        }
    }
    count
}

pub fn get_brightness(img: &[u8], width: usize, height: usize) -> f32 {
    let total_pixels = width * height;
    if img.len() < total_pixels * 4 {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for i in 0..total_pixels {
        let off = i * 4;
        let px = PixelARGB {
            b: img[off],
            g: img[off + 1],
            r: img[off + 2],
            a: img[off + 3],
        };
        sum += px.brightness();
    }
    sum
}
