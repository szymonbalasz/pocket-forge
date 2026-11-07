//! Tile pyramid builder (ADR 0004, EDT Appendix B). Pure: a source file in, tile files out.
//! Where pyramids live, their keys, completion markers and eviction are pf-core's.
//!
//! - 512 px tiles; edge tiles are partial, never padded.
//! - Level 0 = 1:1, lossless WebP. Level n = ⌈size / 2ⁿ⌉, JPEG q90, 2× box filter, up to the
//!   first level that fits one tile.
//! - **Smallest level first** (FR-EDT-07): pass 1 streams the source into an in-memory level 1,
//!   derives levels 2…N from it, and writes N → 1; pass 2 re-streams the source for level 0.
//!   A PNG source is never fully in memory; JPEG/WebP are decoded once.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use image::ImageEncoder;

use crate::probe::ProbeError;
use crate::rgb_source::RgbSource;

pub const TILE_SIZE: u32 = 512;
/// Bump whenever the output changes: it is part of every pyramid key (FR-EDT-10).
pub const BUILDER_VERSION: u32 = 1;
const JPEG_QUALITY: u8 = 90;

/// One pyramid level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelSpec {
    pub level: u32,
    pub width: u32,
    pub height: u32,
    pub cols: u32,
    pub rows: u32,
}

impl LevelSpec {
    /// `webp` for the lossless 1:1 level, `jpg` for the others.
    pub fn ext(&self) -> &'static str {
        if self.level == 0 { "webp" } else { "jpg" }
    }

    pub fn mime(&self) -> &'static str {
        if self.level == 0 {
            "image/webp"
        } else {
            "image/jpeg"
        }
    }

    /// Level px per image px (2⁻ⁿ).
    pub fn scale(&self) -> f64 {
        0.5f64.powi(self.level as i32)
    }
}

/// Level sizes for an image: level 0 … the first level that fits one tile.
pub fn levels(width: u32, height: u32) -> Vec<LevelSpec> {
    let mut out = vec![];
    let (mut w, mut h) = (width.max(1), height.max(1));
    for level in 0.. {
        out.push(LevelSpec {
            level,
            width: w,
            height: h,
            cols: w.div_ceil(TILE_SIZE),
            rows: h.div_ceil(TILE_SIZE),
        });
        if w <= TILE_SIZE && h <= TILE_SIZE {
            break;
        }
        (w, h) = (w.div_ceil(2), h.div_ceil(2));
    }
    out
}

