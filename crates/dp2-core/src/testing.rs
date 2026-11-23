// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Deterministic test doubles for `dp2-core` traits.

use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{
    Clock, DepthError, DepthEstimator, DepthMap, PipelineStatus, SinkError, SinkInfo, SourceError,
    SourceInfo, StateObserver, VideoFrame, VideoSink, VideoSource,
};

/// In-memory [`VideoSource`] that yields pre-scripted frames or errors.
#[derive(Debug)]
pub struct FakeVideoSource {
    info: SourceInfo,
    items: VecDeque<Result<VideoFrame, SourceError>>,
}

impl FakeVideoSource {
    /// Creates an empty source that immediately signals end-of-stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{VideoSource, testing::FakeVideoSource};
    ///
    /// let mut source = FakeVideoSource::empty("cam0", "Test Camera");
    /// assert_eq!(source.next_frame()?, None);
    /// # Ok::<(), dp2_core::SourceError>(())
    /// ```
    #[must_use]
    pub fn empty(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            info: SourceInfo {
                name: name.into(),
                description: description.into(),
            },
            items: VecDeque::new(),
        }
    }

    /// Creates a source that yields `frames` in order and then returns
    /// `Ok(None)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame, VideoSource, testing::FakeVideoSource};
    ///
    /// let frame = VideoFrame::packed(Dimensions::new(1, 1)?, PixelFormat::Rgb8, 0, 1, vec![0; 3])?;
    /// let mut source = FakeVideoSource::from_frames("cam0", "Camera", vec![frame.clone()]);
    /// assert_eq!(source.next_frame()?, Some(frame));
    /// assert_eq!(source.next_frame()?, None);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn from_frames(
        name: impl Into<String>,
        description: impl Into<String>,
        frames: Vec<VideoFrame>,
    ) -> Self {
        Self {
            info: SourceInfo {
                name: name.into(),
                description: description.into(),
            },
            items: frames.into_iter().map(Ok).collect(),
        }
    }

    /// Appends an error to the source queue.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{SourceError, VideoSource, testing::FakeVideoSource};
    ///
    /// let mut source = FakeVideoSource::empty("cam0", "Camera");
    /// source.push_error(SourceError::Disconnected);
    /// assert_eq!(source.next_frame(), Err(SourceError::Disconnected));
    /// ```
    pub fn push_error(&mut self, error: SourceError) {
        self.items.push_back(Err(error));
    }
}

impl VideoSource for FakeVideoSource {
    fn info(&self) -> SourceInfo {
        self.info.clone()
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, SourceError> {
        match self.items.pop_front() {
            Some(Ok(frame)) => Ok(Some(frame)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }
}

/// In-memory [`VideoSink`] that records every published frame.
#[derive(Debug)]
pub struct SpyVideoSink {
    info: SinkInfo,
    frames: Vec<VideoFrame>,
    fail_with: Option<SinkError>,
}

impl SpyVideoSink {
    /// Creates an empty recording sink.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::testing::SpyVideoSink;
    ///
    /// let sink = SpyVideoSink::new("dp2", "Depth Camera");
    /// assert!(sink.frames().is_empty());
    /// ```
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            info: SinkInfo {
                name: name.into(),
                description: description.into(),
            },
            frames: Vec::new(),
            fail_with: None,
        }
    }

    /// Configures the sink to fail on subsequent publish calls.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     Dimensions, PixelFormat, SinkError, VideoFrame, VideoSink, testing::SpyVideoSink,
    /// };
    ///
    /// let mut sink = SpyVideoSink::new("dp2", "Depth Camera");
    /// sink.set_error(SinkError::Disconnected);
    /// let frame = VideoFrame::packed(Dimensions::new(1, 1)?, PixelFormat::Rgb8, 0, 1, vec![0; 3])?;
    /// assert_eq!(sink.publish_frame(&frame), Err(SinkError::Disconnected));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_error(&mut self, error: SinkError) {
        self.fail_with = Some(error);
    }

