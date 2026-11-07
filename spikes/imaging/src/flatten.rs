//! Flatten + export: base image plus N patch layers (position, scale,
//! rotation, opacity), composed in horizontal strips with tiny-skia and
//! streamed into the encoder. A strip as tall as the image = the full-buffer case.

use crate::{Args, Fnv, codecs, mib, ms, region, row, synth, timed};
use rayon::prelude::*;
use std::cell::RefCell;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use tiny_skia::{FilterQuality, IntSize, Pixmap, PixmapPaint, Point, Transform};

pub struct Layer {
    pm: Pixmap,
    t: Transform,
    opacity: f32,
    y0: f32,
    y1: f32,
}

/// Deterministic patch layers: half axis-aligned at integer positions (the
/// usual accepted patch), half scaled and rotated.
pub fn make_layers(n: usize, size: usize, seed: u32) -> Vec<Layer> {
    (0..n)
        .map(|i| {
            let h = synth::hash(i as i64, 0, seed);
            let lw = 1200 + (h % 1800) as usize;
            let lh = 900 + ((h >> 11) % 1400) as usize;
            let rgb = synth::fill(seed + 100 + i as u32, (i * 7919) as i64, (i * 104_729) as i64, lw, lh);
            let mask = region::superellipse(lw, lh, lw as f32 / 2.0 - 8.0, lh as f32 / 2.0 - 8.0, 6.0);
            let alpha = region::feather(&mask, lw, lh, 48);
            let mut data = vec![0u8; lw * lh * 4];
            for p in 0..lw * lh {
                let a = alpha[p] as u32;
                for c in 0..3 {
                    data[p * 4 + c] = ((rgb[p * 3 + c] as u32 * a + 127) / 255) as u8;
                }
                data[p * 4 + 3] = a as u8;
            }
            let pm = Pixmap::from_vec(data, IntSize::from_wh(lw as u32, lh as u32).unwrap()).unwrap();
            let u = |s: u32| synth::unit(synth::hash(i as i64, s as i64, seed));
            let px = (u(1) * (size as f32 - lw as f32)).floor();
            let py = (u(2) * (size as f32 - lh as f32)).floor();
            let t = if i % 2 == 0 {
                Transform::from_translate(px, py)
            } else {
                let s = 0.75 + u(3) * 0.75;
                let rot = -20.0 + u(4) * 40.0;
                Transform::from_translate(px, py).pre_rotate(rot).pre_scale(s, s)
            };
            let mut pts = [
                Point::from_xy(0.0, 0.0),
                Point::from_xy(lw as f32, 0.0),
                Point::from_xy(0.0, lh as f32),
                Point::from_xy(lw as f32, lh as f32),
            ];
            t.map_points(&mut pts);
            let y0 = pts.iter().map(|p| p.y).fold(f32::MAX, f32::min) - 2.0;
            let y1 = pts.iter().map(|p| p.y).fold(f32::MIN, f32::max) + 2.0;
            Layer { pm, t, opacity: 0.7 + 0.3 * u(5), y0, y1 }
        })
        .collect()
}

enum Base {
    Png(png::Reader<BufReader<File>>),
    Synth(u32),
}

pub struct Strips<'a> {
    w: usize,
    h: usize,
    sh: usize,
    k: usize,
    layers: &'a [Layer],
    base: Base,
    strips: Vec<Pixmap>,
    y0: usize, // first row held
    rows: usize,
    pub t_base: f64,
    pub t_compose: f64,
    pub hash: Fnv,
}

