//! The picture an app shows in its row: the png its apk keeps, read once and
//! scaled to the square the row draws it in.

use anyhow::{ensure, Result};

/// A square picture, its pixels laid out as a page draws them: what the png
/// leaves clear stays clear.
#[derive(Clone, Debug)]
pub struct Picture {
    /// The side of the square, in pixels.
    pub size: u32,
    /// `size * size` pixels of RGBA, row by row.
    pub rgba: Vec<u8>,
}

impl PartialEq for Picture {
    /// Two of these are told apart without reading their pixels: a picture is
    /// read once, and the pages ask whether it has come, not what it shows.
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.rgba.len() == other.rgba.len() && self.rgba.as_ptr() == other.rgba.as_ptr()
    }
}

impl Picture {
    /// Read the png `bytes` into a `size` square, the picture drawn over the
    /// whole of it.
    pub fn png(bytes: &[u8], size: u32) -> Result<Picture> {
        ensure!(size > 0, "a picture of no width");
        let (src, w, h, channels) = super::avatar::png_pixels(bytes)?;
        ensure!(w > 0 && h > 0, "a picture of no size");
        Ok(Picture { size, rgba: fit(&src, w, h, channels, size) })
    }
}

/// The picture scaled into a `size` square: each pixel of the square is read
/// from the one under it in the picture, which is laid over the whole of the
/// square, so a picture that is not square is stretched to it.
fn fit(src: &[u8], w: u32, h: u32, channels: usize, size: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let sx = ((x as u64 * w as u64) / size as u64) as u32;
            let sy = ((y as u64 * h as u64) / size as u64) as u32;
            let at = ((sy.min(h - 1) * w + sx.min(w - 1)) as usize) * channels;
            let into = ((y * size + x) * 4) as usize;
            let (r, g, b, a) = match channels {
                4 => (src[at], src[at + 1], src[at + 2], src[at + 3]),
                3 => (src[at], src[at + 1], src[at + 2], 255),
                2 => (src[at], src[at], src[at], src[at + 1]),
                _ => (src[at], src[at], src[at], 255),
            };
            out[into] = r;
            out[into + 1] = g;
            out[into + 2] = b;
            out[into + 3] = a;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A png of `w` by `h` pixels, every one of them one colour.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(std::io::Cursor::new(&mut out), w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let pixels = vec![200u8; (w * h * 4) as usize];
            writer.write_image_data(&pixels).unwrap();
        }
        out
    }

    #[test]
    fn a_picture_is_scaled_into_the_square_its_row_draws() {
        let picture = Picture::png(&png(8, 8), 4).expect("a png of eight by eight");
        assert_eq!((picture.size, picture.rgba.len()), (4, 4 * 4 * 4));
        assert_eq!(&picture.rgba[..4], &[200, 200, 200, 200], "the picture covers the whole square");
    }

    #[test]
    fn a_picture_wider_than_it_is_tall_is_stretched_to_the_square() {
        let out = fit(&vec![9u8; 10 * 4 * 3], 10, 4, 3, 4);
        assert_eq!(out.len(), 4 * 4 * 4);
        assert_eq!(&out[..4], &[9, 9, 9, 255], "a picture of three channels is opaque");
    }
}