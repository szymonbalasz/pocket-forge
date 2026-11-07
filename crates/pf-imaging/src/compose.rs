//! The compositor (export spec, slice 1): flattens a layer stack into RGB8 (FR-EXP-01..05).
//!
//! It is the only compositor in the app (KQ-25). Export, region extraction (FR-RGN-04) and the
//! golden tests all call [`compose_rect`] or [`flatten_strips`]. Drawing uses tiny-skia,
//! premultiplied sRGB source-over (ADR 0004), in horizontal strips so memory stays bounded.
//!
//! - The original is opaque RGB8 and is copied into each strip unchanged (it sits on opaque black,
//!   which it fully covers).
//! - Visible layers are drawn in order. A layer with scale 1, rotation 0 and an integer top-left
//!   is drawn with exact pixel sampling; any other layer is drawn with bicubic sampling through
//!   its affine about the centre (ADR 0003).

use std::sync::Arc;

use image::{RgbImage, RgbaImage};
use tiny_skia::{
    BlendMode, ColorU8, FilterQuality, IntSize, Pixmap, PixmapPaint, PremultipliedColorU8,
    Transform,
};

/// Renderer version. Bump it on any change that can alter output pixels, and regenerate the
/// goldens (`PF_UPDATE_GOLDENS=1`). Each export records it (FR-EXP-05).
pub const COMPOSE_VERSION: u32 = 1;

/// Pinned strip height in rows (FR-EXP-05, ADR 0004). Each export records it.
pub const STRIP_H: u32 = 512;

/// Tolerance for treating a layer's top-left as an integer pixel position (FR-EXP-03).
const INTEGER_EPS: f64 = 1e-6;

/// Transparent border added around a resampled layer, so its edge is filtered instead of
/// aliased (tiny-skia fills the transformed rect without anti-aliasing and pads beyond it).
const AFFINE_PAD: u32 = 2;

/// Where a layer sits, in image px (ADR 0003 layer model): its centre, a uniform scale and a
/// rotation in degrees about the centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub cx: f64,
    pub cy: f64,
    pub scale: f64,
    pub rotation: f64,
}

impl Placement {
    /// A placement with scale 1 and rotation 0 whose top-left is at `(x, y)`.
    pub fn at(x: i64, y: i64, width: u32, height: u32) -> Self {
        Placement {
            cx: x as f64 + f64::from(width) / 2.0,
            cy: y as f64 + f64::from(height) / 2.0,
            scale: 1.0,
            rotation: 0.0,
        }
    }

    /// The integer top-left when the layer can be drawn with exact pixel sampling (FR-EXP-03):
    /// scale 1, rotation 0 (mod 360) and a top-left within 1e-6 px of an integer.
    pub fn integer_origin(&self, width: u32, height: u32) -> Option<(i64, i64)> {
        if self.scale != 1.0 || self.rotation.rem_euclid(360.0) != 0.0 {
            return None;
        }
        let x = self.cx - f64::from(width) / 2.0;
        let y = self.cy - f64::from(height) / 2.0;
        let (rx, ry) = (x.round(), y.round());
        if (x - rx).abs() <= INTEGER_EPS && (y - ry).abs() <= INTEGER_EPS {
            Some((rx as i64, ry as i64))
        } else {
            None
        }
    }

    /// Layer px → image px, as a row-major 2×3 affine `[a, b, c, d, e, f]` with
    /// `X = a·x + c·y + e`, `Y = b·x + d·y + f`, computed in f64.
    fn affine(&self, width: u32, height: u32) -> [f64; 6] {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let (a, b) = (self.scale * cos, self.scale * sin);
        let (c, d) = (-self.scale * sin, self.scale * cos);
        let (hw, hh) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
        let e = self.cx - (a * hw + c * hh);
        let f = self.cy - (b * hw + d * hh);
        [a, b, c, d, e, f]
    }
}

/// One patch layer: an RGBA8 straight-alpha image and how it is drawn.
#[derive(Debug, Clone)]
pub struct Layer {
    pub image: Arc<RgbaImage>,
    pub placement: Placement,
    /// 0–1; values outside are clamped.
    pub opacity: f32,
    pub visible: bool,
}

/// An image-space rectangle in whole pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }
}

