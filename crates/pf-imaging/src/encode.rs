//! Export encoders (export spec, slice 2): PNG and JPEG streamed from strips, plus the shared
//! progress/cancel and error types (FR-EXP-09, FR-EXP-10, FR-EXP-19, FR-EXP-20).
//!
//! Both encoders read from a [`RowSource`]: the strip flattener at scale 1 (streaming, ADR 0004)
//! or a full RGB8 buffer after a rescale. Output carries no metadata: PNG has only
//! IHDR/IDAT/IEND and JPEG only the JFIF APP0 segment.

use std::cell::{Cell, RefCell};
use std::io::{self, Write};
use std::ops::ControlFlow;

use image::RgbImage;

use crate::compose::{ComposeError, STRIP_H, Strip, StripFlattener};
use crate::probe::ImageFormat;

/// PNG compression (OQ-IMG-03).
const PNG_COMPRESSION: png::Compression = png::Compression::Fast;
/// JPEG quality (OQ-EXP-06).
pub const JPEG_QUALITY: u8 = 90;
/// Largest JPEG side (the format's 16-bit size fields).
pub const JPEG_MAX_SIDE: u32 = 65535;

/// The export phases reported with progress (FR-EXP-19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Compose,
    Resize,
    Encode,
    Write,
}

impl Phase {
    /// Event spelling (`export:progress{phase}`).
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Compose => "compose",
            Phase::Resize => "resize",
            Phase::Encode => "encode",
            Phase::Write => "write",
        }
    }
}

/// Progress callback: gets the phase and a 0–1 fraction; `Break` cancels (FR-EXP-20).
pub type Progress<'a> = &'a mut dyn FnMut(Phase, f32) -> ControlFlow<()>;

/// Why an encode failed. `pf-core` maps these to the export error codes.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("canceled")]
    Canceled,
    /// A buffer couldn't be reserved (`out_of_memory`, FR-EXP-23).
    #[error("out of memory")]
    OutOfMemory,
    /// The output is too large for the format (`too_large{format, limit}`).
    #[error("{format:?} supports at most {limit} px per side")]
    TooLarge { format: ImageFormat, limit: u32 },
    /// libwebp's lossy encoder overflowed a partition (FR-EXP-12); the caller retries lossless.
    #[error("WebP partition overflow")]
    PartitionOverflow,
    /// Writing the output failed (`disk_full`, `locked`, … by kind).
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Any other encoder failure (`encode_failed`).
    #[error("encode failed: {0}")]
    Failed(String),
}

impl From<ComposeError> for EncodeError {
    fn from(e: ComposeError) -> Self {
        match e {
            ComposeError::OutOfMemory => EncodeError::OutOfMemory,
            other => EncodeError::Failed(other.to_string()),
        }
    }
}

/// Reports progress and turns `Break` into [`EncodeError::Canceled`].
pub(crate) fn report(
    progress: Progress<'_>,
    phase: Phase,
    fraction: f32,
) -> Result<(), EncodeError> {
    match progress(phase, fraction.clamp(0.0, 1.0)) {
        ControlFlow::Continue(()) => Ok(()),
        ControlFlow::Break(()) => Err(EncodeError::Canceled),
    }
}

/// RGB8 rows handed out top to bottom in bands.
pub trait RowSource {
    /// Width and height in px.
    fn dimensions(&self) -> (u32, u32);
    /// The next band of rows, or `None` after the last row.
    fn next_rows(&mut self) -> Result<Option<Strip<'_>>, EncodeError>;
}

impl RowSource for StripFlattener<'_> {
    fn dimensions(&self) -> (u32, u32) {
        StripFlattener::dimensions(self)
    }

    fn next_rows(&mut self) -> Result<Option<Strip<'_>>, EncodeError> {
        Ok(self.next_strip()?)
    }
}

/// A full RGB8 buffer read in bands of [`STRIP_H`] rows (the rescale path).
pub struct ImageRows<'a> {
    image: &'a RgbImage,
    y: u32,
}

impl<'a> ImageRows<'a> {
    pub fn new(image: &'a RgbImage) -> Self {
        ImageRows { image, y: 0 }
    }
}

