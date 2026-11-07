//! Mask conversion (FR-PRV-12, R1): **polarity only**. The input is the canonical mask (L8,
//! white = edit) at exactly the sent image's size, already padded and resampled by RGN. The
//! output is a PNG of the same size in the model's mask convention, after the row's `soft`
//! rule. `alpha_transparent_edit` masks are RGBA carrying the sent image's RGB.

use crate::pixels::{decode_grey8, decode_rgb8, encode_png};
use crate::{MaskConvention, MaskSupport, ProviderError, ProviderResult, SoftRule};

fn invalid(field: &str) -> ProviderError {
    ProviderError::InvalidRequest {
        field: field.into(),
    }
}

/// Converts the canonical mask for a model. Any size other than the sent image's, or a mask
/// that isn't 8-bit greyscale → `invalid_request(mask)`.
pub fn convert_mask(
    mask_png: &[u8],
    image_png: &[u8],
    support: MaskSupport,
) -> ProviderResult<Vec<u8>> {
    let mask = decode_grey8(mask_png).ok_or_else(|| invalid("mask"))?;
    let image = decode_rgb8(image_png).ok_or_else(|| invalid("image"))?;
    if (mask.width, mask.height) != (image.width, image.height) {
        return Err(invalid("mask"));
    }
    let edit = |v: u8| match support.soft {
        SoftRule::PassThrough => v,
        SoftRule::Threshold(t) => {
            if v >= t {
                255
            } else {
                0
            }
        }
    };
    let (w, h) = (mask.width, mask.height);
    Ok(match support.convention {
        MaskConvention::WhiteEdit => {
            let data: Vec<u8> = mask.data.iter().map(|&v| edit(v)).collect();
            encode_png(w, h, png::ColorType::Grayscale, &data)
        }
        MaskConvention::BlackEdit => {
            let data: Vec<u8> = mask.data.iter().map(|&v| 255 - edit(v)).collect();
            encode_png(w, h, png::ColorType::Grayscale, &data)
        }
        MaskConvention::AlphaTransparentEdit => {
            let data: Vec<u8> = mask
                .data
                .iter()
                .zip(image.data.as_chunks::<3>().0)
                .flat_map(|(&v, rgb)| [rgb[0], rgb[1], rgb[2], 255 - edit(v)])
                .collect();
            encode_png(w, h, png::ColorType::Rgba, &data)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{disc_mask_png, encode_png, room_png};

    /// The canonical input row: 0, 64, 127, 128, 200, 255 (1 px high).
    const RAMP: [u8; 6] = [0, 64, 127, 128, 200, 255];

    fn ramp_mask() -> Vec<u8> {
        encode_png(6, 1, png::ColorType::Grayscale, &RAMP)
    }

    fn image() -> Vec<u8> {
        let rgb: Vec<u8> = (0..6u8).flat_map(|i| [i * 10, 100 + i, 200 - i]).collect();
        encode_png(6, 1, png::ColorType::Rgb, &rgb)
    }

    fn decoded(png_bytes: &[u8]) -> (png::ColorType, Vec<u8>) {
        let mut d = png::Decoder::new(std::io::Cursor::new(png_bytes));
        d.set_transformations(png::Transformations::IDENTITY);
        let mut r = d.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info.color_type, buf)
    }

    fn convert(convention: MaskConvention, soft: SoftRule) -> (png::ColorType, Vec<u8>) {
        decoded(&convert_mask(&ramp_mask(), &image(), MaskSupport { convention, soft }).unwrap())
    }

    const T: SoftRule = SoftRule::Threshold(128);
    const P: SoftRule = SoftRule::PassThrough;
    use MaskConvention::*;
    use png::ColorType::{Grayscale, Rgba};

    // One golden per convention × soft rule (decoded pixels, OQ-IMG-07).

    #[test]
    fn fr_prv_12_golden_white_edit_threshold() {
        assert_eq!(
            convert(WhiteEdit, T),
            (Grayscale, vec![0, 0, 0, 255, 255, 255])
        );
    }

    #[test]
    fn fr_prv_12_golden_white_edit_pass_through() {
        assert_eq!(convert(WhiteEdit, P), (Grayscale, RAMP.to_vec()));
    }

    #[test]
    fn fr_prv_12_golden_black_edit_threshold() {
        assert_eq!(
            convert(BlackEdit, T),
            (Grayscale, vec![255, 255, 255, 0, 0, 0])
        );
    }

    #[test]
    fn fr_prv_12_golden_black_edit_pass_through() {
        assert_eq!(
            convert(BlackEdit, P),
            (Grayscale, vec![255, 191, 128, 127, 55, 0])
        );
    }

    #[test]
    fn fr_prv_12_golden_alpha_transparent_edit_pass_through() {
        let (color, px) = convert(AlphaTransparentEdit, P);
        assert_eq!(color, Rgba);
        let alpha: Vec<u8> = px.chunks(4).map(|p| p[3]).collect();
        assert_eq!(alpha, [255, 191, 128, 127, 55, 0], "transparent = edit");
        let rgb: Vec<u8> = px.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        assert_eq!(rgb, decoded(&image()).1, "carries the sent image's RGB");
    }

    #[test]
    fn fr_prv_12_golden_alpha_transparent_edit_threshold() {
        let (_, px) = convert(AlphaTransparentEdit, T);
        let alpha: Vec<u8> = px.chunks(4).map(|p| p[3]).collect();
        assert_eq!(alpha, [255, 255, 255, 0, 0, 0]);
    }

    #[test]
    fn fr_prv_12_output_is_the_sent_size_and_the_wrong_size_is_rejected() {
        let support = MaskSupport {
            convention: WhiteEdit,
            soft: T,
        };
        let out = convert_mask(&disc_mask_png(64), &room_png(64), support).unwrap();
        assert_eq!(crate::pixels::png_size(&out), Some((64, 64)));
        let e = convert_mask(&disc_mask_png(32), &room_png(64), support).unwrap_err();
        assert_eq!(e, invalid("mask"));
        // Not L8 (an RGB "mask") → invalid too.
        let e = convert_mask(&room_png(64), &room_png(64), support).unwrap_err();
        assert_eq!(e, invalid("mask"));
    }
}
