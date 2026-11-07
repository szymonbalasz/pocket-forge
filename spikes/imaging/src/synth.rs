//! Procedural battlemap fixture: rooms, corridors, flagstones, walls, props,
//! a 100 px grid, and rough ground outside. Deterministic for a given seed.
//! Any rectangle can be generated without generating the whole map.

use crate::{Args, codecs, mib, ms, row, timed};
use rayon::prelude::*;
use std::path::Path;

pub const GS: i64 = 100; // grid square, px (#11: 70-140 px per square)
const CELL: i64 = 1200; // room cell = 12 squares
const WALL: i64 = 24;

#[inline]
pub fn hash(x: i64, y: i64, s: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ s.wrapping_mul(0xcb1a_b31f)
        ^ ((x >> 32) as u32).wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

#[inline]
pub fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

/// Value noise in [0, 1].
#[inline]
pub fn vnoise(x: f32, y: f32, cell: f32, seed: u32) -> f32 {
    let fx = x / cell;
    let fy = y / cell;
    let ix = fx.floor();
    let iy = fy.floor();
    let tx = fx - ix;
    let ty = fy - iy;
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let (ix, iy) = (ix as i64, iy as i64);
    let a = unit(hash(ix, iy, seed));
    let b = unit(hash(ix + 1, iy, seed));
    let c = unit(hash(ix, iy + 1, seed));
    let d = unit(hash(ix + 1, iy + 1, seed));
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

#[derive(Clone, Copy)]
struct R {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}
impl R {
    #[inline]
    fn has(&self, x: i64, y: i64) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
    #[inline]
    fn near(&self, x: i64, y: i64, d: i64) -> bool {
        x >= self.x0 - d && x < self.x1 + d && y >= self.y0 - d && y < self.y1 + d
    }
}

fn room(cx: i64, cy: i64, seed: u32) -> Option<R> {
    let h = hash(cx, cy, seed ^ 0xA1);
    if h % 5 == 0 {
        return None;
    }
    let ins = |s: u32| 1 + ((h >> s) % 3) as i64;
    Some(R {
        x0: cx * CELL + ins(4) * GS,
        y0: cy * CELL + ins(6) * GS,
        x1: (cx + 1) * CELL - ins(8) * GS,
        y1: (cy + 1) * CELL - ins(10) * GS,
    })
}
fn hcorr(cx: i64, cy: i64, seed: u32) -> Option<R> {
    if hash(cx, cy, seed ^ 0xB2) % 3 == 0 {
        return None;
    }
    let mx = cx * CELL + CELL / 2;
    let my = cy * CELL + CELL / 2;
    Some(R { x0: mx, y0: my - GS, x1: mx + CELL, y1: my + GS })
}
fn vcorr(cx: i64, cy: i64, seed: u32) -> Option<R> {
    if hash(cx, cy, seed ^ 0xC3) % 3 == 0 {
        return None;
    }
    let mx = cx * CELL + CELL / 2;
    let my = cy * CELL + CELL / 2;
    Some(R { x0: mx - GS, y0: my, x1: mx + GS, y1: my + CELL })
}

#[inline]
fn c3(r: f32, g: f32, b: f32) -> [u8; 3] {
    [r.clamp(0.0, 255.0) as u8, g.clamp(0.0, 255.0) as u8, b.clamp(0.0, 255.0) as u8]
}

/// One pixel of the map.
#[inline]
pub fn pixel(x: i64, y: i64, seed: u32) -> [u8; 3] {
    let cx = x.div_euclid(CELL);
    let cy = y.div_euclid(CELL);
    let shapes = [
        room(cx, cy, seed),
        hcorr(cx, cy, seed),
        hcorr(cx - 1, cy, seed),
        vcorr(cx, cy, seed),
        vcorr(cx, cy - 1, seed),
    ];
    let floor = shapes.iter().flatten().any(|r| r.has(x, y));
    let (xf, yf) = (x as f32, y as f32);
    let light = 0.78 + 0.44 * vnoise(xf, yf, 420.0, seed ^ 7);
    let fine = vnoise(xf, yf, 3.0, seed ^ 11) * 26.0 - 13.0;
    if floor {
        // Flagstones, 50 px, offset every other row.
        let sy = y.div_euclid(50);
        let sx = (x + sy.rem_euclid(2) * 25).div_euclid(50);
        let lx = (x + sy.rem_euclid(2) * 25).rem_euclid(50);
        let ly = y.rem_euclid(50);
        let mut c = if lx < 3 || ly < 3 {
            [58.0, 52.0, 47.0]
        } else {
            let t = unit(hash(sx, sy, seed ^ 3));
            let m = vnoise(xf, yf, 17.0, seed ^ 13) * 22.0 - 11.0;
            [128.0 + t * 34.0 + m, 116.0 + t * 26.0 + m, 100.0 + t * 20.0 + m]
        };
        // Props: barrels in some squares.
        let (qx, qy) = (x.div_euclid(GS), y.div_euclid(GS));
        let hp = hash(qx, qy, seed ^ 5);
        if hp % 23 == 0 {
            let (dx, dy) = ((x - qx * GS - 50) as f32, (y - qy * GS - 50) as f32);
            let r = 28.0 + (hp % 12) as f32;
            let d = (dx * dx + dy * dy).sqrt();
            if d < r {
                c = if d > r - 4.0 {
                    [40.0, 30.0, 22.0]
                } else if (d as i32) % 9 < 2 {
                    [86.0, 52.0, 26.0]
                } else {
                    [122.0, 78.0, 38.0]
                };
            }
        }
        let grid = if x.rem_euclid(GS) < 2 || y.rem_euclid(GS) < 2 { 0.72 } else { 1.0 };
        let k = light * grid;
        return c3((c[0] + fine) * k, (c[1] + fine) * k, (c[2] + fine) * k);
    }
    let wall = shapes.iter().flatten().any(|r| r.near(x, y, WALL));
    if wall {
        let by = y.div_euclid(20);
        let bx = (x + by.rem_euclid(2) * 20).div_euclid(40);
        let mortar = y.rem_euclid(20) < 3 || (x + by.rem_euclid(2) * 20).rem_euclid(40) < 3;
        let t = unit(hash(bx, by, seed ^ 17)) * 18.0;
        let c = if mortar { [34.0, 32.0, 30.0] } else { [70.0 + t, 66.0 + t, 62.0 + t] };
        return c3((c[0] + fine) * light, (c[1] + fine) * light, (c[2] + fine) * light);
    }
    // Outside: rough ground and rocks.
    let g = vnoise(xf, yf, 60.0, seed ^ 19) * 0.6 + vnoise(xf, yf, 9.0, seed ^ 23) * 0.4;
    let rock = vnoise(xf, yf, 140.0, seed ^ 29);
    let c = if rock > 0.8 {
        let s = 0.8 + 0.4 * vnoise(xf, yf, 6.0, seed ^ 31);
        [100.0 * s, 100.0 * s, 95.0 * s]
    } else {
        [52.0 + 40.0 * g, 70.0 + 14.0 * g, 38.0 + 20.0 * g]
    };
    c3((c[0] + fine) * light, (c[1] + fine) * light, (c[2] + fine) * light)
}

/// Fill an RGB8 buffer with the rectangle (x0, y0, w, h) of the map, in parallel.
pub fn fill(seed: u32, x0: i64, y0: i64, w: usize, h: usize) -> Vec<u8> {
    let mut buf = vec![0u8; w * h * 3];
    buf.par_chunks_mut(w * 3).enumerate().for_each(|(r, line)| {
        let y = y0 + r as i64;
        for c in 0..w {
            let p = pixel(x0 + c as i64, y, seed);
            line[c * 3..c * 3 + 3].copy_from_slice(&p);
        }
    });
    let _ = h;
    buf
}

/// `gen --size N --dir D [--seed S]`: write map-N.png, map-N.jpg and map-N.webp
/// (WebP is clamped to 16383 px, its format maximum).
pub fn cmd_gen(a: &Args) {
    let n: usize = a.get("size", 8192);
    let seed: u32 = a.get("seed", 1);
    let dir = a.str("dir", ".");
    std::fs::create_dir_all(&dir).unwrap();
    let (img, t_gen) = timed(|| fill(seed, 0, 0, n, n));
    row("gen", &[("size", n.to_string()), ("ms", ms(t_gen))]);

    let only = a.str("only", "all");
    if only == "all" {
        let p = Path::new(&dir).join(format!("map-{n}.png"));
        let (bytes, t) = timed(|| codecs::png_encode(&img, n as u32, n as u32, png::Compression::Balanced));
        std::fs::write(&p, &bytes).unwrap();
        row("gen-png", &[("size", n.to_string()), ("ms", ms(t)), ("mib", mib(bytes.len() as u64))]);

        let p = Path::new(&dir).join(format!("map-{n}.jpg"));
        let (bytes, t) = timed(|| codecs::jpeg_encode(&img, n as u32, n as u32, 90));
        std::fs::write(&p, &bytes).unwrap();
        row("gen-jpeg-q90", &[("size", n.to_string()), ("ms", ms(t)), ("mib", mib(bytes.len() as u64))]);
    }

    let p = Path::new(&dir).join(format!("map-{n}.webp"));
    if n <= 8192 {
        let (bytes, t) = timed(|| codecs::webp_lossy(&img, n as u32, n as u32, 3, 90.0).unwrap());
        std::fs::write(&p, &bytes).unwrap();
        row("gen-webp-q90", &[("size", n.to_string()), ("ms", ms(t)), ("mib", mib(bytes.len() as u64))]);
    } else {
        // libwebp lossy fails at 16383^2 on this content (VP8_ENC_ERROR_PARTITION0_OVERFLOW,
        // even with partition_limit=100), and its encoders stop at 16383 px. VP8L
        // (lossless) allows 16384 px, and the pure-Rust image-webp encoder writes it.
        let (bytes, t) = timed(|| codecs::webp_lossless_pure(&img, n as u32, n as u32, 3).unwrap());
        std::fs::write(&p, &bytes).unwrap();
        row("gen-webp-lossless-pure", &[("size", n.to_string()), ("ms", ms(t)), ("mib", mib(bytes.len() as u64))]);
    }
}
