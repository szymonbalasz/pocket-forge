//! Animated WebP and GIF encoding. Frames are synthetic (in the app they come
//! from a provider video model or in-app compositing); only encoding is measured.

use crate::{Args, mib, ms, row, synth, timed};
use image::AnimationDecoder;
use libwebp_sys as w;
use rayon::prelude::*;

pub struct Clip {
    pub w: usize,
    pub h: usize,
    pub frames: Vec<Vec<u8>>, // RGBA8, straight alpha
    pub delay_ms: u32,
}

/// A round token with a rotating glow on its rim and a pulsing face. Transparent outside.
pub fn token(n: usize, frames: usize) -> Clip {
    let face = synth::fill(9, 3000, 3000, n, n);
    let c = n as f32 / 2.0;
    let r = c - 32.0;
    let fr = (0..frames)
        .into_par_iter()
        .map(|i| {
            let ph = i as f32 / frames as f32 * std::f32::consts::TAU;
            let mut px = vec![0u8; n * n * 4];
            for y in 0..n {
                for x in 0..n {
                    let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
                    let d = (dx * dx + dy * dy).sqrt();
                    let a = (r - d + 0.5).clamp(0.0, 1.0);
                    if a == 0.0 {
                        continue;
                    }
                    let o = (y * n + x) * 4;
                    let rgb = if d > r - 36.0 {
                        let ang = dy.atan2(dx);
                        let glow = ((ang - ph).cos() * 0.5 + 0.5).powf(6.0);
                        [180.0 + 75.0 * glow, 140.0 + 100.0 * glow, 40.0 + 160.0 * glow]
                    } else {
                        let k = 0.85 + 0.15 * (ph + d / 60.0).sin();
                        let f = &face[(y * n + x) * 3..];
                        [f[0] as f32 * k, f[1] as f32 * k, f[2] as f32 * k]
                    };
                    for ch in 0..3 {
                        px[o + ch] = rgb[ch].clamp(0.0, 255.0) as u8;
                    }
                    px[o + 3] = (a * 255.0).round() as u8;
                }
            }
            px
        })
        .collect();
    Clip { w: n, h: n, frames: fr, delay_ms: 40 }
}

/// A map region with three flickering torches: most pixels are static.
pub fn map_region(wd: usize, ht: usize, frames: usize) -> Clip {
    let base = synth::fill(1, 4000, 4000, wd, ht);
    let torches = [(0.25f32, 0.3f32), (0.7, 0.4), (0.45, 0.75)];
    let fr = (0..frames)
        .into_par_iter()
        .map(|i| {
            let mut px = vec![255u8; wd * ht * 4];
            for y in 0..ht {
                for x in 0..wd {
                    let mut k = 1.0f32;
                    for (t, &(tx, ty)) in torches.iter().enumerate() {
                        let (dx, dy) = (x as f32 - tx * wd as f32, y as f32 - ty * ht as f32);
                        let d = (dx * dx + dy * dy).sqrt() / 320.0;
                        if d < 1.0 {
                            let fl = synth::unit(synth::hash(i as i64, t as i64, 5));
                            k += (0.15 + 0.25 * fl) * (1.0 - d) * (1.0 - d);
                        }
                    }
                    let s = &base[(y * wd + x) * 3..];
                    let o = (y * wd + x) * 4;
                    px[o] = (s[0] as f32 * k * 1.05).min(255.0) as u8;
                    px[o + 1] = (s[1] as f32 * k).min(255.0) as u8;
                    px[o + 2] = (s[2] as f32 * k * 0.9).min(255.0) as u8;
                }
            }
            px
        })
        .collect();
    Clip { w: wd, h: ht, frames: fr, delay_ms: 83 }
}