impl RowSource for ImageRows<'_> {
    fn dimensions(&self) -> (u32, u32) {
        self.image.dimensions()
    }

    fn next_rows(&mut self) -> Result<Option<Strip<'_>>, EncodeError> {
        let (width, height) = self.image.dimensions();
        if self.y >= height {
            return Ok(None);
        }
        let y = self.y;
        let h = STRIP_H.min(height - y);
        let row = width as usize * 3;
        let start = y as usize * row;
        self.y += h;
        Ok(Some(Strip {
            y,
            width,
            height: h,
            rgb: &self.image.as_raw()[start..start + h as usize * row],
        }))
    }
}

/// Encodes PNG (`Compression::Fast`, RGB8, no metadata) from `rows`, streaming each band to
/// `out` as it arrives (FR-EXP-09, FR-EXP-10). Progress is reported per band as `phase`.
pub fn encode_png<W: Write>(
    rows: &mut dyn RowSource,
    out: W,
    phase: Phase,
    progress: Progress<'_>,
) -> Result<(), EncodeError> {
    let stop = Cell::new(false);
    let mut out = GuardedWriter::new(out, &stop);
    let result = png_stream(rows, &mut out, phase, progress);
    out.keep_io_error(result)
}

fn png_stream<W: Write>(
    rows: &mut dyn RowSource,
    out: W,
    phase: Phase,
    progress: Progress<'_>,
) -> Result<(), EncodeError> {
    let (width, height) = rows.dimensions();
    let mut encoder = png::Encoder::new(out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(PNG_COMPRESSION);
    let mut writer = encoder.write_header().map_err(png_error)?;
    let mut stream = writer.stream_writer().map_err(png_error)?;
    while let Some(strip) = rows.next_rows()? {
        stream.write_all(strip.rgb)?;
        let done = strip.y + strip.height;
        report(progress, phase, done as f32 / height as f32)?;
    }
    stream.finish().map_err(png_error)?;
    writer.finish().map_err(png_error)
}

fn png_error(e: png::EncodingError) -> EncodeError {
    match e {
        png::EncodingError::IoError(io) => EncodeError::Io(io),
        other => EncodeError::Failed(other.to_string()),
    }
}

/// Encodes JPEG q90 (baseline, 4:2:0, JFIF APP0 only) from `rows`. The encoder pulls rows; bands
/// are composed on demand, so at most one band is held (FR-EXP-09, FR-EXP-10). On cancel or a
/// source error the encoder is stopped at its next write.
pub fn encode_jpeg<W: Write>(
    rows: &mut dyn RowSource,
    out: W,
    phase: Phase,
    progress: Progress<'_>,
) -> Result<(), EncodeError> {
    let (width, height) = rows.dimensions();
    if width > JPEG_MAX_SIDE || height > JPEG_MAX_SIDE {
        return Err(EncodeError::TooLarge {
            format: ImageFormat::Jpeg,
            limit: JPEG_MAX_SIDE,
        });
    }
    let stop = Cell::new(false);
    let puller = JpegRows {
        width: width as u16,
        height: height as u16,
        rows: RefCell::new(rows),
        band: RefCell::new(Band::default()),
        failure: RefCell::new(None),
        stop: &stop,
        phase,
        progress: RefCell::new(progress),
    };
    let mut writer = GuardedWriter::new(out, &stop);
    let encoder = jpeg_encoder::Encoder::new(&mut writer, JPEG_QUALITY);
    let result = encoder.encode_image(&puller);
    if let Some(e) = puller.failure.into_inner() {
        return Err(e);
    }
    let result = result.map_err(|e| match e {
        jpeg_encoder::EncodingError::IoError(io) => EncodeError::Io(io),
        other => EncodeError::Failed(other.to_string()),
    });
    writer.keep_io_error(result)
}

/// The current band of rows, copied out of the source.
#[derive(Default)]
struct Band {
    y: u32,
    height: u32,
    rgb: Vec<u8>,
}

struct JpegRows<'r, 'p> {
    width: u16,
    height: u16,
    rows: RefCell<&'r mut dyn RowSource>,
    band: RefCell<Band>,
    failure: RefCell<Option<EncodeError>>,
    stop: &'r Cell<bool>,
    phase: Phase,
    progress: RefCell<Progress<'p>>,
}

impl JpegRows<'_, '_> {
    fn fail(&self, e: EncodeError) {
        self.failure.borrow_mut().get_or_insert(e);
        self.stop.set(true);
    }

    /// Makes `y` part of the current band, pulling bands forward as needed.
    fn advance_to(&self, y: u32) -> Result<(), EncodeError> {
        let mut band = self.band.borrow_mut();
        while y >= band.y + band.height {
            let mut rows = self.rows.borrow_mut();
            let Some(strip) = rows.next_rows()? else {
                return Err(EncodeError::Failed(format!(
                    "row {y} past the end of the image"
                )));
            };
            band.rgb.clear();
            band.rgb
                .try_reserve(strip.rgb.len())
                .map_err(|_| EncodeError::OutOfMemory)?;
            band.rgb.extend_from_slice(strip.rgb);
            band.y = strip.y;
            band.height = strip.height;
            let done = (strip.y + strip.height) as f32 / f32::from(self.height);
            report(&mut **self.progress.borrow_mut(), self.phase, done)?;
        }
        if y < band.y {
            return Err(EncodeError::Failed(format!(
                "row {y} requested out of order"
            )));
        }
        Ok(())
    }
}

impl jpeg_encoder::ImageBuffer for &JpegRows<'_, '_> {
    fn get_jpeg_color_type(&self) -> jpeg_encoder::JpegColorType {
        jpeg_encoder::JpegColorType::Ycbcr
    }

    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn fill_buffers(&self, y: u16, buffers: &mut [Vec<u8>; 4]) {
        let w = usize::from(self.width);
        if !self.stop.get()
            && let Err(e) = self.advance_to(u32::from(y))
        {
            self.fail(e);
        }
        if self.stop.get() {
            // The encoder can't be told to stop here; feed black and let the writer refuse.
            for b in buffers.iter_mut().take(3) {
                b.extend(std::iter::repeat_n(0, w));
            }
            return;
        }
        let band = self.band.borrow();
        let start = (u32::from(y) - band.y) as usize * w * 3;
        for px in band.rgb[start..start + w * 3].as_chunks::<3>().0 {
            let (l, cb, cr) = jpeg_encoder::rgb_to_ycbcr(px[0], px[1], px[2]);
            buffers[0].push(l);
            buffers[1].push(cb);
            buffers[2].push(cr);
        }
    }
}

/// The output writer for both encoders. It refuses writes once `stop` is set (which ends the
/// JPEG encoder early), and it keeps the first I/O error the real writer returned: the PNG
/// stream writer re-wraps errors as `ErrorKind::Other`, and `pf-core` needs the original kind and
/// OS code to tell `disk_full` and `locked` apart.
struct GuardedWriter<'s, W> {
    inner: W,
    stop: &'s Cell<bool>,
    first_error: Option<io::Error>,
}

impl<'s, W: Write> GuardedWriter<'s, W> {
    fn new(inner: W, stop: &'s Cell<bool>) -> Self {
        GuardedWriter {
            inner,
            stop,
            first_error: None,
        }
    }

    fn record(&mut self, e: io::Error) -> io::Error {
        let copy = io::Error::new(e.kind(), e.to_string());
        self.first_error.get_or_insert(e);
        copy
    }

    /// Replaces an I/O failure with the first error the real writer returned.
    fn keep_io_error(self, result: Result<(), EncodeError>) -> Result<(), EncodeError> {
        match (result, self.first_error) {
            (Err(EncodeError::Io(_)), Some(original)) => Err(EncodeError::Io(original)),
            (result, _) => result,
        }
    }
}

impl<W: Write> Write for GuardedWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.stop.get() {
            return Err(io::Error::other("export stopped"));
        }
        self.inner.write(buf).map_err(|e| self.record(e))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush().map_err(|e| self.record(e))
    }
}

#[cfg(test)]
#[path = "encode_tests.rs"]
mod tests;
