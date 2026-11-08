//! Region-edit pixel ops (region-edit spec FR-RGN-03/04, R1): the mask pipeline (pad the
//! inner-rect mask into the outer rect, box-resample it by the fit scale) and the PNG encodes of
//! what is sent. Pure functions on buffers; `pf-core` owns the rules and the errors.

use std::io::Cursor;

use image::RgbImage;

use crate::compose::{ComposeError, PixelRect, try_vec};
use crate::encode::{EncodeError, ImageRows, Phase, encode_png};

/// Why a grey mask PNG can't be used.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GreyDecodeError {
    /// Not 8-bit greyscale (another colour type or bit depth).
    #[error("the mask is not 8-bit greyscale")]
    Depth,
    #[error("the mask is not a readable PNG: {0}")]
    Corrupt(String),
}

/// Decodes an 8-bit greyscale PNG (no alpha) into `(width, height, bytes)`. Anything else is
/// [`GreyDecodeError::Depth`]; no conversion is done.
pub fn decode_grey8_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), GreyDecodeError> {
    let corrupt = |e: &dyn std::fmt::Display| GreyDecodeError::Corrupt(e.to_string());
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| corrupt(&e))?;
    let info = reader.info();
    if info.color_type != png::ColorType::Grayscale || info.bit_depth != png::BitDepth::Eight {
        return Err(GreyDecodeError::Depth);
    }
    let (w, h) = (info.width, info.height);
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| corrupt(&"too large"))?
    ];
    let frame = reader.next_frame(&mut buf).map_err(|e| corrupt(&e))?;
    buf.truncate(frame.buffer_size());
    if buf.len() != w as usize * h as usize {
        return Err(GreyDecodeError::Corrupt("unexpected row layout".into()));
    }
    Ok((w, h, buf))
}

/// Places a `src_w × src_h` grey buffer with its top-left at `(src_x, src_y)` (destination px,
/// may be negative) on a zeroed `dst_w × dst_h` canvas, copying only pixels inside `keep`
/// (destination px). Everything else is 0 (R1: context pixels are always 0).
#[allow(clippy::too_many_arguments)]
pub fn pad_grey(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    src_x: i64,
    src_y: i64,
    dst_w: u32,
    dst_h: u32,
    keep: PixelRect,
) -> Result<Vec<u8>, ComposeError> {
    let mut out = try_vec(dst_w as usize * dst_h as usize)?;
    let x0 = src_x.max(i64::from(keep.x)).max(0);
    let y0 = src_y.max(i64::from(keep.y)).max(0);
    let x1 = (src_x + i64::from(src_w))
        .min(i64::from(keep.x) + i64::from(keep.width))
        .min(i64::from(dst_w));
    let y1 = (src_y + i64::from(src_h))
        .min(i64::from(keep.y) + i64::from(keep.height))
        .min(i64::from(dst_h));
    if x1 <= x0 || y1 <= y0 || src.len() < src_w as usize * src_h as usize {
        return Ok(out);
    }
    let len = (x1 - x0) as usize;
    for y in y0..y1 {
        let s = (y - src_y) as usize * src_w as usize + (x0 - src_x) as usize;
        let d = y as usize * dst_w as usize + x0 as usize;
        out[d..d + len].copy_from_slice(&src[s..s + len]);
    }
    Ok(out)
}

/// Area weights of one axis: for each output index, the first source index and its weights
/// (summing to 1).
fn box_weights(in_len: u32, out_len: u32) -> Vec<(usize, Vec<f32>)> {
    let ratio = f64::from(in_len) / f64::from(out_len);
    (0..out_len)
        .map(|o| {
            let start = f64::from(o) * ratio;
            let end = (f64::from(o) + 1.0) * ratio;
            let first = start.floor() as usize;
            let last = (end.ceil() as usize).min(in_len as usize).max(first + 1);
            let weights = (first..last)
                .map(|i| {
                    let overlap = (end.min(i as f64 + 1.0) - start.max(i as f64)).max(0.0);
                    (overlap / ratio) as f32
                })
                .collect();
            (first, weights)
        })
        .collect()
}

