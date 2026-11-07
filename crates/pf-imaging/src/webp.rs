//! WebP export through libwebp (ADR 0004, `libwebp-sys`): lossy q90 from one full RGB8 buffer,
//! with a single lossless retry on a partition overflow (FR-EXP-10..12). Progress and cancel go
//! through libwebp's progress hook (FR-EXP-19, FR-EXP-20).
//!
//! This is the crate's only FFI module, and the only place `unsafe` is allowed.
#![allow(unsafe_code)]

use std::ffi::{c_int, c_void};
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};

use image::RgbImage;
use libwebp_sys as sys;

use crate::encode::{EncodeError, Phase, Progress, report};
use crate::probe::ImageFormat;

/// Largest WebP side (the format's 14-bit size fields, OQ-IMG-04).
pub const WEBP_MAX_SIDE: u32 = 16383;
/// Lossy quality (OQ-EXP-06).
pub const WEBP_QUALITY: f32 = 90.0;
/// Lossless effort for the fallback: favours speed, since a fallback image is a large one.
const LOSSLESS_LEVEL: c_int = 1;

/// How one WebP encode attempt runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WebpMode {
    Lossy { quality: f32 },
    Lossless,
}

/// One WebP encode attempt. [`LibWebp`] is the real one; tests and `pf-core` can substitute a
/// fake to exercise the lossless fallback (FR-EXP-12).
pub trait WebpEncoder {
    /// Encodes `image`. `progress` gets 0–1; `Break` aborts with [`EncodeError::Canceled`].
    fn encode(
        &self,
        image: &RgbImage,
        mode: WebpMode,
        progress: &mut dyn FnMut(f32) -> ControlFlow<()>,
    ) -> Result<Vec<u8>, EncodeError>;
}

/// A finished WebP export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebpOutput {
    pub bytes: Vec<u8>,
    /// The lossy encode overflowed and the file is lossless (`fallback: lossless`).
    pub lossless_fallback: bool,
}

/// Encodes `image` as lossy WebP q90 with libwebp, retrying once as lossless on a partition
/// overflow (FR-EXP-10..12).
pub fn encode_webp(image: &RgbImage, progress: Progress<'_>) -> Result<WebpOutput, EncodeError> {
    encode_webp_with(&LibWebp, image, progress)
}

/// [`encode_webp`] with an injected encoder (the FR-EXP-12 seam).
pub fn encode_webp_with(
    encoder: &dyn WebpEncoder,
    image: &RgbImage,
    progress: Progress<'_>,
) -> Result<WebpOutput, EncodeError> {
    let (w, h) = image.dimensions();
    if w > WEBP_MAX_SIDE || h > WEBP_MAX_SIDE {
        return Err(EncodeError::TooLarge {
            format: ImageFormat::Webp,
            limit: WEBP_MAX_SIDE,
        });
    }
    let mut hook = |f: f32| match report(progress, Phase::Encode, f) {
        Ok(()) => ControlFlow::Continue(()),
        Err(_) => ControlFlow::Break(()),
    };
    let lossy = WebpMode::Lossy {
        quality: WEBP_QUALITY,
    };
    let output = match encoder.encode(image, lossy, &mut hook) {
        Ok(bytes) => WebpOutput {
            bytes,
            lossless_fallback: false,
        },
        Err(EncodeError::PartitionOverflow) => WebpOutput {
            bytes: encoder.encode(image, WebpMode::Lossless, &mut hook)?,
            lossless_fallback: true,
        },
        Err(e) => return Err(e),
    };
    // libwebp's last report can be short of 100 %.
    report(progress, Phase::Encode, 1.0)?;
    Ok(output)
}

/// The libwebp encoder (`WebPEncode` with a custom writer and progress hook).
#[derive(Debug, Default, Clone, Copy)]
pub struct LibWebp;

/// State shared with the C callbacks through `custom_ptr` / `user_data`.
struct HookState<'a> {
    progress: &'a mut dyn FnMut(f32) -> ControlFlow<()>,
    out: Vec<u8>,
    out_of_memory: bool,
    panicked: bool,
}

/// Frees the picture's buffers however the encode ends.
struct PictureGuard(sys::WebPPicture);

impl Drop for PictureGuard {
    fn drop(&mut self) {
        // SAFETY: the picture was initialised by `WebPPicture::new`; freeing an unallocated
        // picture is a no-op in libwebp.
        unsafe { sys::WebPPictureFree(&mut self.0) };
    }
}