    /// Returns all frames published to this sink in arrival order.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame, VideoSink, testing::SpyVideoSink};
    ///
    /// let mut sink = SpyVideoSink::new("dp2", "Depth Camera");
    /// let frame = VideoFrame::packed(
    ///     Dimensions::new(1, 1)?,
    ///     PixelFormat::Rgb8,
    ///     0,
    ///     1,
    ///     vec![1, 2, 3],
    /// )?;
    /// sink.publish_frame(&frame)?;
    /// assert_eq!(sink.frames(), &[frame]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn frames(&self) -> &[VideoFrame] {
        &self.frames
    }
}

impl VideoSink for SpyVideoSink {
    fn info(&self) -> SinkInfo {
        self.info.clone()
    }

    fn publish_frame(&mut self, frame: &VideoFrame) -> Result<(), SinkError> {
        if let Some(ref err) = self.fail_with {
            return Err(err.clone());
        }
        self.frames.push(frame.clone());
        Ok(())
    }
}

#[derive(Debug, Clone)]
enum DepthPattern {
    Constant(f32),
    HorizontalGradient { left_meters: f32, right_meters: f32 },
    Error(DepthError),
}

/// Deterministic [`DepthEstimator`] fake that records `estimate` and `reset`
/// counts.
#[derive(Debug, Clone)]
pub struct FakeDepthEstimator {
    pattern: DepthPattern,
    focal_length_px: f32,
    estimate_calls: usize,
    reset_calls: usize,
}

impl FakeDepthEstimator {
    /// Creates an estimator that returns a uniform metric depth for every
    /// pixel.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     DepthEstimator, Dimensions, PixelFormat, VideoFrame, testing::FakeDepthEstimator,
    /// };
    ///
    /// let mut est = FakeDepthEstimator::constant(2.5, 500.0);
    /// let frame = VideoFrame::packed(Dimensions::new(2, 1)?, PixelFormat::Rgb8, 0, 0, vec![0; 6])?;
    /// let depth = est.estimate(&frame)?;
    /// assert_eq!(depth.values_meters(), &[2.5, 2.5]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub const fn constant(depth_meters: f32, focal_length_px: f32) -> Self {
        Self {
            pattern: DepthPattern::Constant(depth_meters),
            focal_length_px,
            estimate_calls: 0,
            reset_calls: 0,
        }
    }

    /// Creates an estimator that linearly interpolates depth from `left_meters`
    /// to `right_meters` across each row.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     DepthEstimator, Dimensions, PixelFormat, VideoFrame, testing::FakeDepthEstimator,
    /// };
    ///
    /// let mut est = FakeDepthEstimator::horizontal_gradient(1.0, 3.0, 500.0);
    /// let frame = VideoFrame::packed(Dimensions::new(3, 1)?, PixelFormat::Rgb8, 0, 0, vec![0; 9])?;
    /// let depth = est.estimate(&frame)?;
    /// assert_eq!(depth.values_meters(), &[1.0, 2.0, 3.0]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub const fn horizontal_gradient(
        left_meters: f32,
        right_meters: f32,
        focal_length_px: f32,
    ) -> Self {
        Self {
            pattern: DepthPattern::HorizontalGradient {
                left_meters,
                right_meters,
            },
            focal_length_px,
            estimate_calls: 0,
            reset_calls: 0,
        }
    }

    /// Creates an estimator that always fails with `error`.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     DepthError, DepthEstimator, Dimensions, PixelFormat, VideoFrame,
    ///     testing::FakeDepthEstimator,
    /// };
    ///
    /// let mut est = FakeDepthEstimator::failing(DepthError::Inference("boom".into()));
    /// let frame = VideoFrame::packed(Dimensions::new(1, 1)?, PixelFormat::Rgb8, 0, 0, vec![0; 3])?;
    /// assert_eq!(
    ///     est.estimate(&frame),
    ///     Err(DepthError::Inference("boom".into()))
    /// );
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub const fn failing(error: DepthError) -> Self {
        Self {
            pattern: DepthPattern::Error(error),
            focal_length_px: 500.0,
            estimate_calls: 0,
            reset_calls: 0,
        }
    }

    /// Returns the number of times [`DepthEstimator::estimate`] was called.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::testing::FakeDepthEstimator;
    ///
    /// let est = FakeDepthEstimator::constant(1.0, 500.0);
    /// assert_eq!(est.estimate_calls(), 0);
    /// ```
    #[must_use]
    pub const fn estimate_calls(&self) -> usize {
        self.estimate_calls
    }

    /// Returns the number of times [`DepthEstimator::reset`] was called.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthEstimator, testing::FakeDepthEstimator};
    ///
    /// let mut est = FakeDepthEstimator::constant(1.0, 500.0);
    /// est.reset();
    /// assert_eq!(est.reset_calls(), 1);
    /// ```
    #[must_use]
    pub const fn reset_calls(&self) -> usize {
        self.reset_calls
    }
}

