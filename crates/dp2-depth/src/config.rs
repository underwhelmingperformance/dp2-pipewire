// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;

/// How horizontal focal length in pixels is determined for incoming video
/// frames.
#[derive(Debug, Clone, PartialEq)]
pub enum FocalLengthPolicy {
    /// Use a calibrated horizontal focal length in pixels for the input camera
    /// resolution.
    FixedPx(f32),
    /// Derive focal length from an assumed horizontal field of view in degrees.
    HorizontalFovDegrees(f32),
    /// Estimate focal length from the first frame using a Depth Pro 2 focal
    /// ONNX model.
    EstimateWithModel(PathBuf),
}

impl Default for FocalLengthPolicy {
    fn default() -> Self {
        Self::HorizontalFovDegrees(60.0)
    }
}

/// Configuration for [`crate::OnnxDepthPro2Estimator`].
#[derive(Debug, Clone, PartialEq)]
pub struct DepthPro2Config {
    /// Path to the Depth Pro 2 ONNX model file (single-frame, streaming, or
    /// auto-focal).
    pub model_path: PathBuf,
    /// Path to the ONNX Runtime shared library (`libonnxruntime.so` /
    /// `libonnxruntime.dylib`).
    ///
    /// When `None`, the platform default shared library name is loaded through
    /// the dynamic linker search path.
    pub ort_dylib_path: Option<PathBuf>,
    /// Policy for obtaining camera focal length when the depth graph expects
    /// `f_px`.
    pub focal_length: FocalLengthPolicy,
    /// Optional intra-op thread count passed to ONNX Runtime.
    pub intra_threads: Option<usize>,
}

impl DepthPro2Config {
    /// Creates a configuration pointing at `model_path` with default
    /// focal-length and thread settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    ///
    /// use dp2_depth::{DepthPro2Config, FocalLengthPolicy};
    ///
    /// let cfg = DepthPro2Config::new("models/model_stream4.onnx");
    /// assert_eq!(cfg.model_path, PathBuf::from("models/model_stream4.onnx"));
    /// assert_eq!(
    ///     cfg.focal_length,
    ///     FocalLengthPolicy::HorizontalFovDegrees(60.0)
    /// );
    /// ```
    #[must_use]
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            ort_dylib_path: None,
            focal_length: FocalLengthPolicy::default(),
            intra_threads: None,
        }
    }
}

/// Returns the default ONNX Runtime shared library filename for the target
/// operating system.
///
/// # Examples
///
/// ```
/// use dp2_depth::default_ort_dylib_name;
///
/// let name = default_ort_dylib_name();
/// assert!(name.contains("onnxruntime"));
/// ```
#[must_use]
pub const fn default_ort_dylib_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libonnxruntime.dylib"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "libonnxruntime.so"
    }
}
