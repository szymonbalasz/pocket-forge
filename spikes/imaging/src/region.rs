//! Region extraction + resize round trip (`region`), patch blend with seam
//! metrics and proof crops (`blend`), and resize determinism (`determinism`).

use crate::{Args, codecs, fnv, ms, row, synth, timed};
use fast_image_resize as fir;
use fir::{CpuExtensions, FilterType, PixelType, ResizeAlg};
use rayon::prelude::*;

pub fn resize(src: &[u8], w: u32, h: u32, dw: u32, dh: u32, alg: ResizeAlg, cpu: Option<CpuExtensions>) -> Vec<u8> {
    let s = fir::images::ImageRef::new(w, h, src, PixelType::U8x3).unwrap();
    let mut d = fir::images::Image::new(dw, dh, PixelType::U8x3);
    let mut r = fir::Resizer::new();
    if let Some(c) = cpu {
        unsafe { r.set_cpu_extensions(c) };
    }
    r.resize(&s, &mut d, &fir::ResizeOptions::new().resize_alg(alg)).unwrap();
    d.into_vec()
}

fn crop(src: &[u8], w: usize, x0: usize, y0: usize, cw: usize, ch: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(cw * ch * 3);
    for y in y0..y0 + ch {
        v.extend_from_slice(&src[(y * w + x0) * 3..(y * w + x0 + cw) * 3]);
    }
    v
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let se: f64 = a.par_iter().zip(b.par_iter()).map(|(&x, &y)| ((x as f64) - (y as f64)).powi(2)).sum();
    let mse = se / a.len() as f64;
    10.0 * (255.0f64 * 255.0 / mse).log10()
}

#[inline]
fn luma(p: &[u8]) -> f32 {
    0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
}