/// Why a compose call failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ComposeError {
    /// A buffer couldn't be reserved (FR-EXP-23).
    #[error("out of memory")]
    OutOfMemory,
    #[error("rect {x},{y} {width}x{height} is empty or outside the {image_w}x{image_h} image")]
    RectOutOfBounds {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        image_w: u32,
        image_h: u32,
    },
    #[error("strip height must be at least 1")]
    InvalidStripHeight,
}

/// A layer prepared for drawing: premultiplied pixels and how to place them.
#[derive(Debug)]
struct Prepared {
    pixmap: Pixmap,
    draw: Draw,
    opacity: f32,
    /// Bounding box in image px, `[x0, y0, x1, y1)`, for skipping strips the layer misses.
    bounds: [f64; 4],
}

#[derive(Debug, Clone, Copy)]
enum Draw {
    /// Exact pixel sampling at an integer top-left.
    Integer { x: i64, y: i64 },
    /// Bicubic through the affine (layer px of the padded pixmap → image px).
    Affine([f64; 6]),
}

/// A layer stack ready to compose: the original plus its visible layers, in draw order
/// (FR-EXP-02). Building it premultiplies each layer once, so strips don't repeat that work.
#[derive(Debug)]
pub struct LayerStack {
    base: Arc<RgbImage>,
    layers: Vec<Prepared>,
}

impl LayerStack {
    /// Prepares `layers` over `base`. Hidden, fully transparent (opacity 0) and empty layers are
    /// dropped here.
    pub fn new(base: Arc<RgbImage>, layers: &[Layer]) -> Result<Self, ComposeError> {
        let mut prepared = Vec::new();
        prepared
            .try_reserve_exact(layers.len())
            .map_err(|_| ComposeError::OutOfMemory)?;
        for layer in layers {
            let opacity = layer.opacity.clamp(0.0, 1.0);
            let (w, h) = layer.image.dimensions();
            if !layer.visible || opacity <= 0.0 || w == 0 || h == 0 {
                continue;
            }
            let p = match layer.placement.integer_origin(w, h) {
                Some((x, y)) => Prepared {
                    pixmap: premultiply(&layer.image, 0)?,
                    draw: Draw::Integer { x, y },
                    opacity,
                    bounds: [
                        x as f64,
                        y as f64,
                        (x + i64::from(w)) as f64,
                        (y + i64::from(h)) as f64,
                    ],
                },
                None => {
                    let pad = f64::from(AFFINE_PAD);
                    let m = layer.placement.affine(w, h);
                    // Shift by the pad so padded pixmap px (pad, pad) maps to layer px (0, 0).
                    let m = [
                        m[0],
                        m[1],
                        m[2],
                        m[3],
                        m[4] - m[0] * pad - m[2] * pad,
                        m[5] - m[1] * pad - m[3] * pad,
                    ];
                    let (pw, ph) = (f64::from(w + 2 * AFFINE_PAD), f64::from(h + 2 * AFFINE_PAD));
                    Prepared {
                        pixmap: premultiply(&layer.image, AFFINE_PAD)?,
                        draw: Draw::Affine(m),
                        opacity,
                        bounds: affine_bounds(&m, pw, ph),
                    }
                }
            };
            prepared.push(p);
        }
        Ok(LayerStack {
            base,
            layers: prepared,
        })
    }

    /// The original's size, which is the flatten's size.
    pub fn dimensions(&self) -> (u32, u32) {
        self.base.dimensions()
    }

    /// The number of layers that will be drawn (visible, non-empty).
    pub fn drawn_layers(&self) -> usize {
        self.layers.len()
    }

    fn check_rect(&self, r: PixelRect) -> Result<(), ComposeError> {
        let (image_w, image_h) = self.dimensions();
        let fits = r.width > 0
            && r.height > 0
            && u64::from(r.x) + u64::from(r.width) <= u64::from(image_w)
            && u64::from(r.y) + u64::from(r.height) <= u64::from(image_h);
        if fits {
            Ok(())
        } else {
            Err(ComposeError::RectOutOfBounds {
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
                image_w,
                image_h,
            })
        }
    }
}

/// One strip of a flatten, as tightly packed RGB8 rows.
#[derive(Debug)]
pub struct Strip<'a> {
    /// First image row of this strip.
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// `width × height × 3` bytes.
    pub rgb: &'a [u8],
}

