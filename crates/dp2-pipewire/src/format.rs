// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use dp2_core::{Dimensions, PixelFormat, VideoFrame};

use crate::PipeWireError;

/// Subset of SPA raw video formats supported by `dp2-pipewire`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpaVideoFormat {
    /// 32-bit RGBA (`SPA_VIDEO_FORMAT_RGBA`).
    Rgba,
    /// 32-bit `RGBx` (`SPA_VIDEO_FORMAT_RGBx`).
    Rgbx,
    /// 32-bit BGRA (`SPA_VIDEO_FORMAT_BGRA`).
    Bgra,
    /// 32-bit `BGRx` (`SPA_VIDEO_FORMAT_BGRx`).
    Bgrx,
    /// 24-bit RGB (`SPA_VIDEO_FORMAT_RGB`).
    Rgb,
    /// 24-bit BGR (`SPA_VIDEO_FORMAT_BGR`).
    Bgr,
    /// Packed 4:2:2 YUV (`SPA_VIDEO_FORMAT_YUY2`).
    Yuy2,
    /// Planar 4:2:0 YUV (`SPA_VIDEO_FORMAT_I420`).
    I420,
    /// Semi-planar 4:2:0 YUV (`SPA_VIDEO_FORMAT_NV12`).
    Nv12,
}

impl SpaVideoFormat {
    /// Maps an interleaved [`PixelFormat`] to its corresponding
    /// [`SpaVideoFormat`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::PixelFormat;
    /// use dp2_pipewire::SpaVideoFormat;
    ///
    /// assert_eq!(
    ///     SpaVideoFormat::from_pixel_format(PixelFormat::Rgba8),
    ///     SpaVideoFormat::Rgba
    /// );
    /// ```
    #[must_use]
    pub const fn from_pixel_format(format: PixelFormat) -> Self {
        match format {
            PixelFormat::Rgba8 => Self::Rgba,
            PixelFormat::Bgra8 => Self::Bgra,
            PixelFormat::Rgb8 => Self::Rgb,
            PixelFormat::Bgr8 => Self::Bgr,
        }
    }
}

/// Metadata describing bytes written into an output PipeWire SPA buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodedBuffer {
    /// Total valid bytes written into the destination slice.
    pub size: usize,
    /// Row stride in bytes written into the destination slice.
    pub stride: usize,
}

/// Decodes raw bytes from a PipeWire capture buffer into a [`VideoFrame`].
///
/// RGB and BGR layouts are copied directly with their native stride
/// (normalising `RGBx` to `Rgba8` and `BGRx` to `Bgra8`). `YUY2`, `I420`, and
/// `NV12` buffers are converted to packed `Rgba8`.
///
/// # Errors
///
/// Returns [`PipeWireError::Buffer`] if `raw` does not contain enough bytes for
/// the negotiated dimensions and pixel format, or propagates
/// [`PipeWireError::Frame`].
///
/// # Examples
///
/// ```
/// use dp2_core::{Dimensions, PixelFormat};
/// use dp2_pipewire::{SpaVideoFormat, decode_spa_frame};
///
/// let dims = Dimensions::new(2, 1)?;
/// let raw = [10, 20, 30, 255, 40, 50, 60, 255];
/// let frame = decode_spa_frame(&raw, 8, dims, SpaVideoFormat::Rgba, 1_000, 1)?;
/// assert_eq!(frame.format(), PixelFormat::Rgba8);
/// assert_eq!(frame.data(), &raw);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn decode_spa_frame(
    raw: &[u8],
    stride: usize,
    dimensions: Dimensions,
    format: SpaVideoFormat,
    timestamp_ns: u64,
    sequence: u64,
) -> Result<VideoFrame, PipeWireError> {
    match format {
        SpaVideoFormat::Rgba | SpaVideoFormat::Rgbx => decode_direct(
            raw,
            stride,
            dimensions,
            PixelFormat::Rgba8,
            timestamp_ns,
            sequence,
        ),
        SpaVideoFormat::Bgra | SpaVideoFormat::Bgrx => decode_direct(
            raw,
            stride,
            dimensions,
            PixelFormat::Bgra8,
            timestamp_ns,
            sequence,
        ),
        SpaVideoFormat::Rgb => decode_direct(
            raw,
            stride,
            dimensions,
            PixelFormat::Rgb8,
            timestamp_ns,
            sequence,
        ),
        SpaVideoFormat::Bgr => decode_direct(
            raw,
            stride,
            dimensions,
            PixelFormat::Bgr8,
            timestamp_ns,
            sequence,
        ),
        SpaVideoFormat::Yuy2 => decode_yuy2(raw, stride, dimensions, timestamp_ns, sequence),
        SpaVideoFormat::I420 => decode_i420(raw, dimensions, timestamp_ns, sequence),
        SpaVideoFormat::Nv12 => decode_nv12(raw, dimensions, timestamp_ns, sequence),
    }
}

