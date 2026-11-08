//! Generated reference-pack fixtures (REF §13, KQ-22): pattern swatches, no real art.
//! Deterministic: the same run always gives byte-identical files.
//!
//!   cargo run -p pf-core --example gen_ref_fixtures            → fixtures/generated/refs/
//!   cargo run -p pf-core --example gen_ref_fixtures -- <dir>   → another folder
//!
//! Writes:
//! - `swatches/`: six 256 px pattern swatches (2 PNG, 2 JPEG, 2 lossless WebP),
//! - `large-5000.jpg`: a 5000 × 3000 JPEG (the FR-REF-22 downscale path),
//! - `truncated.png`: a swatch cut short (fails the end-of-file check, R33),
//! - `Swatch pack/`: a pack folder with three swatches, `pack.json` and a newer conflict copy
//!   `pack-LAPTOP.json` (FR-REF-28). Copy it into `<library-root>/References/`.

use std::path::{Path, PathBuf};

use image::{ImageFormat, Rgb, RgbImage};

fn swatch(i: u32) -> RgbImage {
    let palette = [
        [200, 60, 50],
        [60, 160, 80],
        [50, 90, 200],
        [220, 180, 40],
        [150, 70, 180],
        [40, 170, 170],
    ];
    let c = palette[i as usize % palette.len()];
    RgbImage::from_fn(256, 256, |x, y| {
        let on = match i % 3 {
            0 => (x / 32 + y / 32) % 2 == 0,
            1 => (x + y) / 24 % 2 == 0,
            _ => ((x as i32 - 128).pow(2) + (y as i32 - 128).pow(2)) / 900 % 2 == 0,
        };
        let k = if on { 1.0 } else { 0.45 };
        Rgb(c.map(|v| (f64::from(v) * k) as u8))
    })
}

fn save(img: &RgbImage, path: &Path, format: ImageFormat) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).expect("create folder");
    }
    img.save_with_format(path, format).expect("write image");
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/generated/refs")
        });
    let names = [
        ("swatch-1.png", ImageFormat::Png),
        ("swatch-2.png", ImageFormat::Png),
        ("swatch-3.jpg", ImageFormat::Jpeg),
        ("swatch-4.jpg", ImageFormat::Jpeg),
        ("swatch-5.webp", ImageFormat::WebP),
        ("swatch-6.webp", ImageFormat::WebP),
    ];
    for (i, (name, format)) in names.iter().enumerate() {
        save(&swatch(i as u32), &out.join("swatches").join(name), *format);
    }
    let large = RgbImage::from_fn(5000, 3000, |x, y| {
        Rgb([
            (x * 255 / 4999) as u8,
            (y * 255 / 2999) as u8,
            ((x + y) % 256) as u8,
        ])
    });
    save(&large, &out.join("large-5000.jpg"), ImageFormat::Jpeg);
    let full = out.join("swatches/swatch-1.png");
    let bytes = std::fs::read(&full).expect("read swatch");
    std::fs::write(out.join("truncated.png"), &bytes[..bytes.len() / 2]).expect("write");
    let pack = out.join("Swatch pack");
    for (i, name) in ["swatch-1.png", "swatch-3.jpg", "swatch-5.webp"]
        .iter()
        .enumerate()
    {
        let format = names.iter().find(|(n, _)| n == name).map(|(_, f)| *f);
        save(
            &swatch(i as u32 * 2),
            &pack.join(name),
            format.unwrap_or(ImageFormat::Png),
        );
    }
    let pack_json = |notes: &str, updated: &str| {
        format!(
            "{{\n  \"schema_version\": 1,\n  \"pack_id\": \"01JQ0000000000000000000001\",\n  \"created_at\": \"2027-01-15T08:00:00.000Z\",\n  \"updated_at\": \"{updated}\",\n  \"notes\": \"{notes}\",\n  \"images\": {{\n    \"swatch-3.jpg\": {{\n      \"role\": \"identity\"\n    }}\n  }}\n}}\n"
        )
    };
    std::fs::write(
        pack.join("pack.json"),
        pack_json("Desktop version", "2027-01-15T08:00:00.000Z"),
    )
    .expect("write pack.json");
    std::fs::write(
        pack.join("pack-LAPTOP.json"),
        pack_json("Laptop version (newer)", "2027-01-16T08:00:00.000Z"),
    )
    .expect("write conflict copy");
    println!("reference fixtures written to {}", out.display());
}
