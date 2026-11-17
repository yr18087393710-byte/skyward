//! The picture the phone's own system shows for its owner, read from the
//! user's photo and cut to the circle the pages draw it in.

use anyhow::{anyhow, ensure, Context, Result};
use std::path::Path;

/// A square picture, its pixels already cut to a circle: what lies outside the
/// circle is left transparent, so a page lays it over its card as it is.
#[derive(Clone, Debug)]
pub struct Avatar {
    /// The side of the square, in pixels.
    pub size: u32,
    /// `size * size` pixels of RGBA, row by row.
    pub rgba: Vec<u8>,
}

impl PartialEq for Avatar {
    /// Two of these are told apart without reading their pixels: a picture is
    /// read once, and the pages ask whether it has come, not what it shows.
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.rgba.len() == other.rgba.len() && self.rgba.as_ptr() == other.rgba.as_ptr()
    }
}

impl Avatar {
    /// Read the picture at `path` and cut it to a circle `size` pixels across.
    ///
    /// The phone keeps its owner's picture as a JPEG; the pages also take a
    /// PNG, since what a phone saves it as is not theirs to choose.
    pub fn read(path: &Path, size: u32) -> Result<Avatar> {
        ensure!(size > 0, "a picture of no width");
        let bytes = std::fs::read(path).with_context(|| format!("open {}", path.display()))?;
        let (pixels, w, h, channels) = match bytes.as_slice() {
            [0xFF, 0xD8, ..] => jpeg(&bytes, path)?,
            [0x89, b'P', b'N', b'G', ..] => png(&bytes, path)?,
            _ => anyhow::bail!("{} is neither a JPEG nor a PNG", path.display()),
        };
        let rgba = cut(&pixels, w, h, channels, size);
        Ok(Avatar { size, rgba })
    }
}

/// A PNG's pixels, normalized to eight bits a channel.
fn png(bytes: &[u8], path: &Path) -> Result<(Vec<u8>, u32, u32, usize)> {
    let _ = path;
    {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        // The phone's own picture is whatever it was saved as; the pages want
        // RGBA and nothing else.
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().with_context(|| format!("read {}", path.display()))?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).with_context(|| format!("read {}", path.display()))?;
        let (w, h) = (info.width, info.height);
        ensure!(w > 0 && h > 0, "{} is a picture of no size", path.display());
        let channels = match info.color_type {
            png::ColorType::Rgba => 4,
            png::ColorType::Rgb => 3,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Grayscale => 1,
            other => anyhow::bail!("{} is a picture of {other:?}, which a page cannot show", path.display()),
        };
        Ok((buf, w, h, channels))
    }
}

/// A JPEG's pixels, as three channels.
fn jpeg(bytes: &[u8], path: &Path) -> Result<(Vec<u8>, u32, u32, usize)> {
    let mut decoder = zune_jpeg::JpegDecoder::new(bytes);
    // The decoder's own errors are not std ones, so they are said as they come.
    decoder.decode_headers().map_err(|e| anyhow!("read {}: {e:?}", path.display()))?;
    let (w, h) = decoder.dimensions().ok_or_else(|| anyhow!("{} is a picture of no size", path.display()))?;
    let mut pixels = decoder.decode().map_err(|e| anyhow!("read {}: {e:?}", path.display()))?;
    // A four-channel picture is cut down to three: a page wants a picture, not
    // the two bytes per pixel the phone's own saved alongside.
    if pixels.len() == w as usize * h as usize * 4 {
        let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
        for px in pixels.chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
        }
        pixels = rgb;
    }
    ensure!(
        pixels.len() == w as usize * h as usize * 3,
        "{} is a picture of {} bytes, which is not a size",
        path.display(),
        pixels.len()
    );
    Ok((pixels, w as u32, h as u32, 3))
}

/// The picture scaled to `size` square, its corners taken off: each pixel of
/// the square is read from the one under it in the picture, and left clear when
/// it falls outside the circle.
fn cut(src: &[u8], w: u32, h: u32, channels: usize, size: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    let radius = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - radius, y as f32 + 0.5 - radius);
            if dx * dx + dy * dy > radius * radius {
                continue;
            }
            // The picture is square in practice, but one that is not is read
            // across its own width and height.
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

