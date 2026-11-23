// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use thiserror::Error;

/// Errors returned when constructing or converting video frames.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FrameError {
    /// Width or height was zero.
    #[error("frame dimensions must be non-zero, got {width}x{height}")]
    ZeroDimension {
        /// Requested frame width in pixels.
        width: u32,
        /// Requested frame height in pixels.
        height: u32,
    },

    /// Row stride is smaller than the minimum packed row size.
    #[error("row stride {stride} is smaller than minimum {min_stride} bytes")]
    StrideTooSmall {
        /// Provided row stride in bytes.
        stride: usize,
        /// Minimum valid stride for the frame width and pixel format.
        min_stride: usize,
    },

    /// Pixel buffer length is smaller than required for the dimensions and
    /// stride.
    #[error("frame buffer length {actual} is smaller than required {expected} bytes")]
    BufferTooSmall {
        /// Actual length of the provided byte buffer.
        actual: usize,
        /// Minimum required byte length.
        expected: usize,
    },
}

/// Errors returned by depth map construction or depth estimation.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DepthError {
    /// Depth value count does not match the frame dimensions.
    #[error("depth map length {actual} does not match expected pixel count {expected}")]
    LengthMismatch {
        /// Actual number of depth samples.
        actual: usize,
        /// Expected pixel count (`width * height`).
        expected: usize,
    },

    /// Focal length must be finite and strictly positive.
    #[error("focal length must be finite and positive, got {focal_length_px}")]
    InvalidFocalLength {
        /// Invalid focal length value in pixels.
        focal_length_px: f32,
    },

    /// Depth range bounds are invalid.
    #[error("depth range [{min_meters}, {max_meters}] must be finite with 0 < min < max")]
    InvalidRange {
        /// Lower bound in metres.
        min_meters: f32,
        /// Upper bound in metres.
        max_meters: f32,
    },

    /// Inference backend failed to estimate depth for a frame.
    #[error("depth estimation failed: {0}")]
    Inference(String),
}

/// Errors returned when reading frames from a video source.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceError {
    /// Source stream disconnected unexpectedly.
    #[error("video source disconnected")]
    Disconnected,

    /// Source emitted a pixel format that is not supported.
    #[error("unsupported source pixel format: {0}")]
    UnsupportedFormat(String),

    /// Source encountered a backend error.
    #[error("video source error: {0}")]
    Backend(String),
}

/// Errors returned when publishing frames to a video sink.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SinkError {
    /// Sink stream disconnected unexpectedly.
    #[error("video sink disconnected")]
    Disconnected,

    /// Sink does not accept the frame pixel format.
    #[error("unsupported sink pixel format: {0}")]
    UnsupportedFormat(String),

    /// Sink encountered a backend error.
    #[error("video sink error: {0}")]
    Backend(String),
}

/// Errors returned when configuring or applying frame effects.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum EffectError {
    /// Effect requires a depth map, but none was supplied.
    #[error("effect requires a depth map")]
    MissingDepthMap,

    /// Input frame dimensions do not match depth map dimensions.
    #[error(
        "depth map dimensions {depth_width}x{depth_height} do not match frame dimensions {frame_width}x{frame_height}"
    )]
    DimensionMismatch {
        /// Width of the input video frame.
        frame_width: u32,
        /// Height of the input video frame.
        frame_height: u32,
        /// Width of the depth map.
        depth_width: u32,
        /// Height of the depth map.
        depth_height: u32,
    },

    /// Effect identifier is not registered in the effect registry.
    #[error("unknown effect: {0}")]
    UnknownEffect(String),

    /// Effect configuration contains an invalid parameter value.
    #[error("invalid effect parameter: {0}")]
    InvalidParameter(String),

    /// Frame construction failed while producing effect output.
    #[error(transparent)]
    Frame(#[from] FrameError),

    /// Depth validation failed while configuring or applying an effect.
    #[error(transparent)]
    Depth(#[from] DepthError),
}

/// Errors returned while running the video processing pipeline.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PipelineError {
    /// Reading from the video source failed.
    #[error(transparent)]
    Source(#[from] SourceError),

    /// Writing to the video sink failed.
    #[error(transparent)]
    Sink(#[from] SinkError),

    /// Depth estimation failed.
    #[error(transparent)]
    Depth(#[from] DepthError),

    /// Applying the active frame effect failed.
    #[error(transparent)]
    Effect(#[from] EffectError),
}