/// Animated WebP via libwebp's WebPAnimEncoder (sub-frame and keyframe
/// decisions are libwebp's). `lossless`, `mixed` = per-frame lossy/lossless choice.
pub fn webp_anim_libwebp(c: &Clip, quality: f32, lossless: bool, mixed: bool) -> Result<Vec<u8>, String> {
    unsafe {
        let mut opts: w::WebPAnimEncoderOptions = std::mem::zeroed();
        if w::WebPAnimEncoderOptionsInitInternal(&mut opts, w::WEBP_MUX_ABI_VERSION as i32) == 0 {
            return Err("anim options init".into());
        }
        opts.anim_params.loop_count = 0;
        opts.allow_mixed = mixed as i32;
        let enc = w::WebPAnimEncoderNewInternal(c.w as i32, c.h as i32, &opts, w::WEBP_MUX_ABI_VERSION as i32);
        if enc.is_null() {
            return Err(format!("WebPAnimEncoderNew failed for {}x{}", c.w, c.h));
        }
        let mut cfg: w::WebPConfig = std::mem::zeroed();
        w::WebPConfigInitInternal(&mut cfg, w::WebPPreset::WEBP_PRESET_DEFAULT, quality, w::WEBP_ENCODER_ABI_VERSION as i32);
        cfg.lossless = lossless as i32;
        cfg.method = 4;
        let mut ts = 0i32;
        let mut err = None;
        for f in &c.frames {
            let mut pic: w::WebPPicture = std::mem::zeroed();
            w::WebPPictureInitInternal(&mut pic, w::WEBP_ENCODER_ABI_VERSION as i32);
            pic.width = c.w as i32;
            pic.height = c.h as i32;
            pic.use_argb = 1;
            w::WebPPictureImportRGBA(&mut pic, f.as_ptr(), (c.w * 4) as i32);
            let ok = w::WebPAnimEncoderAdd(enc, &mut pic, ts, &cfg);
            w::WebPPictureFree(&mut pic);
            if ok == 0 {
                let m = std::ffi::CStr::from_ptr(w::WebPAnimEncoderGetError(enc)).to_string_lossy().to_string();
                err = Some(format!("WebPAnimEncoderAdd: {m}"));
                break;
            }
            ts += c.delay_ms as i32;
        }
        let res = if let Some(e) = err {
            Err(e)
        } else {
            w::WebPAnimEncoderAdd(enc, std::ptr::null_mut(), ts, std::ptr::null());
            let mut data = w::WebPData::default();
            if w::WebPAnimEncoderAssemble(enc, &mut data) == 0 {
                Err("assemble failed".into())
            } else {
                let v = std::slice::from_raw_parts(data.bytes, data.size).to_vec();
                w::WebPDataClear(&mut data);
                Ok(v)
            }
        };
        w::WebPAnimEncoderDelete(enc);
        res
    }
}

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}
fn u24(v: usize) -> [u8; 3] {
    [(v & 0xff) as u8, ((v >> 8) & 0xff) as u8, ((v >> 16) & 0xff) as u8]
}

/// Find the VP8L chunk (header + payload) in a still WebP from image-webp.
fn vp8l_chunk(webp: &[u8]) -> Vec<u8> {
    let mut i = 12;
    while i + 8 <= webp.len() {
        let id = &webp[i..i + 4];
        let len = u32::from_le_bytes(webp[i + 4..i + 8].try_into().unwrap()) as usize;
        if id == b"VP8L" {
            return webp[i..i + 8 + len + (len & 1)].to_vec();
        }
        i += 8 + len + (len & 1);
    }
    panic!("no VP8L chunk");
}

/// Pure-Rust animated WebP: lossless frames from image-webp, muxed by hand
/// (RIFF/VP8X/ANIM/ANMF). Each frame is cropped to the even-aligned bounding
/// box of pixels that changed since the previous frame. No blending (each ANMF overwrites).
pub fn webp_anim_pure(c: &Clip) -> Result<Vec<u8>, String> {
    let rects: Vec<(usize, usize, usize, usize)> = (0..c.frames.len())
        .map(|i| {
            if i == 0 {
                return (0, 0, c.w, c.h);
            }
            let (a, b) = (&c.frames[i - 1], &c.frames[i]);
            let (mut x0, mut y0, mut x1, mut y1) = (c.w, c.h, 0, 0);
            for y in 0..c.h {
                let ra = &a[y * c.w * 4..(y + 1) * c.w * 4];
                let rb = &b[y * c.w * 4..(y + 1) * c.w * 4];
                if ra == rb {
                    continue;
                }
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
                for x in 0..c.w {
                    if ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4] {
                        x0 = x0.min(x);
                        x1 = x1.max(x + 1);
                    }
                }
            }
            if x1 == 0 {
                return (0, 0, 2.min(c.w), 2.min(c.h)); // unchanged: tiny frame
            }
            let (x0, y0) = (x0 & !1, y0 & !1); // ANMF offsets are stored /2
            (x0, y0, x1 - x0, y1 - y0)
        })
        .collect();
    let encoded: Vec<Result<Vec<u8>, String>> = rects
        .par_iter()
        .enumerate()
        .map(|(i, &(x0, y0, rw, rh))| {
            let mut sub = Vec::with_capacity(rw * rh * 4);
            for y in y0..y0 + rh {
                sub.extend_from_slice(&c.frames[i][(y * c.w + x0) * 4..(y * c.w + x0 + rw) * 4]);
            }
            crate::codecs::webp_lossless_pure(&sub, rw as u32, rh as u32, 4).map(|v| vp8l_chunk(&v))
        })
        .collect();
    let mut body = Vec::new();
    body.extend_from_slice(b"WEBP");
    let mut vp8x = vec![0x02 | 0x10, 0, 0, 0]; // animation + alpha
    vp8x.extend_from_slice(&u24(c.w - 1));
    vp8x.extend_from_slice(&u24(c.h - 1));
    chunk(&mut body, b"VP8X", &vp8x);
    chunk(&mut body, b"ANIM", &[0, 0, 0, 0, 0, 0]); // bg colour, loop forever
    for (r, e) in rects.iter().zip(encoded) {
        let e = e?;
        let mut d = Vec::new();
        d.extend_from_slice(&u24(r.0 / 2));
        d.extend_from_slice(&u24(r.1 / 2));
        d.extend_from_slice(&u24(r.2 - 1));
        d.extend_from_slice(&u24(r.3 - 1));
        d.extend_from_slice(&u24(c.delay_ms as usize));
        d.push(0x02); // do not blend, do not dispose
        d.extend_from_slice(&e);
        chunk(&mut body, b"ANMF", &d);
    }
    let mut out = Vec::new();
    chunk(&mut out, b"RIFF", &body);
    Ok(out)
}

