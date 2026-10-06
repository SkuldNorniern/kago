//! PNG images for setup branding: decoded to straight RGBA, scaled to the
//! size they are drawn at. Aurea draws images unscaled pixel by pixel, so
//! scaling happens here, once.

use std::io::Cursor;

/// straight (not premultiplied) RGBA pixels, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// decodes a PNG of any color type and bit depth to 8-bit RGBA.
///
/// # Errors
///
/// when `bytes` is not a PNG kago can read.
pub fn decode_png(bytes: &[u8]) -> Result<Rgba, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("not a PNG: {e}"))?;
    let size = reader
        .output_buffer_size()
        .ok_or("PNG too large to decode")?;
    let mut buf = vec![0; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("broken PNG: {e}"))?;
    buf.truncate(info.buffer_size());
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed PNG was not expanded".to_owned()),
    };
    Ok(Rgba {
        width: info.width,
        height: info.height,
        pixels,
    })
}

/// the PNG inside an icon's largest PNG image, if it has one. icons made
/// for Windows Vista and later keep their 256px image as PNG.
#[must_use]
pub fn png_in_icon(icon: &super::ico::IconGroup) -> Option<&[u8]> {
    icon.images
        .iter()
        .filter(|image| image.starts_with(b"\x89PNG\r\n\x1a\n"))
        .max_by_key(|image| image.len())
        .map(Vec::as_slice)
}

/// the icon's largest image as PNG bytes, for the setup's logo: a PNG
/// image as it is, else the largest 32-bit bitmap image converted.
#[must_use]
pub fn logo_from_icon(icon: &super::ico::IconGroup) -> Option<Vec<u8>> {
    if let Some(png) = png_in_icon(icon) {
        return Some(png.to_vec());
    }
    let bitmap = icon
        .images
        .iter()
        .filter_map(|image| bitmap_image(image))
        .max_by_key(|rgba| rgba.width)?;
    encode_png(&bitmap).ok()
}