/// Returns the RGB8 flatten of `stack` over `rect` at source resolution (FR-EXP-01). Internally
/// it works in bands of [`STRIP_H`] rows, so a large rect never needs a full RGBA buffer.
pub fn compose_rect(stack: &LayerStack, rect: PixelRect) -> Result<RgbImage, ComposeError> {
    stack.check_rect(rect)?;
    let mut out = try_vec(rect.width as usize * rect.height as usize * 3)?;
    let mut scratch = Scratch::default();
    let mut y = rect.y;
    let end = rect.y + rect.height;
    while y < end {
        let h = STRIP_H.min(end - y);
        let band = PixelRect::new(rect.x, y, rect.width, h);
        let offset = (y - rect.y) as usize * rect.width as usize * 3;
        let len = rect.width as usize * h as usize * 3;
        compose_band(stack, band, &mut scratch, &mut out[offset..offset + len])?;
        y += h;
    }
    RgbImage::from_raw(rect.width, rect.height, out).ok_or(ComposeError::OutOfMemory)
}

/// Flattens the whole stack top to bottom in strips of `strip_h` rows and hands each strip to
/// `sink` (FR-EXP-05, FR-EXP-09). Stops at the first error, including one the sink returns (this
/// is where callers check for cancel between strips, FR-EXP-20).
pub fn flatten_strips<E, F>(stack: &LayerStack, strip_h: u32, mut sink: F) -> Result<(), E>
where
    E: From<ComposeError>,
    F: FnMut(Strip<'_>) -> Result<(), E>,
{
    if strip_h == 0 {
        return Err(ComposeError::InvalidStripHeight.into());
    }
    let (width, height) = stack.dimensions();
    let strip_rows = strip_h.min(height);
    let mut buf = try_vec(width as usize * strip_rows as usize * 3)?;
    let mut scratch = Scratch::default();
    let mut y = 0;
    while y < height {
        let h = strip_rows.min(height - y);
        let len = width as usize * h as usize * 3;
        compose_band(
            stack,
            PixelRect::new(0, y, width, h),
            &mut scratch,
            &mut buf[..len],
        )?;
        sink(Strip {
            y,
            width,
            height: h,
            rgb: &buf[..len],
        })?;
        y += h;
    }
    Ok(())
}

/// Flattens the whole stack into one RGB8 buffer, strip by strip (the WebP and rescale path,
/// FR-EXP-09). `on_strip` runs after each strip with the rows done so far; returning an error
/// stops the flatten (cancel).
pub fn flatten_full<E, F>(stack: &LayerStack, strip_h: u32, mut on_strip: F) -> Result<RgbImage, E>
where
    E: From<ComposeError>,
    F: FnMut(u32) -> Result<(), E>,
{
    let (width, height) = stack.dimensions();
    let row = width as usize * 3;
    let mut out = try_vec(row * height as usize)?;
    flatten_strips(stack, strip_h, |s: Strip<'_>| {
        let start = s.y as usize * row;
        out[start..start + s.rgb.len()].copy_from_slice(s.rgb);
        on_strip(s.y + s.height)
    })?;
    RgbImage::from_raw(width, height, out).ok_or_else(|| ComposeError::OutOfMemory.into())
}

/// Reusable per-call buffers.
#[derive(Default)]
struct Scratch {
    rgba: Vec<u8>,
}