/// Resamples a grey buffer to `out_w × out_h` with a box (area-average) filter (FR-RGN-03).
/// Same size → a copy. All-zero areas stay 0 and all-255 areas stay 255.
pub fn box_resample_grey(
    src: &[u8],
    w: u32,
    h: u32,
    out_w: u32,
    out_h: u32,
) -> Result<Vec<u8>, ComposeError> {
    if (w, h) == (out_w, out_h) {
        let mut out = try_vec(src.len())?;
        out.copy_from_slice(src);
        return Ok(out);
    }
    if w == 0 || h == 0 || out_w == 0 || out_h == 0 {
        return try_vec(out_w as usize * out_h as usize);
    }
    let wx = box_weights(w, out_w);
    let wy = box_weights(h, out_h);
    // Horizontal pass into f32 rows, then the vertical pass straight to bytes.
    let mut mid: Vec<f32> = Vec::new();
    mid.try_reserve_exact(out_w as usize * h as usize)
        .map_err(|_| ComposeError::OutOfMemory)?;
    for y in 0..h as usize {
        let row = &src[y * w as usize..(y + 1) * w as usize];
        mid.extend(wx.iter().map(|(first, ws)| {
            ws.iter()
                .enumerate()
                .map(|(k, wt)| f32::from(row[first + k]) * wt)
                .sum::<f32>()
        }));
    }
    let mut out = try_vec(out_w as usize * out_h as usize)?;
    let mut acc = vec![0f32; out_w as usize];
    for (oy, (first, ws)) in wy.iter().enumerate() {
        acc.iter_mut().for_each(|a| *a = 0.0);
        for (k, wt) in ws.iter().enumerate() {
            let row = &mid[(first + k) * out_w as usize..(first + k + 1) * out_w as usize];
            for (a, v) in acc.iter_mut().zip(row) {
                *a += v * wt;
            }
        }
        let dst = &mut out[oy * out_w as usize..(oy + 1) * out_w as usize];
        for (d, a) in dst.iter_mut().zip(&acc) {
            *d = (a + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    Ok(out)
}

/// Encodes an 8-bit greyscale PNG (no metadata).
pub fn encode_grey8_png(w: u32, h: u32, data: &[u8]) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let failed = |e: png::EncodingError| EncodeError::Failed(e.to_string());
    let mut writer = enc.write_header().map_err(failed)?;
    writer.write_image_data(data).map_err(failed)?;
    writer.finish().map_err(failed)?;
    Ok(out)
}

/// Encodes an RGB8 image as PNG into memory (the sent region, FR-RGN-04).
pub fn encode_rgb8_png(image: &RgbImage) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    encode_png(
        &mut ImageRows::new(image),
        &mut out,
        Phase::Encode,
        &mut |_, _| std::ops::ControlFlow::Continue(()),
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_rgn_03_pad_places_the_mask_and_zeroes_outside_keep() {
        // A 4×3 mask of 9s placed at (2, 1) on a 8×6 canvas, kept only inside x 3..6.
        let src = vec![9u8; 12];
        let out = pad_grey(&src, 4, 3, 2, 1, 8, 6, PixelRect::new(3, 0, 3, 6)).unwrap();
        for y in 0..6 {
            for x in 0..8 {
                let inside = (1..4).contains(&y) && (3..6).contains(&x);
                assert_eq!(out[y * 8 + x], if inside { 9 } else { 0 }, "({x},{y})");
            }
        }
        // Negative offsets clip.
        let out = pad_grey(&src, 4, 3, -2, -1, 3, 3, PixelRect::new(0, 0, 3, 3)).unwrap();
        assert_eq!(out, vec![9, 9, 0, 9, 9, 0, 0, 0, 0]);
    }

    #[test]
    fn fr_rgn_03_box_resample_is_an_area_average() {
        // 4×2 → 2×1: each output averages a 2×2 block.
        let src = vec![0, 255, 255, 255, 0, 255, 255, 255];
        assert_eq!(box_resample_grey(&src, 4, 2, 2, 1).unwrap(), vec![128, 255]);
        // Same size is a copy.
        assert_eq!(box_resample_grey(&src, 4, 2, 4, 2).unwrap(), src);
        // Flat areas stay flat for non-integer ratios, up and down.
        let flat = vec![255u8; 37 * 23];
        assert!(
            box_resample_grey(&flat, 37, 23, 16, 9)
                .unwrap()
                .iter()
                .all(|&v| v == 255)
        );
        assert!(
            box_resample_grey(&flat, 37, 23, 64, 48)
                .unwrap()
                .iter()
                .all(|&v| v == 255)
        );
        let zero = vec![0u8; 37 * 23];
        assert!(
            box_resample_grey(&zero, 37, 23, 16, 9)
                .unwrap()
                .iter()
                .all(|&v| v == 0)
        );
    }

    #[test]
    fn fr_rgn_03_grey_png_round_trips_and_rejects_other_depths() {
        let data: Vec<u8> = (0..30).map(|i| i * 8).collect();
        let png = encode_grey8_png(6, 5, &data).unwrap();
        assert_eq!(decode_grey8_png(&png).unwrap(), (6, 5, data));
        let rgb = encode_rgb8_png(&RgbImage::new(3, 3)).unwrap();
        assert_eq!(decode_grey8_png(&rgb), Err(GreyDecodeError::Depth));
        assert!(matches!(
            decode_grey8_png(b"nope"),
            Err(GreyDecodeError::Corrupt(_))
        ));
    }
}