/// a 32-bit icon bitmap (`BITMAPINFOHEADER`, rows bottom-up, BGRA, height
/// doubled for the mask) as RGBA.
fn bitmap_image(data: &[u8]) -> Option<Rgba> {
    let u32_at = |i: usize| {
        data.get(i..i + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    let u16_at = |i: usize| data.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    if u32_at(0)? != 40 || u16_at(14)? != 32 {
        return None;
    }
    let width = u32_at(4)?;
    let height = u32_at(8)? / 2;
    let (w, h) = (width as usize, height as usize);
    let pixels_at = 40;
    let mut pixels = Vec::with_capacity(w * h * 4);
    for row in (0..h).rev() {
        let start = pixels_at + row * w * 4;
        for p in data.get(start..start + w * 4)?.as_chunks::<4>().0 {
            pixels.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    Some(Rgba {
        width,
        height,
        pixels,
    })
}

/// RGBA as PNG bytes.
///
/// # Errors
///
/// when the encoder fails.
pub fn encode_png(image: &Rgba) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&image.pixels)
        .map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

impl Rgba {
    /// scaled to fit inside `width` x `height`, keeping its shape. shrinking
    /// averages the pixels it covers, growing repeats them.
    #[must_use]
    pub fn fit(&self, width: u32, height: u32) -> Self {
        if self.width == 0 || self.height == 0 || width == 0 || height == 0 {
            return self.clone();
        }
        let scale = (f64::from(width) / f64::from(self.width))
            .min(f64::from(height) / f64::from(self.height));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (w, h) = (
            ((f64::from(self.width) * scale).round() as u32).max(1),
            ((f64::from(self.height) * scale).round() as u32).max(1),
        );
        self.resize(w, h)
    }

    /// the same image with its corners cut round, `radius` pixels, the
    /// edge smoothed over one pixel.
    #[must_use]
    pub fn rounded(&self, radius: u32) -> Self {
        let mut out = self.clone();
        let corner = f64::from(radius.min(self.width / 2).min(self.height / 2));
        if corner < 1.0 {
            return out;
        }
        // the nearest point of the rounded shape's inner box.
        let inside = |at: f64, size: u32| at.clamp(corner, f64::from(size) - corner);
        for row in 0..self.height {
            for col in 0..self.width {
                // how far the pixel's middle is outside the corner's curve.
                let (px, py) = (f64::from(col) + 0.5, f64::from(row) + 0.5);
                let distance = (px - inside(px, self.width)).hypot(py - inside(py, self.height));
                let outside = (distance - corner + 0.5).clamp(0.0, 1.0);
                if outside > 0.0 {
                    let alpha = ((row * self.width + col) * 4 + 3) as usize;
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let kept = (f64::from(out.pixels[alpha]) * (1.0 - outside)).round() as u8;
                    out.pixels[alpha] = kept;
                }
            }
        }
        out
    }

    /// the image's main color as an accent: the average of its colorful,
    /// opaque pixels, made light enough to be a button. `None` for a gray
    /// or see-through image.
    #[must_use]
    pub fn tone(&self) -> Option<[u8; 3]> {
        let mut sum = [0.0f64; 3];
        let mut weight = 0.0;
        for p in self.pixels.as_chunks::<4>().0 {
            let [r, g, b] = [p[0], p[1], p[2]].map(f64::from);
            let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
            // saturation, times opacity: gray and clear pixels count little.
            let w = (hi - lo) / 255.0 * f64::from(p[3]) / 255.0;
            sum[0] += r * w;
            sum[1] += g * w;
            sum[2] += b * w;
            weight += w;
        }
        let pixels = (self.pixels.len() / 4).max(1);
        #[allow(clippy::cast_precision_loss)]
        if weight < 0.04 * pixels as f64 {
            return None;
        }
        let avg = sum.map(|c| c / weight);
        // as bright as kago's own accent, so text on it stays readable.
        let hi = avg[0].max(avg[1]).max(avg[2]).max(1.0);
        let scale = 230.0 / hi;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some(avg.map(|c| (c * scale).clamp(0.0, 255.0).round() as u8))
    }

    /// scaled to fill `width` x `height`, cut evenly at the sides that
    /// stick out, like a side banner.
    #[must_use]
    pub fn cover(&self, width: u32, height: u32) -> Self {
        let scale = (f64::from(width) / f64::from(self.width.max(1)))
            .max(f64::from(height) / f64::from(self.height.max(1)));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (sw, sh) = (
            ((f64::from(self.width) * scale).ceil() as u32).max(width),
            ((f64::from(self.height) * scale).ceil() as u32).max(height),
        );
        let big = self.resize(sw, sh);
        let (x0, y0) = ((sw - width) / 2, (sh - height) / 2);
        let row = width as usize * 4;
        let mut pixels = Vec::with_capacity(row * height as usize);
        for y in 0..height {
            let start = ((y + y0) as usize * sw as usize + x0 as usize) * 4;
            pixels.extend_from_slice(&big.pixels[start..start + row]);
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    /// scaled to exactly `width` x `height`, alpha-weighted so transparent
    /// edges do not turn dark.
    #[must_use]
    pub fn resize(&self, width: u32, height: u32) -> Self {
        let (sw, sh) = (self.width as usize, self.height as usize);
        let (dw, dh) = (width as usize, height as usize);
        let mut pixels = vec![0u8; dw * dh * 4];
        for y in 0..dh {
            let y0 = y * sh / dh;
            let y1 = ((y + 1) * sh).div_ceil(dh).clamp(y0 + 1, sh);
            for x in 0..dw {
                let x0 = x * sw / dw;
                let x1 = ((x + 1) * sw).div_ceil(dw).clamp(x0 + 1, sw);
                let mut sum = [0u64; 4];
                let mut count = 0u64;
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let i = (sy * sw + sx) * 4;
                        let a = u64::from(self.pixels[i + 3]);
                        sum[0] += u64::from(self.pixels[i]) * a;
                        sum[1] += u64::from(self.pixels[i + 1]) * a;
                        sum[2] += u64::from(self.pixels[i + 2]) * a;
                        sum[3] += a;
                        count += 1;
                    }
                }
                let o = (y * dw + x) * 4;
                for c in 0..3 {
                    if let Some(v) = sum[c].checked_div(sum[3]) {
                        #[allow(clippy::cast_possible_truncation)]
                        let v = v as u8;
                        pixels[o + c] = v;
                    }
                }
                #[allow(clippy::cast_possible_truncation)]
                let a = (sum[3] / count) as u8;
                pixels[o + 3] = a;
            }
        }
        Self {
            width,
            height,
            pixels,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn rounded_clears_only_the_corners() {
        let square = Rgba {
            width: 20,
            height: 20,
            pixels: [255, 0, 0, 255].repeat(400),
        };
        let round = square.rounded(8);
        let alpha = |x: u32, y: u32| round.pixels[((y * 20 + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(19, 19), 0);
        assert_eq!(alpha(10, 0), 255, "edges stay");
        assert_eq!(alpha(10, 10), 255);
    }

    #[test]
    fn tone_takes_the_colorful_pixels() {
        let solid = |rgba: [u8; 4], n: usize| Rgba {
            width: u32::try_from(n).unwrap(),
            height: 1,
            pixels: rgba.repeat(n),
        };
        let purple = solid([80, 40, 160, 255], 4).tone().unwrap();
        assert_eq!(purple, [115, 58, 230]);
        assert_eq!(solid([90, 90, 90, 255], 4).tone(), None, "gray");
        assert_eq!(solid([200, 0, 0, 0], 4).tone(), None, "clear");
    }

    #[test]
    fn decodes_and_fits_keeping_shape() {
        let red = [255, 0, 0, 255].repeat(4 * 2);
        let image = decode_png(&png(4, 2, &red)).unwrap();
        assert_eq!((image.width, image.height), (4, 2));
        let small = image.fit(2, 2);
        assert_eq!((small.width, small.height), (2, 1), "keeps 2:1");
        assert_eq!(&small.pixels[..4], &[255, 0, 0, 255]);
        let big = image.fit(8, 8);
        assert_eq!((big.width, big.height), (8, 4));
    }

    #[test]
    fn transparent_pixels_do_not_darken_the_average() {
        // one opaque white pixel, one fully transparent black one.
        let image = Rgba {
            width: 2,
            height: 1,
            pixels: vec![255, 255, 255, 255, 0, 0, 0, 0],
        };
        let one = image.resize(1, 1);
        assert_eq!(one.pixels, [255, 255, 255, 127]);
    }

    #[test]
    fn cover_fills_and_crops_the_middle() {
        // 4x1: red, green, green, blue. covering 1x1 keeps the middle.
        let image = Rgba {
            width: 4,
            height: 1,
            pixels: [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
            ]
            .concat(),
        };
        let square = image.cover(2, 2);
        assert_eq!((square.width, square.height), (2, 2));
        assert_eq!(&square.pixels[..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn bitmap_icons_become_a_png_logo() {
        // a 1x1 32-bit icon bitmap: header, one BGRA pixel, one mask row.
        let mut bmp = Vec::new();
        for v in [40u32, 1, 2] {
            bmp.extend_from_slice(&v.to_le_bytes());
        }
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&[30, 20, 10, 255]);
        bmp.extend_from_slice(&[0; 4]);
        let icon = super::super::ico::IconGroup {
            images: vec![bmp],
            group: Vec::new(),
        };
        let logo = decode_png(&logo_from_icon(&icon).unwrap()).unwrap();
        assert_eq!(logo.pixels, [10, 20, 30, 255]);
    }

    #[test]
    fn rejects_what_is_not_png() {
        assert!(decode_png(b"GIF89a").is_err());
    }
}