impl<'a> Strips<'a> {
    /// Make sure row `y` is composed. Rows must be requested in order.
    fn ensure(&mut self, y: usize) {
        if y >= self.y0 && y < self.y0 + self.rows {
            return;
        }
        let start = self.y0 + self.rows;
        assert!(y >= start, "rows must be requested in order");
        let total = (self.sh * self.k).min(self.h - start);
        let (w, sh) = (self.w, self.sh);
        // 1. Base rows into the strip pixmaps (opaque RGBA, premultiplied = straight).
        let t = std::time::Instant::now();
        match &mut self.base {
            Base::Png(r) => {
                for i in 0..total {
                    let row = r.next_row().unwrap().unwrap();
                    let src = row.data();
                    let dst = &mut self.strips[i / sh].data_mut()[(i % sh) * w * 4..][..w * 4];
                    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                        d[0] = s[0];
                        d[1] = s[1];
                        d[2] = s[2];
                        d[3] = 255;
                    }
                }
            }
            Base::Synth(seed) => {
                let rgb = synth::fill(*seed, 0, start as i64, w, total);
                for i in 0..total {
                    let src = &rgb[i * w * 3..][..w * 3];
                    let dst = &mut self.strips[i / sh].data_mut()[(i % sh) * w * 4..][..w * 4];
                    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                        d[..3].copy_from_slice(s);
                        d[3] = 255;
                    }
                }
            }
        }
        self.t_base += t.elapsed().as_secs_f64() * 1000.0;
        // 2. Draw the layers that touch each strip, strips in parallel.
        let t = std::time::Instant::now();
        let layers = self.layers;
        let n_strips = total.div_ceil(sh);
        self.strips[..n_strips].par_iter_mut().enumerate().for_each(|(i, pm)| {
            let sy = (start + i * sh) as f32;
            let ey = sy + sh as f32;
            for l in layers {
                if l.y1 < sy || l.y0 > ey {
                    continue;
                }
                let paint = PixmapPaint { opacity: l.opacity, quality: FilterQuality::Bicubic, ..Default::default() };
                pm.draw_pixmap(0, 0, l.pm.as_ref(), &paint, l.t.post_translate(0.0, -sy), None);
            }
        });
        self.t_compose += t.elapsed().as_secs_f64() * 1000.0;
        self.y0 = start;
        self.rows = total;
    }

    /// Row `y` as RGBA (opaque).
    fn row(&mut self, y: usize) -> &[u8] {
        self.ensure(y);
        let i = y - self.y0;
        &self.strips[i / self.sh].data()[(i % self.sh) * self.w * 4..][..self.w * 4]
    }

    fn row_rgb(&mut self, y: usize, out: &mut Vec<u8>) {
        out.clear();
        let r = self.row(y);
        for p in r.chunks_exact(4) {
            out.extend_from_slice(&p[..3]);
        }
        self.hash.update(out);
    }
}

struct JpegSrc<'s, 'a>(RefCell<&'s mut Strips<'a>>);

impl jpeg_encoder::ImageBuffer for JpegSrc<'_, '_> {
    fn get_jpeg_color_type(&self) -> jpeg_encoder::JpegColorType {
        jpeg_encoder::JpegColorType::Ycbcr
    }
    fn width(&self) -> u16 {
        self.0.borrow().w as u16
    }
    fn height(&self) -> u16 {
        self.0.borrow().h as u16
    }
    fn fill_buffers(&self, y: u16, b: &mut [Vec<u8>; 4]) {
        let mut s = self.0.borrow_mut();
        let y = y as usize;
        let r = s.row(y).to_vec();
        for p in r.chunks_exact(4) {
            let (yy, cb, cr) = jpeg_encoder::rgb_to_ycbcr(p[0], p[1], p[2]);
            b[0].push(yy);
            b[1].push(cb);
            b[2].push(cr);
        }
    }
}