/// GIF, per-frame NeuQuant palette (gif crate's `from_rgba_speed`), frames quantised in parallel.
pub fn gif_local(c: &Clip, speed: i32) -> Vec<u8> {
    let frames: Vec<gif::Frame<'static>> = c
        .frames
        .par_iter()
        .map(|f| {
            let mut px = f.clone();
            let mut fr = gif::Frame::from_rgba_speed(c.w as u16, c.h as u16, &mut px, speed);
            fr.delay = (c.delay_ms / 10) as u16;
            fr.dispose = gif::DisposalMethod::Background;
            fr
        })
        .collect();
    let mut out = Vec::new();
    {
        let mut e = gif::Encoder::new(&mut out, c.w as u16, c.h as u16, &[]).unwrap();
        e.set_repeat(gif::Repeat::Infinite).unwrap();
        for f in &frames {
            e.write_frame(f).unwrap();
        }
    }
    out
}

/// GIF, one global NeuQuant palette (255 colours + 1 transparent) built from
/// samples of all frames: no palette flicker between frames.
pub fn gif_global(c: &Clip) -> Vec<u8> {
    let mut sample = Vec::new();
    for f in &c.frames {
        for p in f.chunks_exact(4).step_by(7) {
            if p[3] >= 128 {
                sample.extend_from_slice(p);
            }
        }
    }
    let nq = color_quant::NeuQuant::new(10, 255, &sample);
    let mut pal = nq.color_map_rgb();
    pal.extend_from_slice(&[0, 0, 0]);
    let idx: Vec<Vec<u8>> = c
        .frames
        .par_iter()
        .map(|f| f.chunks_exact(4).map(|p| if p[3] < 128 { 255 } else { nq.index_of(p) as u8 }).collect())
        .collect();
    let mut out = Vec::new();
    {
        let mut e = gif::Encoder::new(&mut out, c.w as u16, c.h as u16, &pal).unwrap();
        e.set_repeat(gif::Repeat::Infinite).unwrap();
        for b in &idx {
            let mut fr = gif::Frame::default();
            fr.width = c.w as u16;
            fr.height = c.h as u16;
            fr.buffer = std::borrow::Cow::Borrowed(b);
            fr.delay = (c.delay_ms / 10) as u16;
            fr.transparent = Some(255);
            fr.dispose = gif::DisposalMethod::Background;
            e.write_frame(&fr).unwrap();
        }
    }
    out
}

fn count_frames(kind: &str, data: &[u8]) -> String {
    let cur = std::io::Cursor::new(data);
    let r = if kind == "gif" {
        image::codecs::gif::GifDecoder::new(cur).map(|d| d.into_frames().count())
    } else {
        image::codecs::webp::WebPDecoder::new(cur).map(|d| d.into_frames().count())
    };
    match r {
        Ok(n) => n.to_string(),
        Err(e) => format!("decode-error:{e}"),
    }
}