/// The name the phone gives itself, read from what its settings hold: the
/// `device_name` line of the settings file, whose bytes are a string table
/// rather than text.
///
/// The name the owner gave the phone is the one at the top of its settings,
/// which is kept as `bluetooth_name` — MIUI writes the name set there to that
/// key, since the name the phone is found by over Bluetooth is the same one.
/// Where that is absent the settings' own `device_name` is read; where nothing
/// is kept at all, the caller falls back to the model.
pub fn device_name(secure: &Path) -> Option<String> {
    named(secure, b"bluetooth_name").or_else(|| named(secure, b"device_name"))
}

/// The string kept after `key` in the settings file at `path`.
fn named(path: &Path, key: &[u8]) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    // Each string is a zero, the index it is kept at, a zero, its length, and
    // its bytes. The name wanted is the one kept just after `key`.
    let at = bytes.windows(key.len()).position(|w| w == key)? + key.len();
    let mut i = at;
    // A slash may follow the key, as the phone's own format writes it.
    while i < bytes.len() && bytes[i] == b'/' {
        i += 1;
    }
    while i < bytes.len() && bytes[i] != 0 {
        i += 1;
    }
    let mut j = i;
    while j < bytes.len() && bytes[j] == 0 {
        j += 1;
    }
    // The index the string is kept at, then the length of what follows.
    j += 1;
    while j < bytes.len() && bytes[j] == 0 {
        j += 1;
    }
    let len = *bytes.get(j)? as usize;
    j += 1;
    if len == 0 || len > 512 {
        return None;
    }
    let name = std::str::from_utf8(bytes.get(j..j + len)?).ok()?.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_cut_to_a_circle_its_corners_left_clear() {
        // Four pixels, all of them white: the circle takes the middle of them.
        let src = vec![255u8; 4 * 4 * 4];
        let out = cut(&src, 4, 4, 4, 8);
        let at = |x: u32, y: u32| out[((y * 8 + x) * 4) as usize + 3];
        assert_eq!(at(0, 0), 0, "the corner is outside the circle");
        assert_eq!(at(7, 7), 0);
        assert_eq!(at(4, 4), 255, "the middle is in it");
        assert_eq!(at(4, 0), 255, "and so is the top of it");
        assert_eq!(at(0, 4), 255, "and its left");
    }

    #[test]
    fn a_name_is_read_from_the_settings_string_table() {
        // What the phone keeps: zero, index, zero, length, then the bytes. The
        // Bluetooth name comes first, the model behind it.
        let mut bytes = vec![b'X'; 20];
        bytes.extend(b"\x00\x04\x00\x0ebluetooth_name/\x00\x05\x00\x07Mari0us/\x00\x06\x00\x0bdevice_name/\x00\x07\x00\x09Xiaomi 17/");
        let at = std::env::temp_dir().join("skyward-avatar-name-test");
        std::fs::write(&at, &bytes).unwrap();
        assert_eq!(device_name(&at).as_deref(), Some("Mari0us"), "the name the owner gave is the one read");
        let _ = std::fs::remove_file(&at);
    }

    #[test]
    fn the_settings_own_name_is_read_where_the_owners_is_not_kept() {
        let mut bytes = vec![b'X'; 20];
        bytes.extend(b"\x00\x06\x00\x0bdevice_name/\x00\x07\x00\x09Xiaomi 17/");
        let at = std::env::temp_dir().join("skyward-avatar-model-test");
        std::fs::write(&at, &bytes).unwrap();
        assert_eq!(device_name(&at).as_deref(), Some("Xiaomi 17"));
        let _ = std::fs::remove_file(&at);
    }

    #[test]
    fn a_chinese_name_is_read_whole() {
        // The one the owner gave this phone: four characters, twelve bytes.
        let mut bytes = vec![b'X'; 20];
        bytes.extend(b"\x00\x04\x00\x0ebluetooth_name/\x00\x05\x00\x0c");
        bytes.extend("碎屏飞牛".as_bytes());
        bytes.push(b'/');
        let at = std::env::temp_dir().join("skyward-avatar-chinese-test");
        std::fs::write(&at, &bytes).unwrap();
        assert_eq!(device_name(&at).as_deref(), Some("碎屏飞牛"));
        let _ = std::fs::remove_file(&at);
    }

    #[test]
    fn no_name_is_none() {
        let at = std::env::temp_dir().join("skyward-avatar-none-test");
        std::fs::write(&at, b"nothing here").unwrap();
        assert_eq!(device_name(&at), None);
        let _ = std::fs::remove_file(&at);
    }
}