fn decode_yuy2(
    raw: &[u8],
    stride: usize,
    dimensions: Dimensions,
    timestamp_ns: u64,
    sequence: u64,
) -> Result<VideoFrame, PipeWireError> {
    let width = dimensions.width() as usize;
    let height = dimensions.height() as usize;
    let row_stride = if stride == 0 { width * 2 } else { stride };
    let required = row_stride * height;
    if raw.len() < required {
        return Err(PipeWireError::Buffer(format!(
            "YUY2 buffer too small: got {}, need {required}",
            raw.len()
        )));
    }

    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let row = &raw[y * row_stride..y * row_stride + width * 2];
        let (pairs, remainder) = row.as_chunks::<4>();
        for &[y0, cb, y1, cr] in pairs {
            let [r0, g0, b0] = yuv_to_rgb(y0, cb, cr);
            let [r1, g1, b1] = yuv_to_rgb(y1, cb, cr);
            rgba.extend_from_slice(&[r0, g0, b0, 255, r1, g1, b1, 255]);
        }
        if let Some(&luma) = remainder.first() {
            let [red, green, blue] = yuv_to_rgb(luma, 128, 128);
            rgba.extend_from_slice(&[red, green, blue, 255]);
        }
    }

    Ok(VideoFrame::packed(
        dimensions,
        PixelFormat::Rgba8,
        timestamp_ns,
        sequence,
        rgba,
    )?)
}

fn decode_i420(
    raw: &[u8],
    dimensions: Dimensions,
    timestamp_ns: u64,
    sequence: u64,
) -> Result<VideoFrame, PipeWireError> {
    let width = dimensions.width() as usize;
    let height = dimensions.height() as usize;
    let y_size = width * height;
    let uv_w = width.div_ceil(2);
    let uv_h = height.div_ceil(2);
    let uv_size = uv_w * uv_h;
    let required = y_size + 2 * uv_size;
    if raw.len() < required {
        return Err(PipeWireError::Buffer(format!(
            "I420 buffer too small: got {}, need {required}",
            raw.len()
        )));
    }

    let y_plane = &raw[..y_size];
    let u_plane = &raw[y_size..y_size + uv_size];
    let v_plane = &raw[y_size + uv_size..required];
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let luma = y_plane[y * width + x];
            let uv_idx = (y / 2) * uv_w + (x / 2);
            let [red, green, blue] = yuv_to_rgb(luma, u_plane[uv_idx], v_plane[uv_idx]);
            rgba.extend_from_slice(&[red, green, blue, 255]);
        }
    }

    Ok(VideoFrame::packed(
        dimensions,
        PixelFormat::Rgba8,
        timestamp_ns,
        sequence,
        rgba,
    )?)
}

fn decode_nv12(
    raw: &[u8],
    dimensions: Dimensions,
    timestamp_ns: u64,
    sequence: u64,
) -> Result<VideoFrame, PipeWireError> {
    let width = dimensions.width() as usize;
    let height = dimensions.height() as usize;
    let y_size = width * height;
    let uv_w = width.div_ceil(2);
    let uv_h = height.div_ceil(2);
    let required = y_size + uv_w * uv_h * 2;
    if raw.len() < required {
        return Err(PipeWireError::Buffer(format!(
            "NV12 buffer too small: got {}, need {required}",
            raw.len()
        )));
    }

    let y_plane = &raw[..y_size];
    let uv_plane = &raw[y_size..required];
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let luma = y_plane[y * width + x];
            let uv_idx = ((y / 2) * uv_w + (x / 2)) * 2;
            let [red, green, blue] = yuv_to_rgb(luma, uv_plane[uv_idx], uv_plane[uv_idx + 1]);
            rgba.extend_from_slice(&[red, green, blue, 255]);
        }
    }

    Ok(VideoFrame::packed(
        dimensions,
        PixelFormat::Rgba8,
        timestamp_ns,
        sequence,
        rgba,
    )?)
}