/// `anim --clip token|map --out DIR`
pub fn cmd_anim(a: &Args) {
    let clip = a.str("clip", "token");
    let dir = a.str("out", ".");
    std::fs::create_dir_all(&dir).unwrap();
    let (c, t_gen) = timed(|| if clip == "token" { token(1024, 48) } else { map_region(2048, 1536, 24) });
    row("anim-frames", &[("clip", clip.clone()), ("w", c.w.to_string()), ("h", c.h.to_string()), ("frames", c.frames.len().to_string()), ("gen_ms", ms(t_gen))]);
    let raw = (c.w * c.h * 4 * c.frames.len()) as u64;
    type Enc<'a> = (&'a str, &'a str, Box<dyn Fn(&Clip) -> Result<Vec<u8>, String>>);
    let encs: Vec<Enc> = vec![
        ("webp-libwebp-lossy-q75", "webp", Box::new(|c| webp_anim_libwebp(c, 75.0, false, false))),
        ("webp-libwebp-lossy-q90", "webp", Box::new(|c| webp_anim_libwebp(c, 90.0, false, false))),
        ("webp-libwebp-mixed-q75", "webp", Box::new(|c| webp_anim_libwebp(c, 75.0, false, true))),
        ("webp-libwebp-lossless", "webp", Box::new(|c| webp_anim_libwebp(c, 75.0, true, false))),
        ("webp-pure-rust-lossless", "webp", Box::new(webp_anim_pure)),
        ("gif-local-neuquant-s10", "gif", Box::new(|c| Ok(gif_local(c, 10)))),
        ("gif-global-neuquant", "gif", Box::new(|c| Ok(gif_global(c)))),
    ];
    for (name, kind, f) in encs {
        let (r, t) = timed(|| f(&c));
        match r {
            Ok(b) => {
                let p = std::path::Path::new(&dir).join(format!("{clip}-{name}.{kind}"));
                std::fs::write(&p, &b).unwrap();
                row("anim", &[
                    ("clip", clip.clone()),
                    ("encoder", name.into()),
                    ("ms", ms(t)),
                    ("ms_per_frame", format!("{:.0}", t / c.frames.len() as f64)),
                    ("mib", mib(b.len() as u64)),
                    ("ratio_vs_raw", format!("{:.3}", b.len() as f64 / raw as f64)),
                    ("frames_decoded", count_frames(kind, &b)),
                ]);
            }
            Err(e) => row("anim", &[("clip", clip.clone()), ("encoder", name.into()), ("error", e)]),
        }
    }
}

/// Dimension probes for the animation encoders (called from `limits`).
pub fn probe_limits() {
    for wd in [16383usize, 16384] {
        let c = Clip { w: wd, h: 2, frames: vec![vec![128u8; wd * 2 * 4]; 2], delay_ms: 100 };
        let r = webp_anim_libwebp(&c, 75.0, false, false);
        row("limit", &[("encoder", "libwebp-anim".into()), ("width", wd.to_string()), ("result", match r {
            Ok(v) => format!("ok({}B)", v.len()),
            Err(e) => format!("ERR: {e}"),
        })]);
    }
    // GIF: dimensions are u16 in the format and in the gif crate API.
    let c = Clip { w: 65535, h: 2, frames: vec![vec![128u8; 65535 * 2 * 4]], delay_ms: 100 };
    row("limit", &[("encoder", "gif".into()), ("width", "65535".into()), ("result", format!("ok({}B)", gif_local(&c, 30).len()))]);
}

/// `animcheck --file F.webp`: decode an animated WebP with libwebp's
/// WebPAnimDecoder (the decoder browsers use) and count frames.
pub fn cmd_animcheck(a: &Args) {
    let file = a.str("file", "");
    let data = std::fs::read(&file).unwrap();
    let n = unsafe {
        let wd = w::WebPData { bytes: data.as_ptr(), size: data.len() };
        let dec = w::WebPAnimDecoderNew(&wd, std::ptr::null());
        if dec.is_null() {
            -1
        } else {
            let mut n = 0;
            while w::WebPAnimDecoderHasMoreFrames(dec) != 0 {
                let mut buf: *mut u8 = std::ptr::null_mut();
                let mut ts = 0i32;
                if w::WebPAnimDecoderGetNext(dec, &mut buf, &mut ts) == 0 {
                    n = -2;
                    break;
                }
                n += 1;
            }
            w::WebPAnimDecoderDelete(dec);
            n
        }
    };
    let name = std::path::Path::new(&file).file_name().unwrap().to_string_lossy().to_string();
    row("animcheck-libwebp", &[("file", name), ("frames", n.to_string())]);
}