/// `<dir>/<level>/<col>_<row>.<ext>`.
pub fn tile_path(dir: &Path, spec: &LevelSpec, col: u32, row: u32) -> PathBuf {
    dir.join(spec.level.to_string())
        .join(format!("{col}_{row}.{}", spec.ext()))
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error(transparent)]
    Source(#[from] ProbeError),
    #[error("the image is {got:?} px, expected {expected:?}")]
    SizeChanged {
        expected: (u32, u32),
        got: (u32, u32),
    },
    #[error("encoding a tile: {0}")]
    Encode(String),
    #[error("writing tiles: {0}")]
    Io(#[from] std::io::Error),
    /// The `on_level` hook stopped the build (cancel or injected fault).
    #[error("build stopped: {0}")]
    Stopped(String),
}

/// Builds the levels marked in `needed` (indexed by level) from `src` into `dir`.
/// `on_level(l)` runs after every tile of level `l` is written (pf-core writes the completion
/// marker there); an `Err` stops the build. `expected` is the size the caller keyed the pyramid on.
pub fn build(
    src: &Path,
    dir: &Path,
    expected: (u32, u32),
    needed: &[bool],
    on_level: &mut dyn FnMut(u32) -> Result<(), String>,
) -> Result<(), BuildError> {
    let source = RgbSource::open(src)?;
    let got = (source.width, source.height);
    if got != expected {
        return Err(BuildError::SizeChanged { expected, got });
    }
    let specs = levels(got.0, got.1);
    let need = |l: usize| needed.get(l).copied().unwrap_or(true);

    if specs.len() > 1 && (1..specs.len()).any(need) {
        let l1 = specs[1];
        let mut buf = vec![0u8; l1.width as usize * l1.height as usize * 3];
        let out_stride = l1.width as usize * 3;
        // Bands of TILE_SIZE (even) rows, so each band maps to whole level-1 rows.
        source.for_each_band(TILE_SIZE, |y0, rows| -> Result<(), BuildError> {
            let n = rows.len() / (got.0 as usize * 3);
            let (half, oy) = (n.div_ceil(2), y0 as usize / 2);
            downsample2_into(
                rows,
                got.0,
                n as u32,
                &mut buf[oy * out_stride..(oy + half) * out_stride],
            );
            Ok(())
        })?;
        let mut bufs = vec![buf];
        for s in &specs[2..] {
            let prev = &specs[s.level as usize - 1];
            let next = downsample2(&bufs[bufs.len() - 1], prev);
            bufs.push(next);
        }
        for s in specs[1..].iter().rev() {
            if need(s.level as usize) {
                write_level(dir, s, &bufs[s.level as usize - 1])?;
                on_level(s.level).map_err(BuildError::Stopped)?;
            }
        }
    }

    if need(0) {
        let s = specs[0];
        std::fs::create_dir_all(dir.join("0"))?;
        source.for_each_band(TILE_SIZE, |y0, rows| -> Result<(), BuildError> {
            write_band(dir, &s, rows, y0 / TILE_SIZE)
        })?;
        on_level(0).map_err(BuildError::Stopped)?;
    }
    Ok(())
}

/// 2× box filter: each output pixel is the rounded mean of the 1, 2 or 4 source pixels it
/// covers (odd edges average what exists).
pub fn downsample2(src: &[u8], spec: &LevelSpec) -> Vec<u8> {
    let (w2, h2) = (spec.width.div_ceil(2), spec.height.div_ceil(2));
    let mut out = vec![0u8; w2 as usize * h2 as usize * 3];
    downsample2_into(src, spec.width, spec.height, &mut out);
    out
}

fn downsample2_into(src: &[u8], w: u32, h: u32, out: &mut [u8]) {
    let (w, h) = (w as usize, h as usize);
    let w2 = w.div_ceil(2);
    let stride = w * 3;
    for oy in 0..h.div_ceil(2) {
        let r0 = &src[2 * oy * stride..(2 * oy + 1) * stride];
        let r1 = (2 * oy + 1 < h).then(|| &src[(2 * oy + 1) * stride..(2 * oy + 2) * stride]);
        let dst = &mut out[oy * w2 * 3..(oy + 1) * w2 * 3];
        for ox in 0..w2 {
            let x0 = 2 * ox * 3;
            let x1 = (2 * ox + 1 < w).then_some(x0 + 3);
            for c in 0..3 {
                let mut sum = u32::from(r0[x0 + c]);
                let mut n = 1;
                if let Some(x1) = x1 {
                    sum += u32::from(r0[x1 + c]);
                    n += 1;
                }
                if let Some(r1) = r1 {
                    sum += u32::from(r1[x0 + c]);
                    n += 1;
                    if let Some(x1) = x1 {
                        sum += u32::from(r1[x1 + c]);
                        n += 1;
                    }
                }
                dst[ox * 3 + c] = ((sum + n / 2) / n) as u8;
            }
        }
    }
}

/// Writes every tile of an in-memory level.
fn write_level(dir: &Path, s: &LevelSpec, buf: &[u8]) -> Result<(), BuildError> {
    std::fs::create_dir_all(dir.join(s.level.to_string()))?;
    let stride = s.width as usize * 3;
    let rows: Vec<u32> = (0..s.rows).collect();
    par_try(&rows, |&row| {
        let y0 = (row * TILE_SIZE) as usize;
        let y1 = (y0 + TILE_SIZE as usize).min(s.height as usize);
        write_band(dir, s, &buf[y0 * stride..y1 * stride], row)
    })
}

/// Writes the tiles of one tile row from its pixel rows.
fn write_band(dir: &Path, s: &LevelSpec, rows: &[u8], row: u32) -> Result<(), BuildError> {
    let stride = s.width as usize * 3;
    let th = (rows.len() / stride) as u32;
    let cols: Vec<u32> = (0..s.cols).collect();
    par_try(&cols, |&col| {
        let x0 = col * TILE_SIZE;
        let tw = TILE_SIZE.min(s.width - x0);
        let mut tile = Vec::with_capacity(tw as usize * th as usize * 3);
        for r in rows.chunks_exact(stride) {
            tile.extend_from_slice(&r[x0 as usize * 3..(x0 + tw) as usize * 3]);
        }
        let bytes = encode_tile(s, &tile, tw, th)?;
        std::fs::write(tile_path(dir, s, col, row), bytes)?;
        Ok(())
    })
}

fn encode_tile(s: &LevelSpec, rgb: &[u8], w: u32, h: u32) -> Result<Vec<u8>, BuildError> {
    let mut out = Vec::new();
    let color = image::ExtendedColorType::Rgb8;
    let r = if s.level == 0 {
        image::codecs::webp::WebPEncoder::new_lossless(&mut out).write_image(rgb, w, h, color)
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
            .write_image(rgb, w, h, color)
    };
    r.map_err(|e| BuildError::Encode(e.to_string()))?;
    Ok(out)
}

/// Runs `f` over `items` on all cores; the first error wins and stops the rest.
fn par_try<T: Sync>(
    items: &[T],
    f: impl Fn(&T) -> Result<(), BuildError> + Sync,
) -> Result<(), BuildError> {
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(items.len())
        .max(1);
    let next = AtomicUsize::new(0);
    let error = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= items.len() {
                        break;
                    }
                    if let Err(e) = f(&items[i]) {
                        error
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get_or_insert(e);
                        next.store(items.len(), Ordering::Relaxed);
                        break;
                    }
                }
            });
        }
    });
    match error.into_inner().unwrap_or_else(|e| e.into_inner()) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic pattern (gradients + a grid), public-safe.
    pub(crate) fn pattern(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let grid = if x % 70 == 0 || y % 70 == 0 { 255 } else { 0 };
                v.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, grid]);
            }
        }
        v
    }

    fn write_png(path: &Path, w: u32, h: u32, rgb: &[u8]) {
        let f = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
        enc.set_color(png::ColorType::Rgb);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(rgb).unwrap();
    }

    fn decode(path: &Path) -> image::RgbImage {
        image::open(path).unwrap().into_rgb8()
    }

    #[test]
    fn fr_edt_07_level_sizes_stop_at_one_tile() {
        let l = levels(1300, 900);
        let sizes: Vec<_> = l
            .iter()
            .map(|s| (s.width, s.height, s.cols, s.rows))
            .collect();
        assert_eq!(
            sizes,
            [(1300, 900, 3, 2), (650, 450, 2, 1), (325, 225, 1, 1)]
        );
        assert_eq!(levels(512, 512).len(), 1);
        assert_eq!(levels(513, 10).len(), 2);
        let big = levels(16384, 16384);
        assert_eq!(big.len(), 6);
        assert_eq!((big[5].width, big[5].cols), (512, 1));
        assert_eq!((l[0].ext(), l[1].ext()), ("webp", "jpg"));
        assert_eq!(l[2].scale(), 0.25);
    }

    #[test]
    fn box_filter_averages_odd_edges() {
        // 3×3 grey ramp → 2×2.
        let src: Vec<u8> = [0u8, 10, 20, 30, 40, 50, 60, 70, 80]
            .iter()
            .flat_map(|&g| [g, g, g])
            .collect();
        let spec = levels(3, 3)[0];
        let out = downsample2(&src, &spec);
        let g: Vec<u8> = out.chunks(3).map(|p| p[0]).collect();
        // (0+10+30+40)/4=20, (20+50)/2=35, (60+70)/2=65, 80
        assert_eq!(g, [20, 35, 65, 80]);
    }

    #[test]
    fn fr_edt_07_builds_smallest_first_level_0_exact_jpeg_close() {
        let dir = tempfile::tempdir().unwrap();
        let (w, h) = (1300, 900);
        let rgb = pattern(w, h);
        let src = dir.path().join("src.png");
        write_png(&src, w, h, &rgb);
        let out = dir.path().join("pyr");
        let mut order = vec![];
        build(&src, &out, (w, h), &[true; 3], &mut |l| {
            order.push(l);
            Ok(())
        })
        .unwrap();
        assert_eq!(order, [2, 1, 0]);

        let specs = levels(w, h);
        // Level 0: lossless, every tile exactly equal to the source.
        for row in 0..specs[0].rows {
            for col in 0..specs[0].cols {
                let t = decode(&tile_path(&out, &specs[0], col, row));
                let (x0, y0) = (col * TILE_SIZE, row * TILE_SIZE);
                assert_eq!(
                    (t.width(), t.height()),
                    (TILE_SIZE.min(w - x0), TILE_SIZE.min(h - y0))
                );
                for (x, y, p) in t.enumerate_pixels() {
                    let i = (((y0 + y) * w + x0 + x) * 3) as usize;
                    assert_eq!(
                        p.0,
                        [rgb[i], rgb[i + 1], rgb[i + 2]],
                        "({col},{row}) {x},{y}"
                    );
                }
            }
        }
        // JPEG levels: within tolerance of the reference box downsample.
        let mut reference = rgb.clone();
        for s in &specs[1..] {
            reference = downsample2(&reference, &specs[s.level as usize - 1]);
            let mut err_sum = 0u64;
            for row in 0..s.rows {
                for col in 0..s.cols {
                    let t = decode(&tile_path(&out, s, col, row));
                    for (x, y, p) in t.enumerate_pixels() {
                        let i =
                            (((row * TILE_SIZE + y) * s.width + col * TILE_SIZE + x) * 3) as usize;
                        for c in 0..3 {
                            err_sum += u64::from(p.0[c].abs_diff(reference[i + c]));
                        }
                    }
                }
            }
            let mean = err_sum as f64 / f64::from(s.width * s.height * 3);
            assert!(mean < 4.0, "level {} mean error {mean}", s.level);
        }
    }

    #[test]
    fn jpeg_source_is_decoded_whole_and_needed_levels_only() {
        let dir = tempfile::tempdir().unwrap();
        let (w, h) = (700, 300);
        let src = dir.path().join("src.jpg");
        image::RgbImage::from_raw(w, h, pattern(w, h))
            .unwrap()
            .save(&src)
            .unwrap();
        let out = dir.path().join("pyr");
        let mut order = vec![];
        build(&src, &out, (w, h), &[true, false], &mut |l| {
            order.push(l);
            Ok(())
        })
        .unwrap();
        assert_eq!(order, [0]);
        assert!(!out.join("1").exists());
        assert!(matches!(
            build(&src, &out, (w, h + 1), &[true, true], &mut |_| Ok(())),
            Err(BuildError::SizeChanged { .. })
        ));
    }

    #[test]
    fn a_stopping_hook_ends_the_build() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.png");
        write_png(&src, 1100, 600, &pattern(1100, 600));
        let out = dir.path().join("pyr");
        let mut done = vec![];
        let r = build(&src, &out, (1100, 600), &[true; 3], &mut |l| {
            done.push(l);
            if l == 1 { Err("fault".into()) } else { Ok(()) }
        });
        assert!(matches!(r, Err(BuildError::Stopped(_))));
        assert_eq!(done, [2, 1]);
        assert!(!out.join("0").exists());
    }
}
