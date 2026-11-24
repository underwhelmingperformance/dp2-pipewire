// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use dp2_core::{SinkError, SinkInfo, SourceError, SourceInfo, VideoFrame, VideoSink, VideoSource};

use crate::{PipeWireError, PipeWireSinkConfig, PipeWireSourceConfig};

/// Placeholder PipeWire capture source on non-Linux targets.
#[derive(Debug)]
pub struct PipeWireVideoSource {
    info: SourceInfo,
}

impl PipeWireVideoSource {
    /// Returns [`PipeWireError::UnsupportedPlatform`] on non-Linux targets.
    ///
    /// # Errors
    ///
    /// Always returns [`PipeWireError::UnsupportedPlatform`] when compiled for
    /// a non-Linux operating system.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_pipewire::{PipeWireError, PipeWireSourceConfig, PipeWireVideoSource};
    ///
    /// let result = PipeWireVideoSource::connect(PipeWireSourceConfig::new(None));
    /// assert!(matches!(result, Err(PipeWireError::UnsupportedPlatform)));
    /// ```
    pub fn connect(_config: PipeWireSourceConfig) -> Result<Self, PipeWireError> {
        Err(PipeWireError::UnsupportedPlatform)
    }

    /// Attaches a shutdown predicate (no-op on non-Linux targets).
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_pipewire::{PipeWireError, PipeWireSourceConfig, PipeWireVideoSource};
    ///
    /// let result = PipeWireVideoSource::connect(PipeWireSourceConfig::new(None))
    ///     .map(|src| src.with_shutdown_check(|| true));
    /// assert!(matches!(result, Err(PipeWireError::UnsupportedPlatform)));
    /// ```
    #[must_use]
    pub fn with_shutdown_check(self, _check: impl Fn() -> bool + Send + 'static) -> Self {
        self
    }

    /// Returns [`PipeWireError::UnsupportedPlatform`] on non-Linux targets.
    ///
    /// # Errors
    ///
    /// Always returns an unsupported-platform error on non-Linux targets.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use dp2_pipewire::{PipeWireError, PipeWireSourceConfig, PipeWireVideoSource};
    ///
    /// let result = PipeWireVideoSource::connect(PipeWireSourceConfig::new(None));
    /// assert!(matches!(result, Err(PipeWireError::UnsupportedPlatform)));
    /// ```
    pub fn recv_frame_timeout(
        &mut self,
        _timeout: std::time::Duration,
    ) -> Result<Option<VideoFrame>, SourceError> {
        Err(PipeWireError::UnsupportedPlatform.into())
    }
}

impl VideoSource for PipeWireVideoSource {
    fn info(&self) -> SourceInfo {
        self.info.clone()
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, SourceError> {
        Err(PipeWireError::UnsupportedPlatform.into())
    }
}

/// Placeholder PipeWire virtual camera sink on non-Linux targets.
#[derive(Debug)]
pub struct PipeWireVideoSink {
    info: SinkInfo,
}

impl PipeWireVideoSink {
    /// Returns [`PipeWireError::UnsupportedPlatform`] on non-Linux targets.
    ///
    /// # Errors
    ///
    /// Always returns [`PipeWireError::UnsupportedPlatform`] when compiled for
    /// a non-Linux operating system.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::Dimensions;
    /// use dp2_pipewire::{PipeWireError, PipeWireSinkConfig, PipeWireVideoSink};
    ///
    /// let cfg = PipeWireSinkConfig::new("dp2", "Depth", Dimensions::new(640, 480)?);
    /// let result = PipeWireVideoSink::connect(cfg);
    /// assert!(matches!(result, Err(PipeWireError::UnsupportedPlatform)));
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    pub fn connect(_config: PipeWireSinkConfig) -> Result<Self, PipeWireError> {
        Err(PipeWireError::UnsupportedPlatform)
    }
}

impl VideoSink for PipeWireVideoSink {
    fn info(&self) -> SinkInfo {
        self.info.clone()
    }

    fn publish_frame(&mut self, _frame: &VideoFrame) -> Result<(), SinkError> {
        Err(PipeWireError::UnsupportedPlatform.into())
    }
}
