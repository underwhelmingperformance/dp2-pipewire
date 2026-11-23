// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use dp2_core::{DepthError, Dimensions, VideoFrame};
use ndarray::Array4;

/// Computes the horizontal focal length in pixels for a pinhole camera with
/// `width` pixels and horizontal field of view `horizontal_fov_deg` degrees.
///
/// # Errors
///
/// Returns [`DepthError::InvalidFocalLength`] if `width` is zero or
/// `horizontal_fov_deg` is not strictly within `(0.0, 180.0)`.
///
/// # Examples
///
/// ```
/// use dp2_depth::focal_length_from_fov;
///
/// let f_px = focal_length_from_fov(640, 90.0)?;
/// assert!((f_px - 320.0).abs() < 1e-4);
/// # Ok::<(), dp2_core::DepthError>(())
/// ```
pub fn focal_length_from_fov(width: u32, horizontal_fov_deg: f32) -> Result<f32, DepthError> {
    if width == 0
        || !horizontal_fov_deg.is_finite()
        || horizontal_fov_deg <= 0.0
        || horizontal_fov_deg >= 180.0
    {
        return Err(DepthError::InvalidFocalLength {
            focal_length_px: f32::NAN,
        });
    }

    let half_rad = (horizontal_fov_deg.to_radians()) * 0.5;
    #[expect(
        clippy::cast_precision_loss,
        reason = "video frame widths are well below 2^24"
    )]
    let f_px = (width as f32) / (2.0 * half_rad.tan());
    if !f_px.is_finite() || f_px <= 0.0 {
        return Err(DepthError::InvalidFocalLength {
            focal_length_px: f_px,
        });
    }

    Ok(f_px)
}

/// Scales a horizontal focal length from `input_width` to `model_width`.
///
/// # Examples
///
/// ```
/// use dp2_depth::scale_focal_length;
///
/// assert!((scale_focal_length(1280.0, 1280, 640) - 640.0).abs() < 1e-5);
/// ```
#[must_use]
pub fn scale_focal_length(f_px_for_input: f32, input_width: u32, model_width: u32) -> f32 {
    if input_width == model_width || input_width == 0 {
        return f_px_for_input;
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "video frame widths are well below 2^24"
    )]
    let ratio = (model_width as f32) / (input_width as f32);
    f_px_for_input * ratio
}

/// Converts a [`VideoFrame`] into an NCHW `f32` tensor `[1, 3, target_h,
/// target_w]` with values normalised into `[0.0, 1.0]`, using bilinear
/// interpolation when resizing.
///
/// # Examples
///
/// ```
/// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
/// use dp2_depth::frame_to_nchw_f32;
///
/// let frame = VideoFrame::packed(
///     Dimensions::new(2, 1)?,
///     PixelFormat::Rgb8,
///     0,
///     1,
///     vec![255, 0, 0, 0, 255, 0],
/// )?;
/// let tensor = frame_to_nchw_f32(&frame, Dimensions::new(2, 1)?);
/// assert_eq!(tensor.shape(), &[1, 3, 1, 2]);
/// assert_eq!(tensor[[0, 0, 0, 0]], 1.0);
/// assert_eq!(tensor[[0, 1, 0, 1]], 1.0);
/// # Ok::<(), dp2_core::FrameError>(())
/// ```
#[must_use]
pub fn frame_to_nchw_f32(frame: &VideoFrame, target: Dimensions) -> Array4<f32> {
    let rgb = frame.to_rgb8();
    let src_w = frame.dimensions().width() as usize;
    let src_h = frame.dimensions().height() as usize;
    let dst_w = target.width() as usize;
    let dst_h = target.height() as usize;

    let mut tensor = Array4::<f32>::zeros((1, 3, dst_h, dst_w));

    if src_w == dst_w && src_h == dst_h {
        for y in 0..dst_h {
            for x in 0..dst_w {
                let idx = (y * dst_w + x) * 3;
                tensor[[0, 0, y, x]] = f32::from(rgb[idx]) / 255.0;
                tensor[[0, 1, y, x]] = f32::from(rgb[idx + 1]) / 255.0;
                tensor[[0, 2, y, x]] = f32::from(rgb[idx + 2]) / 255.0;
            }
        }
        return tensor;
    }

    for y in 0..dst_h {
        let (y0, y1, fy) = sample_axis(y, src_h, dst_h);
        for x in 0..dst_w {
            let (x0, x1, fx) = sample_axis(x, src_w, dst_w);
            let i00 = (y0 * src_w + x0) * 3;
            let i01 = (y0 * src_w + x1) * 3;
            let i10 = (y1 * src_w + x0) * 3;
            let i11 = (y1 * src_w + x1) * 3;

            for c in 0..3 {
                let v00 = f32::from(rgb[i00 + c]);
                let v01 = f32::from(rgb[i01 + c]);
                let v10 = f32::from(rgb[i10 + c]);
                let v11 = f32::from(rgb[i11 + c]);

                let top = v00 + (v01 - v00) * fx;
                let bottom = v10 + (v11 - v10) * fx;
                tensor[[0, c, y, x]] = (top + (bottom - top) * fy) / 255.0;
            }
        }
    }

    tensor
}

