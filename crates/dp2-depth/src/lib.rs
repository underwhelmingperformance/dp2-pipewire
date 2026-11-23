// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Depth Pro 2 ONNX Runtime inference adapter implementing
//! [`dp2_core::DepthEstimator`].

mod config;
mod onnx;
mod preprocess;

pub use config::{DepthPro2Config, FocalLengthPolicy, default_ort_dylib_name};
pub use onnx::{GraphSpec, OnnxDepthPro2Estimator, StateDtype, StateTensorSpec};
pub use preprocess::{
    focal_length_from_fov, frame_to_nchw_f32, resize_depth_bilinear, scale_focal_length,
};