/// Encodes `frame` into a PipeWire output SPA buffer slice with the negotiated
/// dimensions and pixel format.
///
/// When `frame.dimensions()` differs from `dst_dimensions`, nearest-neighbour
/// scaling is applied so the virtual camera output remains valid even across
/// input resolution changes.
///
/// # Errors
///
/// Returns [`PipeWireError::Buffer`] if `dst` is smaller than the encoded frame
/// size.
///
/// # Examples
///
/// ```
/// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
/// use dp2_pipewire::{EncodedBuffer, encode_spa_frame};
///
/// let dims = Dimensions::new(2, 1)?;
/// let frame = VideoFrame::packed(dims, PixelFormat::Rgb8, 0, 1, vec![10, 20, 30, 40, 50, 60])?;
/// let mut out = vec![0u8; 8];
/// let encoded = encode_spa_frame(&frame, PixelFormat::Rgba8, dims, &mut out)?;
/// assert_eq!(encoded, EncodedBuffer { size: 8, stride: 8 });
/// assert_eq!(out, vec![10, 20, 30, 255, 40, 50, 60, 255]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn encode_spa_frame(
    frame: &VideoFrame,
    dst_format: PixelFormat,
    dst_dimensions: Dimensions,
    dst: &mut [u8],
) -> Result<EncodedBuffer, PipeWireError> {
    let dst_w = dst_dimensions.width() as usize;
    let dst_h = dst_dimensions.height() as usize;
    let dst_bpp = dst_format.bytes_per_pixel();
    let dst_stride = dst_w * dst_bpp;
    let dst_size = dst_stride * dst_h;

    if dst.len() < dst_size {
        return Err(PipeWireError::Buffer(format!(
            "PipeWire output buffer too small: got {}, need {dst_size}",
            dst.len()
        )));
    }

    if frame.dimensions() == dst_dimensions && frame.format() == dst_format {
        let src = frame.data();
        let src_stride = frame.stride();
        for y in 0..dst_h {
            let src_row = &src[y * src_stride..y * src_stride + dst_stride];
            let dst_row = &mut dst[y * dst_stride..(y + 1) * dst_stride];
            dst_row.copy_from_slice(src_row);
        }
        return Ok(EncodedBuffer {
            size: dst_size,
            stride: dst_stride,
        });
    }

    let rgb = frame.to_rgb8();
    let src_w = frame.dimensions().width() as usize;
    let src_h = frame.dimensions().height() as usize;

    for y in 0..dst_h {
        let src_y = (y * src_h) / dst_h;
        for x in 0..dst_w {
            let src_x = (x * src_w) / dst_w;
            let src_idx = (src_y * src_w + src_x) * 3;
            let r = rgb[src_idx];
            let g = rgb[src_idx + 1];
            let b = rgb[src_idx + 2];
            let dst_idx = y * dst_stride + x * dst_bpp;
            match dst_format {
                PixelFormat::Rgba8 => {
                    dst[dst_idx..dst_idx + 4].copy_from_slice(&[r, g, b, 255]);
                }
                PixelFormat::Bgra8 => {
                    dst[dst_idx..dst_idx + 4].copy_from_slice(&[b, g, r, 255]);
                }
                PixelFormat::Rgb8 => {
                    dst[dst_idx..dst_idx + 3].copy_from_slice(&[r, g, b]);
                }
                PixelFormat::Bgr8 => {
                    dst[dst_idx..dst_idx + 3].copy_from_slice(&[b, g, r]);
                }
            }
        }
    }

    Ok(EncodedBuffer {
        size: dst_size,
        stride: dst_stride,
    })
}