/// Mean absolute Laplacian of luma inside a box: a sharpness proxy.
fn lap(img: &[u8], w: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> f64 {
    let mut s = 0.0f64;
    let mut n = 0u64;
    for y in y0.max(1)..y1 {
        for x in x0.max(1)..x1 {
            let c = luma(&img[(y * w + x) * 3..]);
            let l = luma(&img[(y * w + x - 1) * 3..]);
            let r = luma(&img[(y * w + x + 1) * 3..]);
            let u = luma(&img[((y - 1) * w + x) * 3..]);
            let d = luma(&img[((y + 1) * w + x) * 3..]);
            s += (4.0 * c - l - r - u - d).abs() as f64;
            n += 1;
        }
    }
    s / n as f64
}

fn filters() -> Vec<(&'static str, ResizeAlg)> {
    vec![
        ("nearest", ResizeAlg::Nearest),
        ("bilinear", ResizeAlg::Convolution(FilterType::Bilinear)),
        ("catmullrom", ResizeAlg::Convolution(FilterType::CatmullRom)),
        ("mitchell", ResizeAlg::Convolution(FilterType::Mitchell)),
        ("lanczos3", ResizeAlg::Convolution(FilterType::Lanczos3)),
    ]
}

/// `region --size 16384`: extract regions with context from a 16k source,
/// downscale to provider max input, upscale back; compare filters.
pub fn cmd_region(a: &Args) {
    let n: usize = a.get("size", 16384);
    let ctx: usize = a.get("ctx", 256);
    let (src, t_gen) = timed(|| synth::fill(1, 0, 0, n, n));
    row("region-source", &[("size", n.to_string()), ("gen_ms", ms(t_gen))]);
    for (label, rw, rh) in [("room", 3000usize, 2200usize), ("wing", 6000, 4400)] {
        let (w, h) = (rw + 2 * ctx, rh + 2 * ctx);
        let (x0, y0) = (n / 2 - w / 2, n / 2 - h / 2);
        let (reg, t_crop) = timed(|| crop(&src, n, x0, y0, w, h));
        row("region-extract", &[("region", label.into()), ("w", w.to_string()), ("h", h.to_string()), ("ms", format!("{t_crop:.1}"))]);
        for pmax in [3840usize, 1024] {
            let s = (pmax as f64 / w.max(h) as f64).min(1.0);
            if s >= 1.0 {
                row("region-resize", &[("region", label.into()), ("pmax", pmax.to_string()), ("note", "fits: no resize needed".into())]);
                continue;
            }
            let (pw, ph) = (((w as f64) * s).round() as u32, ((h as f64) * s).round() as u32);
            let base_lap = lap(&reg, w, w / 4, h / 4, 3 * w / 4, 3 * h / 4);
            for (fname, alg) in filters() {
                let (small, t_down) = timed(|| resize(&reg, w as u32, h as u32, pw, ph, alg, None));
                let (up, t_up) = timed(|| resize(&small, pw, ph, w as u32, h as u32, alg, None));
                row("region-resize", &[
                    ("region", label.into()),
                    ("pmax", pmax.to_string()),
                    ("lib", "fast_image_resize".into()),
                    ("filter", fname.into()),
                    ("down", format!("{w}x{h}->{pw}x{ph}")),
                    ("down_ms", format!("{t_down:.1}")),
                    ("up_ms", format!("{t_up:.1}")),
                    ("psnr_db", format!("{:.2}", psnr(&reg, &up))),
                    ("sharpness", format!("{:.2}", lap(&up, w, w / 4, h / 4, 3 * w / 4, 3 * h / 4) / base_lap)),
                ]);
            }
            // The image crate's resizer, for comparison (one filter).
            let buf = image::RgbImage::from_raw(w as u32, h as u32, reg.clone()).unwrap();
            let (small, t_down) = timed(|| image::imageops::resize(&buf, pw, ph, image::imageops::FilterType::Lanczos3));
            let (up, t_up) = timed(|| image::imageops::resize(&small, w as u32, h as u32, image::imageops::FilterType::Lanczos3));
            row("region-resize", &[
                ("region", label.into()),
                ("pmax", pmax.to_string()),
                ("lib", "image::imageops".into()),
                ("filter", "lanczos3".into()),
                ("down_ms", format!("{t_down:.1}")),
                ("up_ms", format!("{t_up:.1}")),
                ("psnr_db", format!("{:.2}", psnr(&reg, up.as_raw()))),
            ]);
        }
    }
}

// ---------- patch blend ----------

/// Canonical mask (ADR 0001): 8-bit, white (255) = editable. A rounded
/// rectangle (superellipse), like a painted room.
pub fn superellipse(w: usize, h: usize, ax: f32, by: f32, p: f32) -> Vec<u8> {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut m = vec![0u8; w * h];
    m.par_chunks_mut(w).enumerate().for_each(|(y, line)| {
        for (x, v) in line.iter_mut().enumerate() {
            let dx = ((x as f32 + 0.5 - cx) / ax).abs();
            let dy = ((y as f32 + 0.5 - cy) / by).abs();
            if dx.powf(p) + dy.powf(p) <= 1.0 {
                *v = 255;
            }
        }
    });
    m
}

fn box_h(src: &[u8], w: usize, r: usize) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    let n = (2 * r + 1) as u32;
    out.par_chunks_mut(w).zip(src.par_chunks(w)).for_each(|(o, s)| {
        let at = |i: isize| s[i.clamp(0, w as isize - 1) as usize] as u32;
        let mut sum: u32 = (-(r as isize)..=r as isize).map(at).sum();
        for x in 0..w {
            o[x] = ((sum + n / 2) / n) as u8;
            sum += at(x as isize + r as isize + 1);
            sum -= at(x as isize - r as isize);
        }
    });
    out
}

fn transpose(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            out[x * h + y] = src[y * w + x];
        }
    }
    out
}

/// Inward feather: alpha is 0 at the mask edge and 255 about `f` px inside.
/// Pixels outside the mask are never changed. Integer box blur x3, deterministic.
pub fn feather(mask: &[u8], w: usize, h: usize, f: usize) -> Vec<u8> {
    if f == 0 {
        return mask.to_vec();
    }
    let r = (f / 3).max(1);
    let mut b = mask.to_vec();
    for _ in 0..3 {
        b = box_h(&b, w, r);
    }
    let mut t = transpose(&b, w, h);
    for _ in 0..3 {
        t = box_h(&t, h, r);
    }
    let b = transpose(&t, h, w);
    b.iter()
        .zip(mask)
        .map(|(&v, &m)| if m == 0 { 0 } else { (2 * v as i32 - 255).clamp(0, 255) as u8 })
        .collect()
}