/// `flatten --size 16384 --layers 8 --strip 512 --par 8 --fmt png-fast|png|jpeg|webp|webp-ll|none --base synth|<file.png> --out F`
pub fn cmd_flatten(a: &Args) {
    let size: usize = a.get("size", 16384);
    let n_layers: usize = a.get("layers", 8);
    let sh: usize = a.get::<usize>("strip", 512).min(size);
    let k: usize = a.get::<usize>("par", 8).min(size.div_ceil(sh));
    let fmt = a.str("fmt", "png-fast");
    let base_s = a.str("base", "synth");
    let out_path = a.str("out", "flat.out");

    let (layers, t_layers) = timed(|| make_layers(n_layers, size, 42));
    let (base, w, h) = if base_s == "synth" {
        (Base::Synth(1), size, size)
    } else {
        let r = codecs::png_row_reader(&base_s);
        let (w, h) = (r.info().width as usize, r.info().height as usize);
        (Base::Png(r), w, h)
    };
    let strips = (0..k).map(|_| Pixmap::new(w as u32, sh as u32).unwrap()).collect();
    let mut s = Strips { w, h, sh, k, layers: &layers, base, strips, y0: 0, rows: 0, t_base: 0.0, t_compose: 0.0, hash: Fnv::new() };

    let (res, t_all) = timed(|| -> Result<u64, String> {
        let mut rgb = Vec::with_capacity(w * 3);
        match fmt.as_str() {
            "png-fast" | "png" => {
                let f = BufWriter::with_capacity(1 << 22, File::create(&out_path).unwrap());
                let mut e = png::Encoder::new(f, w as u32, h as u32);
                e.set_color(png::ColorType::Rgb);
                e.set_depth(png::BitDepth::Eight);
                e.set_compression(if fmt == "png" { png::Compression::Balanced } else { png::Compression::Fast });
                let mut wr = e.write_header().unwrap();
                let mut sw = wr.stream_writer_with_size(1 << 22).unwrap();
                for y in 0..h {
                    s.row_rgb(y, &mut rgb);
                    sw.write_all(&rgb).unwrap();
                }
                sw.finish().unwrap();
            }
            "jpeg" => {
                if w > 65535 || h > 65535 {
                    return Err("JPEG max is 65535 px per side".into());
                }
                let f = BufWriter::with_capacity(1 << 22, File::create(&out_path).unwrap());
                let e = jpeg_encoder::Encoder::new(f, 90);
                let src = JpegSrc(RefCell::new(&mut s));
                e.encode_image(src).map_err(|e| e.to_string())?;
                // Hash for JPEG is not comparable (rows are visited with padding repeats).
            }
            "webp" | "webp-ll" | "webp-ll-libwebp" | "none" => {
                // Encoders that need the whole image: collect full RGB first.
                let mut full = if fmt == "none" { Vec::new() } else { Vec::with_capacity(w * h * 3) };
                for y in 0..h {
                    s.row_rgb(y, &mut rgb);
                    if fmt != "none" {
                        full.extend_from_slice(&rgb);
                    }
                }
                if fmt != "none" {
                    let bytes = match fmt.as_str() {
                        "webp" => codecs::webp_lossy(&full, w as u32, h as u32, 3, 90.0)?,
                        "webp-ll" => codecs::webp_lossless_pure(&full, w as u32, h as u32, 3)?,
                        _ => codecs::webp_lossless_libwebp(&full, w as u32, h as u32)?,
                    };
                    std::fs::write(&out_path, bytes).unwrap();
                }
            }
            _ => return Err("unknown fmt".into()),
        }
        Ok(std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0))
    });
    let mut kv = vec![
        ("size", format!("{w}x{h}")),
        ("layers", n_layers.to_string()),
        ("strip", sh.to_string()),
        ("par", k.to_string()),
        ("fmt", fmt.clone()),
        ("base", if base_s == "synth" { "synth".into() } else { "png-stream".into() }),
        ("layers_ms", ms(t_layers)),
        ("base_ms", ms(s.t_base)),
        ("compose_ms", ms(s.t_compose)),
        ("total_ms", ms(t_all)),
    ];
    match res {
        Ok(b) => {
            kv.push(("out_mib", mib(b)));
            if fmt != "jpeg" {
                kv.push(("pixel_hash", format!("{:016x}", s.hash.0)));
            }
        }
        Err(e) => kv.push(("error", e)),
    }
    row("flatten", &kv);
}