/// Composes one band (at most a strip) into `out` (RGB8, `band.width × band.height × 3`).
fn compose_band(
    stack: &LayerStack,
    band: PixelRect,
    scratch: &mut Scratch,
    out: &mut [u8],
) -> Result<(), ComposeError> {
    let (bw, bh) = (band.width as usize, band.height as usize);
    let need = bw * bh * 4;
    let mut rgba = std::mem::take(&mut scratch.rgba);
    rgba.clear();
    rgba.try_reserve_exact(need)
        .map_err(|_| ComposeError::OutOfMemory)?;

    // The original over opaque black: it is opaque, so its pixels are copied as they are.
    let base = stack.base.as_raw();
    let base_row = stack.base.width() as usize * 3;
    for row in 0..bh {
        let start = (band.y as usize + row) * base_row + band.x as usize * 3;
        for px in base[start..start + bw * 3].as_chunks::<3>().0 {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
    }

    let size = IntSize::from_wh(band.width, band.height).ok_or(ComposeError::OutOfMemory)?;
    let mut canvas = Pixmap::from_vec(rgba, size).ok_or(ComposeError::OutOfMemory)?;
    let (bx0, by0) = (f64::from(band.x), f64::from(band.y));
    let (bx1, by1) = (bx0 + bw as f64, by0 + bh as f64);
    for layer in &stack.layers {
        let [x0, y0, x1, y1] = layer.bounds;
        if x1 <= bx0 || x0 >= bx1 || y1 <= by0 || y0 >= by1 {
            continue;
        }
        let mut paint = PixmapPaint {
            opacity: layer.opacity,
            blend_mode: BlendMode::SourceOver,
            quality: FilterQuality::Nearest,
        };
        match layer.draw {
            Draw::Integer { x, y } => {
                let dx = x - i64::from(band.x);
                let dy = y - i64::from(band.y);
                // Bounds overlap the band, so both offsets fit in i32 (sides are ≤ 16384 px).
                let (Ok(dx), Ok(dy)) = (i32::try_from(dx), i32::try_from(dy)) else {
                    continue;
                };
                canvas.draw_pixmap(
                    dx,
                    dy,
                    layer.pixmap.as_ref(),
                    &paint,
                    Transform::identity(),
                    None,
                );
            }
            Draw::Affine(m) => {
                paint.quality = FilterQuality::Bicubic;
                let ts = Transform::from_row(
                    m[0] as f32,
                    m[1] as f32,
                    m[2] as f32,
                    m[3] as f32,
                    (m[4] - bx0) as f32,
                    (m[5] - by0) as f32,
                );
                canvas.draw_pixmap(0, 0, layer.pixmap.as_ref(), &paint, ts, None);
            }
        }
    }

    // Every pixel is opaque (opaque base, source-over), so premultiplied RGB is the colour.
    let data = canvas.take();
    for (dst, src) in out
        .as_chunks_mut::<3>()
        .0
        .iter_mut()
        .zip(data.as_chunks::<4>().0)
    {
        *dst = [src[0], src[1], src[2]];
    }
    scratch.rgba = data;
    Ok(())
}

/// Straight RGBA8 → a premultiplied tiny-skia pixmap with a transparent border of `pad` px.
fn premultiply(image: &RgbaImage, pad: u32) -> Result<Pixmap, ComposeError> {
    let (w, h) = image.dimensions();
    let (pw, ph) = (w + 2 * pad, h + 2 * pad);
    let size = IntSize::from_wh(pw, ph).ok_or(ComposeError::OutOfMemory)?;
    let mut data = try_vec(pw as usize * ph as usize * 4)?;
    let src = image.as_raw();
    for y in 0..h as usize {
        let src_row = &src[y * w as usize * 4..(y + 1) * w as usize * 4];
        let dst_start = ((y + pad as usize) * pw as usize + pad as usize) * 4;
        let dst_row = &mut data[dst_start..dst_start + w as usize * 4];
        for (d, s) in dst_row
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(src_row.as_chunks::<4>().0)
        {
            let p: PremultipliedColorU8 = ColorU8::from_rgba(s[0], s[1], s[2], s[3]).premultiply();
            *d = [p.red(), p.green(), p.blue(), p.alpha()];
        }
    }
    Pixmap::from_vec(data, size).ok_or(ComposeError::OutOfMemory)
}

/// Image-space bounding box of a `w × h` pixmap under `m`, widened by a pixel for the filter.
fn affine_bounds(m: &[f64; 6], w: f64, h: f64) -> [f64; 4] {
    let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in corners {
        let (ix, iy) = (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        b = [b[0].min(ix), b[1].min(iy), b[2].max(ix), b[3].max(iy)];
    }
    [b[0] - 1.0, b[1] - 1.0, b[2] + 1.0, b[3] + 1.0]
}

/// A zeroed buffer reserved fallibly (FR-EXP-23).
pub(crate) fn try_vec(len: usize) -> Result<Vec<u8>, ComposeError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| ComposeError::OutOfMemory)?;
    v.resize(len, 0);
    Ok(v)
}

#[cfg(test)]
#[path = "compose_tests.rs"]
mod tests;