/// Least-squares per-channel gain/offset that maps the candidate onto the
/// source over the unmasked context (where the provider should not have changed anything).
pub fn colour_match(src: &[u8], cand: &[u8], mask: &[u8]) -> Vec<u8> {
    let mut g = [1.0f64; 3];
    let mut o = [0.0f64; 3];
    for c in 0..3 {
        let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0f64, 0f64, 0f64, 0f64, 0f64);
        for i in (0..mask.len()).step_by(3) {
            if mask[i] == 0 {
                let x = cand[i * 3 + c] as f64;
                let y = src[i * 3 + c] as f64;
                n += 1.0;
                sx += x;
                sy += y;
                sxx += x * x;
                sxy += x * y;
            }
        }
        let var = sxx / n - (sx / n).powi(2);
        let cov = sxy / n - (sx / n) * (sy / n);
        g[c] = (cov / var).clamp(0.8, 1.25);
        o[c] = sy / n - g[c] * sx / n;
    }
    cand.chunks(3)
        .flat_map(|p| {
            let mut q = [0u8; 3];
            for c in 0..3 {
                q[c] = (p[c] as f64 * g[c] + o[c]).round().clamp(0.0, 255.0) as u8;
            }
            q
        })
        .collect()
}

/// Straight-alpha "over" in gamma-encoded sRGB, integer (matches canvas 2D defaults).
pub fn composite_srgb(src: &[u8], cand: &[u8], alpha: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    out.par_chunks_mut(3).enumerate().for_each(|(i, o)| {
        let a = alpha[i] as u32;
        for c in 0..3 {
            o[c] = ((src[i * 3 + c] as u32 * (255 - a) + cand[i * 3 + c] as u32 * a + 127) / 255) as u8;
        }
    });
    out
}

