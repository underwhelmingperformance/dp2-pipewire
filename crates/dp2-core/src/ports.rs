// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{DepthError, DepthMap, SinkError, SourceError, VideoFrame};

/// Identifying metadata for an input video source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Machine-readable device or node name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
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
