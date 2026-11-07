//! Generated, public-safe test images (KQ-22): a 512 px "room" with disc and ramp masks
//! (spec §13). Recordings refer to these by name (`{"fixture": "room-512.png"}`), so no binary
//! is committed. Deterministic: the same name always gives the same bytes.

/// Encodes 8-bit pixels as PNG.
pub fn encode_png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(color);
    enc.set_depth(png::BitDepth::Eight);
    // Writing to a Vec with a matching buffer size can't fail.
    if let Ok(mut writer) = enc.write_header() {
        let _ = writer.write_image_data(data);
        let _ = writer.finish();
    }
    out
}

/// A top-down "room": floor tiles, walls and a grid line every 64 px (RGB).
pub fn room_png(side: u32) -> Vec<u8> {
    let data: Vec<u8> = (0..side * side)
        .flat_map(|i| {
            let (x, y) = (i % side, i / side);
            let wall =
                x < side / 16 || y < side / 16 || x >= side - side / 16 || y >= side - side / 16;
            let grid = x % 64 == 0 || y % 64 == 0;
            if wall {
                [70, 62, 55]
            } else if grid {
                [40, 40, 40]
            } else {
                let t = ((x / 32 + y / 32) % 2) as u8 * 18;
                [150 + t, 140 + t, 120 + (x % 7) as u8]
            }
        })
        .collect();
    encode_png(side, side, png::ColorType::Rgb, &data)
}

/// Canonical mask (L8, white = edit): a disc in the centre, radius `side / 4`, hard edge.
pub fn disc_mask_png(side: u32) -> Vec<u8> {
    let c = side as f64 / 2.0;
    let r = side as f64 / 4.0;
    let data: Vec<u8> = (0..side * side)
        .map(|i| {
            let (x, y) = ((i % side) as f64 + 0.5, (i / side) as f64 + 0.5);
            if (x - c).hypot(y - c) <= r { 255 } else { 0 }
        })
        .collect();
    encode_png(side, side, png::ColorType::Grayscale, &data)
}

/// Canonical soft mask: a horizontal ramp from 0 (left) to 255 (right).
pub fn ramp_mask_png(side: u32) -> Vec<u8> {
    let data: Vec<u8> = (0..side * side)
        .map(|i| ((i % side) * 255 / (side - 1).max(1)) as u8)
        .collect();
    encode_png(side, side, png::ColorType::Grayscale, &data)
}

/// Uncompressible RGB noise, used to push a file over the 4 MB upload threshold.
pub fn noise_png(side: u32, seed: u64) -> Vec<u8> {
    let mut z = seed;
    let data: Vec<u8> = (0..side * side * 3)
        .map(|_| {
            z = z
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (z >> 33) as u8
        })
        .collect();
    encode_png(side, side, png::ColorType::Rgb, &data)
}

/// Bytes for a named fixture (`room-512.png`, `disc-512.png`, `ramp-512.png`,
/// `noise-1200.png`, `candidate-<n>-<side>.png`, `not-an-image.png`).
pub fn by_name(name: &str) -> Option<Vec<u8>> {
    let stem = name.strip_suffix(".png")?;
    let (kind, side) = stem.rsplit_once('-')?;
    let side: u32 = side.parse().ok()?;
    match kind {
        "room" => Some(room_png(side)),
        "disc" => Some(disc_mask_png(side)),
        "ramp" => Some(ramp_mask_png(side)),
        "noise" => Some(noise_png(side, 7)),
        k if k.starts_with("candidate-") => {
            let n: u64 = k["candidate-".len()..].parse().ok()?;
            Some(noise_png(side, 100 + n))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_are_deterministic_and_named() {
        assert_eq!(by_name("room-512.png"), Some(room_png(512)));
        assert_eq!(room_png(64), room_png(64));
        assert!(by_name("candidate-2-64.png").is_some());
        assert_ne!(by_name("candidate-1-64.png"), by_name("candidate-2-64.png"));
        assert!(by_name("unknown-1.png").is_none());
        assert!(noise_png(1200, 1).len() > 4 * 1024 * 1024);
    }
}