fn decode_direct(
    raw: &[u8],
    stride: usize,
    dimensions: Dimensions,
    format: PixelFormat,
    timestamp_ns: u64,
    sequence: u64,
) -> Result<VideoFrame, PipeWireError> {
    let min_stride = (dimensions.width() as usize) * format.bytes_per_pixel();
    let effective_stride = if stride == 0 { min_stride } else { stride };
    let required = effective_stride * (dimensions.height() as usize);
    if raw.len() < required {
        return Err(PipeWireError::Buffer(format!(
            "raw video buffer too small: got {}, need {required}",
            raw.len()
        )));
    }
    Ok(VideoFrame::new(
        dimensions,
        format,
        effective_stride,
        timestamp_ns,
        sequence,
        raw[..required].to_vec(),
    )?)
}

fn yuv_to_rgb(luma: u8, cb: u8, cr: u8) -> [u8; 3] {
    let y_shifted = i32::from(luma) - 16;
    let chroma_blue = i32::from(cb) - 128;
    let chroma_red = i32::from(cr) - 128;

    let red = (298 * y_shifted + 409 * chroma_red + 128) >> 8;
    let green = (298 * y_shifted - 100 * chroma_blue - 208 * chroma_red + 128) >> 8;
    let blue = (298 * y_shifted + 516 * chroma_blue + 128) >> 8;

    [clamp_u8(red), clamp_u8(green), clamp_u8(blue)]
}

fn clamp_u8(value: i32) -> u8 {
    u8::try_from(value.clamp(0, 255)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn decode_yuy2_converts_neutral_grey() -> Result<(), Box<dyn std::error::Error>> {
        let dims = Dimensions::new(2, 1)?;
        let yuy2 = [126u8, 128, 126, 128];
        let frame = decode_spa_frame(&yuy2, 4, dims, SpaVideoFormat::Yuy2, 10, 1)?;
        assert_eq!(frame.format(), PixelFormat::Rgba8);
        assert_eq!(frame.data(), &[128, 128, 128, 255, 128, 128, 128, 255]);
        Ok(())
    }

    #[test]
    fn decode_i420_and_nv12_match_for_uniform_frame() -> Result<(), Box<dyn std::error::Error>> {
        let dims = Dimensions::new(2, 2)?;
        let i420 = [235u8, 235, 235, 235, 128, 128];
        let nv12 = [235u8, 235, 235, 235, 128, 128];

        let from_i420 = decode_spa_frame(&i420, 0, dims, SpaVideoFormat::I420, 0, 1)?;
        let from_nv12 = decode_spa_frame(&nv12, 0, dims, SpaVideoFormat::Nv12, 0, 1)?;
        assert_eq!(from_i420, from_nv12);
        assert_eq!(
            from_i420.data(),
            &[
                255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
            ]
        );
        Ok(())
    }

    #[test]
    fn encode_spa_frame_resizes_and_converts_format() -> Result<(), Box<dyn std::error::Error>> {
        let frame = VideoFrame::packed(
            Dimensions::new(2, 1)?,
            PixelFormat::Rgb8,
            0,
            0,
            vec![10, 20, 30, 40, 50, 60],
        )?;
        let mut out = vec![0u8; 4 * 4];
        let encoded =
            encode_spa_frame(&frame, PixelFormat::Bgra8, Dimensions::new(2, 2)?, &mut out)?;
        assert_eq!(
            encoded,
            EncodedBuffer {
                size: 16,
                stride: 8
            }
        );
        assert_eq!(
            out,
            vec![
                30, 20, 10, 255, 60, 50, 40, 255, 30, 20, 10, 255, 60, 50, 40, 255,
            ]
        );
        Ok(())
    }

    #[test]
    fn encode_rejects_short_destination_buffer() -> Result<(), Box<dyn std::error::Error>> {
        let frame = VideoFrame::packed(
            Dimensions::new(2, 2)?,
            PixelFormat::Rgba8,
            0,
            0,
            vec![0; 16],
        )?;
        let mut tiny = vec![0u8; 4];
        assert_matches!(
            encode_spa_frame(
                &frame,
                PixelFormat::Rgba8,
                Dimensions::new(2, 2)?,
                &mut tiny
            ),
            Err(PipeWireError::Buffer(_))
        );
        Ok(())
    }
}