impl DepthEstimator for FakeDepthEstimator {
    fn estimate(&mut self, frame: &VideoFrame) -> Result<DepthMap, DepthError> {
        self.estimate_calls += 1;
        let dims = frame.dimensions();
        let values = match &self.pattern {
            DepthPattern::Constant(d) => vec![*d; dims.pixel_count()],
            DepthPattern::HorizontalGradient {
                left_meters,
                right_meters,
            } => {
                let w = dims.width() as usize;
                let h = dims.height() as usize;
                let mut out = Vec::with_capacity(w * h);
                for _y in 0..h {
                    for x in 0..w {
                        let t = if w <= 1 {
                            0.0
                        } else {
                            #[expect(
                                clippy::cast_precision_loss,
                                reason = "test frame dimensions are small"
                            )]
                            let frac = (x as f32) / ((w - 1) as f32);
                            frac
                        };
                        out.push(left_meters + (right_meters - left_meters) * t);
                    }
                }
                out
            }
            DepthPattern::Error(err) => return Err(err.clone()),
        };

        DepthMap::new(dims, values, self.focal_length_px)
    }

    fn reset(&mut self) {
        self.reset_calls += 1;
    }
}

/// Deterministic [`Clock`] that advances by a fixed duration on each call to
/// [`Clock::now`].
#[derive(Debug)]
pub struct FakeClock {
    step_nanos: u64,
    current_nanos: AtomicU64,
}

impl FakeClock {
    /// Creates a clock that starts at zero and advances by `step` on each read.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use dp2_core::{Clock, testing::FakeClock};
    ///
    /// let clock = FakeClock::with_step(Duration::from_millis(10));
    /// assert_eq!(clock.now(), Duration::from_millis(0));
    /// assert_eq!(clock.now(), Duration::from_millis(10));
    /// ```
    #[must_use]
    pub fn with_step(step: Duration) -> Self {
        let step_nanos = u64::try_from(step.as_nanos()).unwrap_or(u64::MAX);
        Self {
            step_nanos,
            current_nanos: AtomicU64::new(0),
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        let nanos = self
            .current_nanos
            .fetch_add(self.step_nanos, Ordering::AcqRel);
        Duration::from_nanos(nanos)
    }
}

/// Thread-safe [`StateObserver`] spy that records every status notification.
#[derive(Debug, Default)]
pub struct SpyObserver {
    snapshots: Mutex<Vec<PipelineStatus>>,
}

impl SpyObserver {
    /// Creates an empty status observer spy.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::testing::SpyObserver;
    ///
    /// let observer = SpyObserver::new();
    /// assert!(observer.snapshots().is_empty());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a copy of all status snapshots observed so far.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::testing::SpyObserver;
    ///
    /// let observer = SpyObserver::new();
    /// assert_eq!(observer.snapshots().len(), 0);
    /// ```
    #[must_use]
    pub fn snapshots(&self) -> Vec<PipelineStatus> {
        self.snapshots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl StateObserver for SpyObserver {
    fn on_status_changed(&self, status: &PipelineStatus) {
        self.snapshots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(status.clone());
    }
}
