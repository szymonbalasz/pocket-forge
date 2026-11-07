//! Encoder and decoder wrappers, plus the `decode` and `limits` subcommands.

use crate::{Args, Fnv, mib, ms, row, timed};
use libwebp_sys as w;
use std::fs::File;
use std::io::BufReader;

pub fn png_encode(rgb: &[u8], wd: u32, ht: u32, c: png::Compression) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, wd, ht);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.set_compression(c);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(rgb).unwrap();
    }
    out
}

pub fn jpeg_encode(rgb: &[u8], wd: u32, ht: u32, q: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let e = jpeg_encoder::Encoder::new(&mut out, q);
    e.encode(rgb, wd as u16, ht as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    out
}

/// Lossy WebP via libwebp's advanced API (method 4). Reports libwebp's error
/// code. `partition_limit` (0-100) trades quality for fitting VP8's 512 KiB
/// first partition on very large images. `threads` sets thread_level.
pub fn webp_lossy_adv(px: &[u8], wd: u32, ht: u32, ch: u32, q: f32, partition_limit: i32, threads: bool) -> Result<Vec<u8>, String> {
    unsafe {
        let mut cfg: w::WebPConfig = std::mem::zeroed();
        w::WebPConfigInitInternal(&mut cfg, w::WebPPreset::WEBP_PRESET_DEFAULT, q, w::WEBP_ENCODER_ABI_VERSION as i32);
        cfg.method = 4;
        cfg.partition_limit = partition_limit;
        cfg.thread_level = threads as i32;
        if w::WebPValidateConfig(&cfg) == 0 {
            return Err("invalid config".into());
        }
        let mut pic: w::WebPPicture = std::mem::zeroed();
        w::WebPPictureInitInternal(&mut pic, w::WEBP_ENCODER_ABI_VERSION as i32);
        pic.width = wd as i32;
        pic.height = ht as i32;
        let ok = if ch == 4 {
            w::WebPPictureImportRGBA(&mut pic, px.as_ptr(), (wd * 4) as i32)
        } else {
            w::WebPPictureImportRGB(&mut pic, px.as_ptr(), (wd * 3) as i32)
        };
        if ok == 0 {
            let e = pic.error_code;
            w::WebPPictureFree(&mut pic);
            return Err(format!("libwebp import {wd}x{ht}: {e:?}"));
        }
        let mut wr: w::WebPMemoryWriter = std::mem::zeroed();
        w::WebPMemoryWriterInit(&mut wr);
        pic.writer = Some(w::WebPMemoryWrite);
        pic.custom_ptr = &mut wr as *mut _ as *mut _;
        let ok = w::WebPEncode(&cfg, &mut pic);
        let e = pic.error_code;
        w::WebPPictureFree(&mut pic);
        let res = if ok == 0 {
            Err(format!("libwebp encode {wd}x{ht}: {e:?}"))
        } else {
            Ok(std::slice::from_raw_parts(wr.mem, wr.size).to_vec())
        };
        w::WebPMemoryWriterClear(&mut wr);
        res
    }
}

/// Lossy WebP; on partition-0 overflow (large images) retry with partition_limit 100.
pub fn webp_lossy(px: &[u8], wd: u32, ht: u32, ch: u32, q: f32) -> Result<Vec<u8>, String> {
    match webp_lossy_adv(px, wd, ht, ch, q, 0, false) {
        Err(e) if e.contains("PARTITION0") => {
            eprintln!("note: {e}; retrying with partition_limit=100");
            webp_lossy_adv(px, wd, ht, ch, q, 100, false).map_err(|e2| format!("{e}; retry: {e2}"))
        }
        r => r,
    }
}

/// Lossy WebP via libwebp's simple API (method 4, single thread).
#[allow(dead_code)]
pub fn webp_lossy_simple(px: &[u8], wd: u32, ht: u32, ch: u32, q: f32) -> Result<Vec<u8>, String> {
    unsafe {
        let mut p: *mut u8 = std::ptr::null_mut();
        let n = if ch == 4 {
            w::WebPEncodeRGBA(px.as_ptr(), wd as i32, ht as i32, (wd * 4) as i32, q, &mut p)
        } else {
            w::WebPEncodeRGB(px.as_ptr(), wd as i32, ht as i32, (wd * 3) as i32, q, &mut p)
        };
        if n == 0 {
            return Err(format!("libwebp: encode failed for {wd}x{ht}"));
        }
        let v = std::slice::from_raw_parts(p, n).to_vec();
        w::WebPFree(p as *mut _);
        Ok(v)
    }
}

pub fn webp_lossless_libwebp(rgb: &[u8], wd: u32, ht: u32) -> Result<Vec<u8>, String> {
    unsafe {
        let mut p: *mut u8 = std::ptr::null_mut();
        let n = w::WebPEncodeLosslessRGB(rgb.as_ptr(), wd as i32, ht as i32, (wd * 3) as i32, &mut p);
        if n == 0 {
            return Err(format!("libwebp: lossless encode failed for {wd}x{ht}"));
        }
        let v = std::slice::from_raw_parts(p, n).to_vec();
        w::WebPFree(p as *mut _);
        Ok(v)
    }
}

/// Lossless WebP, pure Rust (image-webp). Its encoder is lossless-only.
pub fn webp_lossless_pure(px: &[u8], wd: u32, ht: u32, ch: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let ct = if ch == 4 { image_webp::ColorType::Rgba8 } else { image_webp::ColorType::Rgb8 };
    image_webp::WebPEncoder::new(&mut out)
        .encode(px, wd, ht, ct)
        .map_err(|e| format!("image-webp: {e}"))?;
    Ok(out)
}

pub fn webp_decode_libwebp(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    unsafe {
        let (mut wd, mut ht) = (0i32, 0i32);
        let p = w::WebPDecodeRGB(data.as_ptr(), data.len(), &mut wd, &mut ht);
        if p.is_null() {
            return Err("libwebp: decode failed".into());
        }
        let n = (wd * ht * 3) as usize;
        let v = std::slice::from_raw_parts(p, n).to_vec();
        w::WebPFree(p as *mut _);
        Ok((wd as u32, ht as u32, v))
    }
}

/// Decode an image file to RGB8 with the `image` crate and no allocation limit.
pub fn decode_rgb(path: &str) -> (u32, u32, Vec<u8>) {
    let mut r = image::ImageReader::open(path).unwrap().with_guessed_format().unwrap();
    r.no_limits();
    let img = r.decode().unwrap().into_rgb8();
    let (wd, ht) = img.dimensions();
    (wd, ht, img.into_raw())
}

pub fn png_row_reader(path: &str) -> png::Reader<BufReader<File>> {
    let f = BufReader::with_capacity(1 << 20, File::open(path).unwrap());
    let mut d = png::Decoder::new_with_limits(f, png::Limits { bytes: usize::MAX });
    d.set_transformations(png::Transformations::EXPAND);
    d.read_info().unwrap()
}

/// `decode --file F --with image|image-default|png-rows|libwebp`
pub fn cmd_decode(a: &Args) {
    let file = a.str("file", "");
    let with = a.str("with", "image");
    let name = std::path::Path::new(&file).file_name().unwrap().to_string_lossy().to_string();
    let case = format!("decode-{with}");
    match with.as_str() {
        "image-default" | "image" | "image-rgba" => {
            let (res, t) = timed(|| {
                let mut r = image::ImageReader::open(&file).unwrap().with_guessed_format().unwrap();
                if with != "image-default" {
                    r.no_limits();
                }
                r.decode()
            });
            match res {
                Ok(img) => {
                    let ct = format!("{:?}", img.color());
                    let (wd, ht) = (img.width(), img.height());
                    let mut extra = 0.0;
                    let h = if with == "image-rgba" {
                        let (rgba, t2) = timed(|| img.into_rgba8());
                        extra = t2;
                        crate::fnv(&rgba.as_raw()[..1 << 20])
                    } else {
                        crate::fnv(&img.as_bytes()[..1 << 20])
                    };
                    row(&case, &[
                        ("file", name),
                        ("w", wd.to_string()),
                        ("h", ht.to_string()),
                        ("color", ct),
                        ("ms", ms(t)),
                        ("to_rgba_ms", ms(extra)),
                        ("hash1m", format!("{h:016x}")),
                    ]);
                }
                Err(e) => row(&case, &[("file", name), ("ms", ms(t)), ("error", e.to_string())]),
            }
        }
        "png-rows" => {
            let (res, t) = timed(|| {
                let mut r = png_row_reader(&file);
                let (wd, ht) = (r.info().width, r.info().height);
                let mut f = Fnv::new();
                let mut first = true;
                let mut n = 0u64;
                while let Some(rw) = r.next_row().unwrap() {
                    if first {
                        f.update(rw.data());
                        first = false;
                    }
                    n += rw.data().len() as u64;
                }
                (wd, ht, n, f.0)
            });
            row(&case, &[
                ("file", name),
                ("w", res.0.to_string()),
                ("h", res.1.to_string()),
                ("ms", ms(t)),
                ("bytes_out_mib", mib(res.2)),
            ]);
        }
        "libwebp" => {
            let data = std::fs::read(&file).unwrap();
            let (res, t) = timed(|| webp_decode_libwebp(&data));
            match res {
                Ok((wd, ht, px)) => row(&case, &[
                    ("file", name),
                    ("w", wd.to_string()),
                    ("h", ht.to_string()),
                    ("ms", ms(t)),
                    ("hash1m", format!("{:016x}", crate::fnv(&px[..1 << 20]))),
                ]),
                Err(e) => row(&case, &[("file", name), ("error", e)]),
            }
        }
        _ => panic!("unknown --with"),
    }
}

/// `limits`: probe dimension limits of each encoder with thin images.
pub fn cmd_limits(_a: &Args) {
    let probe = |label: &str, wd: u32, f: &dyn Fn(&[u8], u32) -> Result<usize, String>| {
        let px = vec![128u8; wd as usize * 8 * 3];
        let r = f(&px, wd);
        row("limit", &[
            ("encoder", label.to_string()),
            ("width", wd.to_string()),
            ("result", match r {
                Ok(n) => format!("ok({n}B)"),
                Err(e) => format!("ERR: {e}"),
            }),
        ]);
    };
    for wd in [16383u32, 16384] {
        probe("libwebp-lossy", wd, &|p, wd| webp_lossy(p, wd, 8, 3, 80.0).map(|v| v.len()));
        probe("libwebp-lossless", wd, &|p, wd| webp_lossless_libwebp(p, wd, 8).map(|v| v.len()));
        probe("image-webp-lossless", wd, &|p, wd| webp_lossless_pure(p, wd, 8, 3).map(|v| v.len()));
        // Can libwebp (what browsers and most VTT hosts use) decode it?
        probe("image-webp-lossless->libwebp-decode", wd, &|p, wd| {
            let v = webp_lossless_pure(p, wd, 8, 3)?;
            webp_decode_libwebp(&v).map(|(w2, h2, _)| (w2 * h2) as usize)
        });
        probe("image-webp-lossless->image-webp-decode", wd, &|p, wd| {
            let v = webp_lossless_pure(p, wd, 8, 3)?;
            image::load_from_memory(&v).map(|i| (i.width() * i.height()) as usize).map_err(|e| e.to_string())
        });
    }
    probe("jpeg-encoder", 65535, &|p, wd| Ok(jpeg_encode(p, wd, 8, 80).len()));
    probe("png", 100_000, &|p, wd| Ok(png_encode(p, wd, 8, png::Compression::Fast).len()));
    crate::anim::probe_limits();
}

/// `compare --a A.png --b B.png`: stream two RGB PNGs and report pixel differences.
pub fn cmd_compare(a: &Args) {
    let (pa, pb) = (a.str("a", ""), a.str("b", ""));
    let mut ra = png_row_reader(&pa);
    let mut rb = png_row_reader(&pb);
    let (mut diff_px, mut max_d, mut total, mut sum_d) = (0u64, 0u8, 0u64, 0u64);
    let mut hist = [0u64; 4]; // max channel diff 1, 2, 3, >3
    let (mut y0, mut y1) = (u32::MAX, 0u32);
    let mut y = 0u32;
    while let (Some(x), Some(z)) = (ra.next_row().unwrap().map(|r| r.data().to_vec()), rb.next_row().unwrap().map(|r| r.data().to_vec())) {
        for (p, q) in x.chunks_exact(3).zip(z.chunks_exact(3)) {
            total += 1;
            let d = p.iter().zip(q).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            if d > 0 {
                diff_px += 1;
                sum_d += d as u64;
                max_d = max_d.max(d);
                hist[(d.min(4) - 1) as usize] += 1;
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
        y += 1;
    }
    row("compare", &[
        ("pixels", total.to_string()),
        ("differing", diff_px.to_string()),
        ("differing_pct", format!("{:.4}", diff_px as f64 * 100.0 / total as f64)),
        ("max_diff", max_d.to_string()),
        ("mean_diff_of_differing", format!("{:.2}", sum_d as f64 / diff_px.max(1) as f64)),
        ("hist_1_2_3_gt3", format!("{:?}", hist)),
        ("rows", format!("{y0}..{y1}")),
    ]);
}
