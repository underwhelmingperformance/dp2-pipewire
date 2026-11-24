// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::Path,
    time::{Duration, Instant},
};

use derive_more::{Display, From, Into};
use serde::{Deserialize, Serialize};

use crate::{DepthError, DepthMap, SinkError, SourceError, VideoFrame};

/// Stable string identifier for a Depth Pro 2 model variant (for example `"s"`
/// or `"b"`).
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Display, From, Into, Serialize, Deserialize,
)]
pub struct ModelId(String);

impl AsRef<str> for ModelId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl ModelId {
    /// Creates a model identifier from any string-like value.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::ModelId;
    ///
    /// let id = ModelId::new("s-stream4");
    /// assert_eq!(id.as_str(), "s-stream4");
    /// ```
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the identifier as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::ModelId;
    ///
    /// let id = ModelId::new("b");
    /// assert_eq!(id.as_str(), "b");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Static metadata describing a selectable Depth Pro 2 model variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// Machine-readable identifier passed to model-switching APIs.
    pub id: ModelId,
    /// Human-readable display name.
    pub name: String,
    /// Short summary of speed, resolution, and streaming properties.
    pub description: String,
    /// Short size label (for example `"S (24M)"` or `"B (97M)"`).
    pub size_label: String,
    /// Whether the model weights are currently present locally and ready to
    /// load.
    pub available: bool,
}

/// Identifying metadata for an input video source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Machine-readable device or node name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
}

/// Discovered PipeWire video source device or imported looping video device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoDeviceDescriptor {
    /// PipeWire `node.name` used to target this device.
    pub name: String,
    /// Human-readable label shown in UI menus.
    pub description: String,
    /// Whether this device was created by importing a looping video file.
    pub is_imported_video: bool,
    /// Backing video file path when `is_imported_video` is `true`.
    pub video_path: Option<String>,
}

/// Identifying metadata for an output video sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SinkInfo {
    /// Machine-readable device or node name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
}

/// Supplies decoded input video frames to the pipeline.
pub trait VideoSource: Send {
    /// Returns identifying metadata for this video source.
    fn info(&self) -> SourceInfo;

    /// Returns the next available video frame, or `Ok(None)` when the source
    /// reaches end-of-stream.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] if the underlying capture stream fails.
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, SourceError>;
}

/// Publishes processed video frames to downstream consumers.
pub trait VideoSink: Send {
    /// Returns identifying metadata for this video sink.
    fn info(&self) -> SinkInfo;

    /// Publishes a single processed frame.
    ///
    /// # Errors
    ///
    /// Returns a [`SinkError`] if the underlying output stream fails.
    fn publish_frame(&mut self, frame: &VideoFrame) -> Result<(), SinkError>;
}

/// Estimates metric depth from decoded video frames.
pub trait DepthEstimator: Send {
    /// Estimates a [`DepthMap`] matching the dimensions of `frame`.
    ///
    /// # Errors
    ///
    /// Returns a [`DepthError`] if inference fails.
    fn estimate(&mut self, frame: &VideoFrame) -> Result<DepthMap, DepthError>;

    /// Resets any temporal state kept between consecutive video frames.
    fn reset(&mut self);
}

/// Discovers, opens, and imports video sources at runtime.
pub trait SourceFactory: Send + Sync {
    /// Lists currently available camera devices and imported looping videos.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] if device discovery fails.
    fn list_sources(&self) -> Result<Vec<VideoDeviceDescriptor>, SourceError>;

    /// Opens a [`VideoSource`] targeting `target` (or the default camera when
    /// `None`).
    ///
    /// `should_stop` is polled while waiting for frames so switching sources or
    /// shutting down interrupts a quiet source promptly.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] if the source cannot be opened.
    fn open_source(
        &self,
        target: Option<&str>,
        should_stop: Box<dyn Fn() -> bool + Send>,
    ) -> Result<Box<dyn VideoSource>, SourceError>;

    /// Imports a video file and exposes it as a looping PipeWire video device.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] if the video file cannot be decoded or
    /// published.
    fn import_video(&self, path: &Path) -> Result<VideoDeviceDescriptor, SourceError>;

    /// Stops and removes a previously imported looping video device by name.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] if no imported video device with `name` exists.
    fn remove_video(&self, name: &str) -> Result<(), SourceError>;
}

/// Lists and instantiates Depth Pro 2 estimator backends at runtime.
pub trait ModelFactory: Send + Sync {
    /// Lists all configured model variants and whether each is available.
    fn list_models(&self) -> Vec<ModelDescriptor>;

    /// Instantiates a [`DepthEstimator`] for the model identified by `id`.
    ///
    /// # Errors
    ///
    /// Returns [`DepthError`] if `id` is unknown or cannot be loaded.
    fn load_model(&self, id: &ModelId) -> Result<Box<dyn DepthEstimator>, DepthError>;
}

/// Monotonic clock abstraction for deterministic latency measurement in unit
/// tests.
pub trait Clock: Send + Sync {
    /// Returns elapsed monotonic time since an arbitrary fixed epoch.
    fn now(&self) -> Duration;
}

/// Production [`Clock`] backed by [`std::time::Instant`].
#[derive(Debug, Clone)]
pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemClock {
    /// Creates a new monotonic clock anchored at the current instant.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Clock, SystemClock};
    ///
    /// let clock = SystemClock::new();
    /// let t0 = clock.now();
    /// let t1 = clock.now();
    /// assert!(t1 >= t0);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}