impl WebpEncoder for LibWebp {
    fn encode(
        &self,
        image: &RgbImage,
        mode: WebpMode,
        progress: &mut dyn FnMut(f32) -> ControlFlow<()>,
    ) -> Result<Vec<u8>, EncodeError> {
        let (w, h) = image.dimensions();
        let config = config(mode)?;
        let mut picture = PictureGuard(
            sys::WebPPicture::new().map_err(|()| EncodeError::Failed("WebPPictureInit".into()))?,
        );
        let pic = &mut picture.0;
        pic.width = w as c_int;
        pic.height = h as c_int;
        // Lossless needs ARGB input; lossy converts to YUV on import.
        pic.use_argb = c_int::from(mode == WebpMode::Lossless);
        let stride = (w * 3) as c_int;
        // SAFETY: `image` holds `h` rows of `stride` bytes; libwebp copies them into the picture.
        if unsafe { sys::WebPPictureImportRGB(pic, image.as_raw().as_ptr(), stride) } == 0 {
            return Err(EncodeError::OutOfMemory);
        }

        let mut state = HookState {
            progress,
            out: Vec::new(),
            out_of_memory: false,
            panicked: false,
        };
        let state_ptr = (&mut state as *mut HookState<'_>).cast::<c_void>();
        pic.writer = Some(write_cb);
        pic.custom_ptr = state_ptr;
        pic.progress_hook = Some(progress_cb);
        pic.user_data = state_ptr;

        // SAFETY: config and picture are valid and initialised; `state` outlives the call and
        // the callbacks only run on this thread during it (thread_level is 0).
        let ok = unsafe { sys::WebPEncode(&config, pic) } != 0;
        if ok {
            return Ok(state.out);
        }
        use sys::WebPEncodingError as E;
        Err(match pic.error_code {
            E::VP8_ENC_ERROR_PARTITION0_OVERFLOW | E::VP8_ENC_ERROR_PARTITION_OVERFLOW => {
                EncodeError::PartitionOverflow
            }
            E::VP8_ENC_ERROR_USER_ABORT if state.panicked => {
                EncodeError::Failed("progress callback panicked".into())
            }
            E::VP8_ENC_ERROR_USER_ABORT => EncodeError::Canceled,
            E::VP8_ENC_ERROR_BAD_WRITE if state.out_of_memory => EncodeError::OutOfMemory,
            E::VP8_ENC_ERROR_OUT_OF_MEMORY | E::VP8_ENC_ERROR_BITSTREAM_OUT_OF_MEMORY => {
                EncodeError::OutOfMemory
            }
            E::VP8_ENC_ERROR_BAD_DIMENSION => EncodeError::TooLarge {
                format: ImageFormat::Webp,
                limit: WEBP_MAX_SIDE,
            },
            other => EncodeError::Failed(format!("libwebp: {other:?}")),
        })
    }
}

fn config(mode: WebpMode) -> Result<sys::WebPConfig, EncodeError> {
    let failed = |what: &str| EncodeError::Failed(format!("libwebp config: {what}"));
    let mut config = match mode {
        WebpMode::Lossy { quality } => {
            sys::WebPConfig::new_with_preset(sys::WebPPreset::WEBP_PRESET_DEFAULT, quality)
                .map_err(|()| failed("init"))?
        }
        WebpMode::Lossless => {
            let mut c = sys::WebPConfig::new().map_err(|()| failed("init"))?;
            // SAFETY: `c` is an initialised config.
            if unsafe { sys::WebPConfigLosslessPreset(&mut c, LOSSLESS_LEVEL) } == 0 {
                return Err(failed("lossless preset"));
            }
            c
        }
    };
    // Single-threaded, so the callbacks never run concurrently.
    config.thread_level = 0;
    // SAFETY: `config` is initialised.
    if unsafe { sys::WebPValidateConfig(&config) } == 0 {
        return Err(failed("invalid"));
    }
    Ok(config)
}

/// libwebp writer: appends to `HookState::out`, reserving fallibly (FR-EXP-23).
unsafe extern "C" fn write_cb(
    data: *const u8,
    size: usize,
    picture: *const sys::WebPPicture,
) -> c_int {
    if size == 0 {
        return 1;
    }
    // SAFETY: libwebp passes the picture we configured; `custom_ptr` points at the live
    // `HookState`, and `data` is valid for `size` bytes for the duration of the call.
    let (state, bytes) = unsafe {
        (
            &mut *(*picture).custom_ptr.cast::<HookState<'_>>(),
            std::slice::from_raw_parts(data, size),
        )
    };
    if state.out.try_reserve(size).is_err() {
        state.out_of_memory = true;
        return 0;
    }
    state.out.extend_from_slice(bytes);
    1
}

/// libwebp progress hook: returns 0 to abort (cancel). Never unwinds into C.
unsafe extern "C" fn progress_cb(percent: c_int, picture: *const sys::WebPPicture) -> c_int {
    // SAFETY: as in `write_cb`; `user_data` points at the same live `HookState`, and the two
    // callbacks never run at the same time.
    let state = unsafe { &mut *(*picture).user_data.cast::<HookState<'_>>() };
    let fraction = percent.clamp(0, 100) as f32 / 100.0;
    match catch_unwind(AssertUnwindSafe(|| (state.progress)(fraction))) {
        Ok(ControlFlow::Continue(())) => 1,
        Ok(ControlFlow::Break(())) => 0,
        Err(_) => {
            state.panicked = true;
            0
        }
    }
}

#[cfg(test)]
#[path = "webp_tests.rs"]
mod tests;
