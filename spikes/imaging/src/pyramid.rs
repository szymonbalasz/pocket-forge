//! Tile pyramid to a disk cache: level 0 = full resolution, each next level is
//! a 2x2 box downscale, until one tile covers the image.

use crate::{Args, codecs, mib, ms, row, timed};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// 2x2 box downscale of RGB8 (odd edges clamp). Integer, deterministic.
pub fn half(src: &[u8], w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let (w2, h2) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = vec![0u8; w2 * h2 * 3];
    out.par_chunks_mut(w2 * 3).enumerate().for_each(|(y, line)| {
        let y0 = (2 * y).min(h - 1);
        let y1 = (2 * y + 1).min(h - 1);
        let (r0, r1) = (&src[y0 * w * 3..][..w * 3], &src[y1 * w * 3..][..w * 3]);
        for x in 0..w2 {
            let x0 = (2 * x).min(w - 1) * 3;
            let x1 = (2 * x + 1).min(w - 1) * 3;
            for c in 0..3 {
                let s = r0[x0 + c] as u32 + r0[x1 + c] as u32 + r1[x0 + c] as u32 + r1[x1 + c] as u32;
                line[x * 3 + c] = ((s + 2) / 4) as u8;
            }
        }
    });
    (out, w2, h2)
}

fn crop(src: &[u8], w: usize, x0: usize, y0: usize, tw: usize, th: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(tw * th * 3);
    for y in y0..y0 + th {
        v.extend_from_slice(&src[(y * w + x0) * 3..(y * w + x0 + tw) * 3]);
    }
    v
}

pub fn ext(fmt: &str) -> &'static str {
    match fmt {
        "jpeg" => "jpg",
        "png" | "png-fast" => "png",
        _ => "webp",
    }
}

pub fn encode_tile(fmt: &str, rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    match fmt {
        "jpeg" => codecs::jpeg_encode(rgb, w, h, 85),
        "webp" => codecs::webp_lossy(rgb, w, h, 3, 80.0).unwrap(),
        "webp-ll" => codecs::webp_lossless_pure(rgb, w, h, 3).unwrap(),
        "png-fast" => codecs::png_encode(rgb, w, h, png::Compression::Fast),
        "png" => codecs::png_encode(rgb, w, h, png::Compression::Balanced),
        _ => panic!("unknown fmt"),
    }
}

/// Encode and write all tiles of one level (or of one strip of level 0).
/// Returns (tile count, bytes).
fn emit(dir: &Path, level: u32, img: &[u8], w: usize, h: usize, y_off: usize, tile: usize, fmt: &str) -> (u64, u64) {
    let ld = dir.join(level.to_string());
    std::fs::create_dir_all(&ld).unwrap();
    let mut coords = Vec::new();
    for ty in (0..h).step_by(tile) {
        for tx in (0..w).step_by(tile) {
            coords.push((tx, ty));
        }
    }
    coords
        .par_iter()
        .map(|&(tx, ty)| {
            let (tw, th) = (tile.min(w - tx), tile.min(h - ty));
            let px = crop(img, w, tx, ty, tw, th);
            let b = encode_tile(fmt, &px, tw as u32, th as u32);
            let p = ld.join(format!("{}_{}.{}", tx / tile, (ty + y_off) / tile, ext(fmt)));
            std::fs::write(p, &b).unwrap();
            (1u64, b.len() as u64)
        })
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
}

/// Levels 1.. from an in-memory level. Returns (levels, tiles, bytes).
fn rest(dir: &Path, mut lvl: u32, mut img: Vec<u8>, mut w: usize, mut h: usize, tile: usize, fmt: &str) -> (u32, u64, u64) {
    let (mut n, mut b) = (0, 0);
    loop {
        let (tn, tb) = emit(dir, lvl, &img, w, h, 0, tile, fmt);
        n += tn;
        b += tb;
        if w <= tile && h <= tile {
            return (lvl + 1, n, b);
        }
        let (i2, w2, h2) = half(&img, w, h);
        img = i2;
        w = w2;
        h = h2;
        lvl += 1;
    }
}

/// `pyramid --file F --fmt jpeg|webp|webp-ll|png|png-fast --tile 512 --out D --mode full|stream`
pub fn cmd_pyramid(a: &Args) {
    let file = a.str("file", "");
    let fmt = a.str("fmt", "jpeg");
    let tile: usize = a.get("tile", 512);
    let mode = a.str("mode", "full");
    let dir = PathBuf::from(a.str("out", "tiles")).join(format!("{mode}-{fmt}-{tile}"));
    let _ = std::fs::remove_dir_all(&dir);
    let name = Path::new(&file).file_name().unwrap().to_string_lossy().to_string();

    let ((levels, tiles, bytes, t_dec), t_all) = timed(|| {
        if mode == "full" {
            let ((w, h, img), t_dec) = timed(|| codecs::decode_rgb(&file));
            let (l, n, b) = rest(&dir, 0, img, w as usize, h as usize, tile, &fmt);
            (l, n, b, t_dec)
        } else {
            // Stream a PNG row by row: emit level-0 tiles per strip, keep only
            // the half-resolution level in memory.
            let mut r = codecs::png_row_reader(&file);
            let (w, h) = (r.info().width as usize, r.info().height as usize);
            assert_eq!(r.info().color_type, png::ColorType::Rgb, "stream mode expects RGB PNG");
            let mut strip = Vec::with_capacity(w * tile * 3);
            let mut l1 = Vec::with_capacity(w.div_ceil(2) * h.div_ceil(2) * 3);
            let (mut n, mut b) = (0, 0);
            let mut y = 0;
            while y < h {
                strip.clear();
                let sh = tile.min(h - y);
                for _ in 0..sh {
                    strip.extend_from_slice(r.next_row().unwrap().unwrap().data());
                }
                let (tn, tb) = emit(&dir, 0, &strip, w, sh, y, tile, &fmt);
                n += tn;
                b += tb;
                let (hs, _, _) = half(&strip, w, sh);
                l1.extend_from_slice(&hs);
                y += sh;
            }
            let (l, n2, b2) = rest(&dir, 1, l1, w.div_ceil(2), h.div_ceil(2), tile, &fmt);
            (l, n + n2, b + b2, f64::NAN)
        }
    });

    // Decode cost: decode up to 128 level-0 tiles on one thread.
    let l0 = dir.join("0");
    let files: Vec<PathBuf> = std::fs::read_dir(&l0).unwrap().take(128).map(|e| e.unwrap().path()).collect();
    let datas: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let (_, t_d) = timed(|| {
        for d in &datas {
            let img = image::load_from_memory(d).unwrap();
            std::hint::black_box(img);
        }
    });
    let mut libwebp_us = String::from("-");
    if fmt == "webp" {
        let (_, t) = timed(|| {
            for d in &datas {
                std::hint::black_box(codecs::webp_decode_libwebp(d).unwrap());
            }
        });
        libwebp_us = format!("{:.0}", t * 1000.0 / datas.len() as f64);
    }
    row("pyramid", &[
        ("file", name),
        ("mode", mode),
        ("fmt", fmt),
        ("tile", tile.to_string()),
        ("levels", levels.to_string()),
        ("tiles", tiles.to_string()),
        ("mib", mib(bytes)),
        ("decode_ms", ms(t_dec)),
        ("total_ms", ms(t_all)),
        ("tile_decode_us_rust", format!("{:.0}", t_d * 1000.0 / datas.len() as f64)),
        ("tile_decode_us_libwebp", libwebp_us),
    ]);
}