/// Resizes a row-major metric depth buffer from `src` to `dst` using bilinear
/// interpolation.
///
/// Non-finite samples (such as infinite sky depth) are excluded from weighted
/// averages so they never turn neighbouring finite pixels into `NaN`.
///
/// # Examples
///
/// ```
/// use dp2_core::Dimensions;
/// use dp2_depth::resize_depth_bilinear;
///
/// let src = Dimensions::new(2, 1)?;
/// let dst = Dimensions::new(3, 1)?;
/// let resized = resize_depth_bilinear(&[1.0, 3.0], src, dst);
/// assert_eq!(resized.len(), 3);
/// assert!((resized[1] - 2.0).abs() < 1e-5);
/// # Ok::<(), dp2_core::FrameError>(())
/// ```
#[must_use]
pub fn resize_depth_bilinear(depth: &[f32], src: Dimensions, dst: Dimensions) -> Vec<f32> {
    if src == dst {
        return depth.to_vec();
    }

    let src_w = src.width() as usize;
    let src_h = src.height() as usize;
    let dst_w = dst.width() as usize;
    let dst_h = dst.height() as usize;
    let mut out = Vec::with_capacity(dst_w * dst_h);

    for y in 0..dst_h {
        let (y0, y1, fy) = sample_axis(y, src_h, dst_h);
        for x in 0..dst_w {
            let (x0, x1, fx) = sample_axis(x, src_w, dst_w);
            let samples = [
                (depth[y0 * src_w + x0], (1.0 - fx) * (1.0 - fy)),
                (depth[y0 * src_w + x1], fx * (1.0 - fy)),
                (depth[y1 * src_w + x0], (1.0 - fx) * fy),
                (depth[y1 * src_w + x1], fx * fy),
            ];

            let mut weighted_sum = 0.0_f32;
            let mut weight_total = 0.0_f32;
            let mut any_inf = false;

            for (value, weight) in samples {
                if value.is_finite() && value > 0.0 {
                    weighted_sum += value * weight;
                    weight_total += weight;
                } else if value.is_infinite() {
                    any_inf = true;
                }
            }

            if weight_total > 0.0 {
                out.push(weighted_sum / weight_total);
            } else if any_inf {
                out.push(f32::INFINITY);
            } else {
                out.push(0.0);
            }
        }
    }

    out
}

fn sample_axis(dst_index: usize, src_len: usize, dst_len: usize) -> (usize, usize, f32) {
    if src_len <= 1 {
        return (0, 0, 0.0);
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "video frame pixel coordinates are well below 2^24"
    )]
    let coord = ((dst_index as f32) + 0.5) * (src_len as f32) / (dst_len as f32) - 0.5;
    #[expect(
        clippy::cast_precision_loss,
        reason = "video frame dimensions are well below 2^24"
    )]
    let max_coord = (src_len - 1) as f32;
    let clamped = coord.clamp(0.0, max_coord);

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped is guaranteed within [0, src_len - 1]"
    )]
    let i0 = clamped.floor() as usize;
    let i1 = (i0 + 1).min(src_len - 1);
    #[expect(clippy::cast_precision_loss, reason = "i0 is a pixel index below 2^24")]
    let frac = (clamped - (i0 as f32)).clamp(0.0, 1.0);
    (i0, i1, frac)
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use dp2_core::PixelFormat;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::zero_width(0, 60.0)]
    #[case::zero_fov(640, 0.0)]
    #[case::wide_fov(640, 180.0)]
    #[case::nan_fov(640, f32::NAN)]
    fn focal_length_rejects_invalid_fov(#[case] width: u32, #[case] fov: f32) {
        assert_matches!(
            focal_length_from_fov(width, fov),
            Err(DepthError::InvalidFocalLength { .. })
        );
    }

    #[test]
    fn frame_to_nchw_resizes_and_normalises() -> Result<(), Box<dyn std::error::Error>> {
        let frame = VideoFrame::packed(
            Dimensions::new(2, 2)?,
            PixelFormat::Bgra8,
            0,
            0,
            vec![
                0, 0, 255, 255, 0, 0, 255, 255, 0, 255, 0, 255, 0, 255, 0, 255,
            ],
        )?;

        let tensor = frame_to_nchw_f32(&frame, Dimensions::new(1, 1)?);
        assert_eq!(tensor.shape(), &[1, 3, 1, 1]);
        assert!((tensor[[0, 0, 0, 0]] - 0.5).abs() < 1e-5);
        assert!((tensor[[0, 1, 0, 0]] - 0.5).abs() < 1e-5);
        assert!((tensor[[0, 2, 0, 0]] - 0.0).abs() < 1e-5);
        Ok(())
    }

    #[test]
    fn resize_depth_avoids_nan_near_infinite_sky() -> Result<(), Box<dyn std::error::Error>> {
        let src = Dimensions::new(2, 1)?;
        let dst = Dimensions::new(4, 1)?;
        let resized = resize_depth_bilinear(&[2.0, f32::INFINITY], src, dst);
        assert_eq!(resized.len(), 4);
        assert!(resized.iter().all(|v| !v.is_nan()));
        assert!((resized[0] - 2.0).abs() < 1e-5);
        assert!(resized[3].is_infinite() && resized[3].is_sign_positive());
        Ok(())
    }
}