fn s2l(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
fn l2s(l: f32) -> u8 {
    let c = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// The same blend in linear light, to measure how far a linear pipeline would drift from the canvas.
pub fn composite_linear(src: &[u8], cand: &[u8], alpha: &[u8]) -> Vec<u8> {
    let lut: Vec<f32> = (0..=255).map(|v| s2l(v as u8)).collect();
    let mut out = vec![0u8; src.len()];
    out.par_chunks_mut(3).enumerate().for_each(|(i, o)| {
        let a = alpha[i] as f32 / 255.0;
        for c in 0..3 {
            let l = lut[src[i * 3 + c] as usize] * (1.0 - a) + lut[cand[i * 3 + c] as usize] * a;
            o[c] = l2s(l);
        }
    });
    out
}

fn kitchen(x: i64, y: i64) -> [f32; 3] {
    let n = synth::vnoise(x as f32, y as f32, 5.0, 77) * 20.0 - 10.0;
    if (x.div_euclid(40) + y.div_euclid(40)) % 2 == 0 {
        [196.0 + n, 186.0 + n, 160.0 + n]
    } else {
        [92.0 + n, 64.0 + n, 48.0 + n]
    }
}

/// Fake provider: downscale the region to the provider's max input, repaint
/// the inner (edit) area, add colour drift and noise everywhere, upscale back.
fn simulate(src: &[u8], w: usize, h: usize, edit: &[u8], pmax: usize) -> Vec<u8> {
    let s = (pmax as f64 / w.max(h) as f64).min(1.0);
    let (pw, ph) = (((w as f64) * s).round() as usize, ((h as f64) * s).round() as usize);
    let lz = ResizeAlg::Convolution(FilterType::Lanczos3);
    let mut small = if s < 1.0 { resize(src, w as u32, h as u32, pw as u32, ph as u32, lz, None) } else { src.to_vec() };
    small.par_chunks_mut(pw * 3).enumerate().for_each(|(py, line)| {
        for px in 0..pw {
            let fx = ((px as f64 + 0.5) / s) as usize;
            let fy = ((py as f64 + 0.5) / s) as usize;
            let p = &mut line[px * 3..px * 3 + 3];
            let mut c = [p[0] as f32, p[1] as f32, p[2] as f32];
            if edit[fy.min(h - 1) * w + fx.min(w - 1)] > 0 {
                c = kitchen(fx as i64, fy as i64);
            }
            let nz = (synth::hash(px as i64, py as i64, 99) % 7) as f32 - 3.0;
            p[0] = (c[0] * 1.06 + 5.0 + nz).clamp(0.0, 255.0) as u8;
            p[1] = (c[1] * 1.01 + 2.0 + nz).clamp(0.0, 255.0) as u8;
            p[2] = (c[2] * 0.94 - 2.0 + nz).clamp(0.0, 255.0) as u8;
        }
    });
    if s < 1.0 {
        resize(&small, pw as u32, ph as u32, w as u32, h as u32, ResizeAlg::Convolution(FilterType::CatmullRom), None)
    } else {
        small
    }
}

/// Column-mean luma of (out - src) across the left mask edge. Returns
/// (max step between adjacent columns, total offset inside vs outside).
fn edge_profile(out: &[u8], src: &[u8], w: usize, h: usize, ex: usize) -> (f64, f64) {
    let (y0, y1) = (h * 3 / 8, h * 5 / 8);
    let xs: Vec<usize> = (ex - 96..ex + 96).collect();
    let p: Vec<f64> = xs
        .iter()
        .map(|&x| {
            let mut s = 0.0;
            for y in y0..y1 {
                let i = (y * w + x) * 3;
                s += (luma(&out[i..]) - luma(&src[i..])) as f64;
            }
            s / (y1 - y0) as f64
        })
        .collect();
    let step = p.windows(2).map(|q| (q[1] - q[0]).abs()).fold(0.0, f64::max);
    let inside: f64 = p[160..].iter().sum::<f64>() / 32.0;
    let outside: f64 = p[..32].iter().sum::<f64>() / 32.0;
    (step, (inside - outside).abs())
}

/// `blend --proof P.png`: composite a fake candidate with hard / feathered /
/// colour-matched edges, print seam metrics and write a proof image.
pub fn cmd_blend(a: &Args) {
    let proof = a.str("proof", "seam-proof.png");
    let ctx = 256usize;
    let (rw, rh) = (3000usize, 2200usize);
    let (w, h) = (rw + 2 * ctx, rh + 2 * ctx);
    let src = synth::fill(1, 5200, 4300, w, h);
    let (ax, by) = ((rw / 2 - 100) as f32, (rh / 2 - 100) as f32);
    let mask = superellipse(w, h, ax, by, 8.0);
    let edit = superellipse(w, h, ax - 160.0, by - 160.0, 8.0);
    let ex = (w as f32 / 2.0 - ax).round() as usize; // left mask edge on the middle row
    let cs = 176usize; // proof crop size
    let (cx0, cy0) = (ex - cs / 2, h / 2 - cs / 2);
    let mut panels: Vec<Vec<u8>> = Vec::new();
    for pmax in [3840usize, 1024] {
        let cand = simulate(&src, w, h, &edit, pmax);
        let drift_before = drift(&src, &cand, &mask);
        let (cm, t_cm) = timed(|| colour_match(&src, &cand, &mask));
        let drift_after = drift(&src, &cm, &mask);
        row("blend-colour-match", &[
            ("pmax", pmax.to_string()),
            ("ms", ms(t_cm)),
            ("context_drift_before", format!("{drift_before:.2}")),
            ("context_drift_after", format!("{drift_after:.2}")),
        ]);
        panels.push(crop(&src, w, cx0, cy0, cs, cs));
        for (label, f, use_cm) in [("hard", 0usize, false), ("feather16", 16, false), ("feather48", 48, false), ("feather48+cm", 48, true)] {
            let (alpha, t_f) = timed(|| feather(&mask, w, h, f));
            let c = if use_cm { &cm } else { &cand };
            let (out, t_c) = timed(|| composite_srgb(&src, c, &alpha));
            let (step, offset) = edge_profile(&out, &src, w, h, ex);
            let sharp = lap(&out, w, ex + 70, h * 3 / 8, ex + 140, h * 5 / 8) / lap(&src, w, ex + 70, h * 3 / 8, ex + 140, h * 5 / 8);
            row("blend", &[
                ("pmax", pmax.to_string()),
                ("variant", label.into()),
                ("region", format!("{w}x{h}")),
                ("feather_ms", ms(t_f)),
                ("composite_ms", ms(t_c)),
                ("edge_max_step_levels_per_px", format!("{step:.2}")),
                ("edge_offset_levels", format!("{offset:.2}")),
                ("sharpness_inside", format!("{sharp:.2}")),
            ]);
            if label != "feather16" {
                panels.push(crop(&out, w, cx0, cy0, cs, cs));
            }
            if label == "feather48+cm" {
                let lin = composite_linear(&src, c, &alpha);
                let (mut maxd, mut sum, mut n) = (0u8, 0u64, 0u64);
                for i in 0..alpha.len() {
                    if alpha[i] > 0 && alpha[i] < 255 {
                        for ch in 0..3 {
                            let d = out[i * 3 + ch].abs_diff(lin[i * 3 + ch]);
                            maxd = maxd.max(d);
                            sum += d as u64;
                            n += 1;
                        }
                    }
                }
                row("parity-srgb-vs-linear", &[
                    ("pmax", pmax.to_string()),
                    ("feather_zone_max_diff", maxd.to_string()),
                    ("feather_zone_mean_diff", format!("{:.2}", sum as f64 / n as f64)),
                ]);
            }
        }
    }
    // Proof image: 2 rows (provider max 3840 px, 1024 px) x 4 panels
    // (source, hard edge, feather 48, feather 48 + colour match). 4 px gutters.
    let g = 4;
    let (pw, ph) = (4 * cs + 3 * g, 2 * cs + g);
    let mut img = vec![255u8; pw * ph * 3];
    for (k, p) in panels.iter().enumerate() {
        let (r, c) = (k / 4, k % 4);
        for y in 0..cs {
            let dst = ((r * (cs + g) + y) * pw + c * (cs + g)) * 3;
            img[dst..dst + cs * 3].copy_from_slice(&p[y * cs * 3..(y + 1) * cs * 3]);
        }
    }
    let bytes = codecs::png_encode(&img, pw as u32, ph as u32, png::Compression::High);
    std::fs::write(&proof, &bytes).unwrap();
    row("blend-proof", &[("file", proof), ("bytes", bytes.len().to_string())]);
}

fn drift(src: &[u8], cand: &[u8], mask: &[u8]) -> f64 {
    let (mut s, mut n) = (0f64, 0f64);
    for i in 0..mask.len() {
        if mask[i] == 0 {
            for c in 0..3 {
                s += (cand[i * 3 + c] as f64 - src[i * 3 + c] as f64).abs();
                n += 1.0;
            }
        }
    }
    s / n
}

/// `determinism`: fast_image_resize output across SIMD back-ends and runs.
pub fn cmd_determinism(_a: &Args) {
    let (w, h) = (4000u32, 3000u32);
    let src = synth::fill(3, 0, 0, w as usize, h as usize);
    let mut exts = vec![("none", CpuExtensions::None)];
    #[cfg(target_arch = "x86_64")]
    {
        exts.push(("sse4.1", CpuExtensions::Sse4_1));
        exts.push(("avx2", CpuExtensions::Avx2));
    }
    for (fname, alg) in filters() {
        let mut hashes = Vec::new();
        for (ename, e) in &exts {
            let d = resize(&src, w, h, 1024, 768, alg, Some(*e));
            let u = resize(&d, 1024, 768, w, h, alg, Some(*e));
            hashes.push(format!("{ename}:{:016x}/{:016x}", fnv(&d), fnv(&u)));
        }
        let first = hashes[0].split(':').nth(1).unwrap().to_string();
        let same = hashes.iter().all(|s| s.ends_with(&first));
        row("determinism-fir", &[("filter", fname.into()), ("identical_across_simd", same.to_string()), ("hashes", hashes.join(","))]);
    }
    // Feather + colour match + composite twice: must be identical.
    let m = superellipse(1200, 900, 500.0, 350.0, 8.0);
    let s = synth::fill(4, 0, 0, 1200, 900);
    let c = synth::fill(5, 0, 0, 1200, 900);
    let run = || {
        let al = feather(&m, 1200, 900, 48);
        let cm = colour_match(&s, &c, &m);
        fnv(&composite_srgb(&s, &cm, &al))
    };
    let (h1, h2) = (run(), run());
    row("determinism-blend", &[("identical", (h1 == h2).to_string()), ("hash", format!("{h1:016x}"))]);
}
