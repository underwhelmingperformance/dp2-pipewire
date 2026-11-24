// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::{Dimensions, FrameError, VideoFrame};

/// Downsampled RGBA8 preview image suitable for inline UI display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrame {
    /// Preview frame dimensions in pixels.
    pub dimensions: Dimensions,
    /// Packed RGBA8 pixel data (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
    /// Source frame sequence number.
    pub sequence: u64,
}

/// Downsamples `frame` to a packed RGBA8 [`PreviewFrame`] whose width does not
/// exceed `max_width` while preserving aspect ratio.
///
/// # Errors
///
/// Returns [`FrameError::ZeroDimension`] if `max_width` is zero.
///
/// # Examples
///
/// ```
/// use dp2_core::{Dimensions, PixelFormat, VideoFrame, downsample_preview};
///
/// let frame = VideoFrame::packed(
///     Dimensions::new(4, 2)?,
///     PixelFormat::Rgb8,
///     0,
///     7,
///     vec![200; 24],
/// )?;
/// let preview = downsample_preview(&frame, 2)?;
/// assert_eq!(preview.dimensions, Dimensions::new(2, 1)?);
/// assert_eq!(preview.rgba, vec![200, 200, 200, 255, 200, 200, 200, 255]);
/// assert_eq!(preview.sequence, 7);
/// # Ok::<(), dp2_core::FrameError>(())
/// ```
pub fn downsample_preview(frame: &VideoFrame, max_width: u32) -> Result<PreviewFrame, FrameError> {
    if max_width == 0 {
        return Err(FrameError::ZeroDimension {
            width: 0,
            height: frame.dimensions().height(),
        });
    }

    let src_w = frame.dimensions().width();
    let src_h = frame.dimensions().height();
    let (dst_w, dst_h) = if src_w <= max_width {
        (src_w, src_h)
    } else {
        let scaled_h = (u64::from(src_h) * u64::from(max_width)).div_ceil(u64::from(src_w));
        let dst_h = u32::try_from(scaled_h).unwrap_or(src_h).max(1);
        (max_width, dst_h)
    };
    let dimensions = Dimensions::new(dst_w, dst_h)?;
    let rgb = frame.to_rgb8();

    let src_width = src_w as usize;
    let src_height = src_h as usize;
    let dst_width = dst_w as usize;
    let dst_height = dst_h as usize;

    let mut rgba = Vec::with_capacity(dst_width * dst_height * 4);
    for y in 0..dst_height {
        let src_y = (y * src_height) / dst_height;
        for x in 0..dst_width {
            let src_x = (x * src_width) / dst_width;
            let idx = (src_y * src_width + src_x) * 3;
            rgba.extend_from_slice(&[rgb[idx], rgb[idx + 1], rgb[idx + 2], 255]);
        }
    }

    Ok(PreviewFrame {
        dimensions,
        rgba,
        sequence: frame.sequence(),
    })
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::PixelFormat;

    #[test]
    fn downsample_preview_preserves_aspect_ratio() -> Result<(), FrameError> {
        let frame = VideoFrame::packed(
            Dimensions::new(640, 480)?,
            PixelFormat::Rgba8,
            0,
            3,
            vec![42; 640 * 480 * 4],
        )?;
        let preview = downsample_preview(&frame, 240)?;
        assert_eq!(preview.dimensions, Dimensions::new(240, 180)?);
        assert_eq!(preview.rgba.len(), 240 * 180 * 4);
        assert_eq!(preview.sequence, 3);
        Ok(())
    }

    #[test]
    fn downsample_preview_rejects_zero_width() -> Result<(), FrameError> {
        let frame =
            VideoFrame::packed(Dimensions::new(2, 2)?, PixelFormat::Rgb8, 0, 1, vec![0; 12])?;
        assert_matches!(
            downsample_preview(&frame, 0),
            Err(FrameError::ZeroDimension { .. })
        );
        Ok(())
    }
}
