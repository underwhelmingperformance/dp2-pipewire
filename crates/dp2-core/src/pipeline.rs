// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, info_span};

use crate::{
    Clock, DepthEstimator, Dimensions, EffectConfig, EffectDescriptor, EffectError, EffectRegistry,
    FrameEffect, PipelineError, SinkInfo, SourceInfo, VideoSink, VideoSource,
};

/// Snapshot of the pipeline's runtime configuration and metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineStatus {
    /// Whether the processing loop is currently active.
    pub running: bool,
    /// Input video source metadata.
    pub source: SourceInfo,
    /// Output video sink metadata.
    pub sink: SinkInfo,
    /// Active frame effect configuration.
    pub active_effect: EffectConfig,
    /// Total frames successfully processed and published.
    pub frames_processed: u64,
    /// Most recent depth inference duration in milliseconds.
    pub last_inference_ms: Option<f64>,
    /// Most recent frame effect duration in milliseconds.
    pub last_effect_ms: Option<f64>,
    /// Most recent focal length in pixels reported by the depth estimator.
    pub last_focal_length_px: Option<f32>,
    /// Dimensions of the most recently processed frame.
    pub frame_dimensions: Option<Dimensions>,
}

/// Receives notifications whenever the pipeline's observable status changes.
pub trait StateObserver: Send + Sync {
    /// Called with the updated [`PipelineStatus`] after effect swaps or
    /// lifecycle changes.
    fn on_status_changed(&self, status: &PipelineStatus);
}

struct ActiveEffectState {
    config: EffectConfig,
    effect: Arc<dyn FrameEffect>,
}

struct ControllerInner {
    registry: EffectRegistry,
    active: RwLock<ActiveEffectState>,
    status: RwLock<PipelineStatus>,
    observer: RwLock<Option<Arc<dyn StateObserver>>>,
    reset_depth: AtomicBool,
    shutdown: AtomicBool,
}

/// Thread-safe handle shared between the video pipeline loop and external
/// control planes such as the Varlink service.
#[derive(Clone)]
pub struct PipelineController {
    inner: Arc<ControllerInner>,
}

impl PipelineController {
    /// Creates a controller with `registry` and validates `initial_effect`.
    ///
    /// # Errors
    ///
    /// Returns an [`EffectError`] if `initial_effect` cannot be constructed by
    /// `registry`.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo};
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     SourceInfo {
    ///         name: "cam0".into(),
    ///         description: "Camera".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "dp2".into(),
    ///         description: "Depth".into(),
    ///     },
    /// )?;
    /// assert_eq!(controller.active_effect(), EffectConfig::default());
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    pub fn new(
        registry: EffectRegistry,
        initial_effect: EffectConfig,
        source: SourceInfo,
        sink: SinkInfo,
    ) -> Result<Self, EffectError> {
        let effect: Arc<dyn FrameEffect> = Arc::from(registry.build(&initial_effect)?);
        let status = PipelineStatus {
            running: false,
            source,
            sink,
            active_effect: initial_effect.clone(),
            frames_processed: 0,
            last_inference_ms: None,
            last_effect_ms: None,
            last_focal_length_px: None,
            frame_dimensions: None,
        };

        Ok(Self {
            inner: Arc::new(ControllerInner {
                registry,
                active: RwLock::new(ActiveEffectState {
                    config: initial_effect,
                    effect,
                }),
                status: RwLock::new(status),
                observer: RwLock::new(None),
                reset_depth: AtomicBool::new(false),
                shutdown: AtomicBool::new(false),
            }),
        })
    }

    /// Attaches a [`StateObserver`] that receives updated [`PipelineStatus`]
    /// snapshots.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use dp2_core::{
    ///     EffectConfig, EffectRegistry, PipelineController, PipelineStatus, SinkInfo, SourceInfo,
    ///     StateObserver,
    /// };
    ///
    /// struct NoopObserver;
    /// impl StateObserver for NoopObserver {
    ///     fn on_status_changed(&self, _status: &PipelineStatus) {}
    /// }
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     SourceInfo {
    ///         name: "cam0".into(),
    ///         description: "Camera".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "dp2".into(),
    ///         description: "Depth".into(),
    ///     },
    /// )?;
    /// controller.set_observer(Arc::new(NoopObserver));
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    pub fn set_observer(&self, observer: Arc<dyn StateObserver>) {
        let mut guard = self
            .inner
            .observer
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(observer);
    }

    /// Returns a snapshot of the current pipeline status.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo};
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::Passthrough,
    ///     SourceInfo {
    ///         name: "in".into(),
    ///         description: "In".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "out".into(),
    ///         description: "Out".into(),
    ///     },
    /// )?;
    /// assert_eq!(controller.status().frames_processed, 0);
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    #[must_use]
    pub fn status(&self) -> PipelineStatus {
        self.inner
            .status
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Lists all effects registered in the pipeline's [`EffectRegistry`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo};
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     SourceInfo {
    ///         name: "in".into(),
    ///         description: "In".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "out".into(),
    ///         description: "Out".into(),
    ///     },
    /// )?;
    /// assert_eq!(controller.list_effects().len(), 2);
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    #[must_use]
    pub fn list_effects(&self) -> Vec<EffectDescriptor> {
        self.inner.registry.descriptors()
    }

    /// Returns the currently active [`EffectConfig`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo};
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::Passthrough,
    ///     SourceInfo {
    ///         name: "in".into(),
    ///         description: "In".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "out".into(),
    ///         description: "Out".into(),
    ///     },
    /// )?;
    /// assert_eq!(controller.active_effect(), EffectConfig::Passthrough);
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    #[must_use]
    pub fn active_effect(&self) -> EffectConfig {
        self.inner
            .active
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .config
            .clone()
    }

    /// Validates and atomically replaces the active frame effect at runtime.
    ///
    /// # Errors
    ///
    /// Returns an [`EffectError`] if `config` is unknown or invalid; the
    /// previous active effect remains unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo};
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     SourceInfo {
    ///         name: "in".into(),
    ///         description: "In".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "out".into(),
    ///         description: "Out".into(),
    ///     },
    /// )?;
    /// let applied = controller.set_effect(EffectConfig::Passthrough)?;
    /// assert_eq!(applied, EffectConfig::Passthrough);
    /// assert_eq!(controller.active_effect(), EffectConfig::Passthrough);
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    pub fn set_effect(&self, config: EffectConfig) -> Result<EffectConfig, EffectError> {
        let built: Arc<dyn FrameEffect> = Arc::from(self.inner.registry.build(&config)?);
        {
            let mut active = self
                .inner
                .active
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active.config = config.clone();
            active.effect = built;
        }

        let snapshot = {
            let mut status = self
                .inner
                .status
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            status.active_effect = config.clone();
            status.clone()
        };

        info!(effect = %config.id(), "switched active effect");
        self.notify(&snapshot);
        Ok(config)
    }

    /// Requests that the depth estimator discard its temporal streaming state
    /// before the next inference call.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo};
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     SourceInfo {
    ///         name: "in".into(),
    ///         description: "In".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "out".into(),
    ///         description: "Out".into(),
    ///     },
    /// )?;
    /// controller.request_depth_reset();
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    pub fn request_depth_reset(&self) {
        self.inner.reset_depth.store(true, Ordering::Release);
    }

    /// Requests that the pipeline stop cleanly after the current frame.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo};
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     SourceInfo {
    ///         name: "in".into(),
    ///         description: "In".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "out".into(),
    ///         description: "Out".into(),
    ///     },
    /// )?;
    /// controller.request_shutdown();
    /// assert!(controller.is_shutdown_requested());
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    pub fn request_shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::Release);
    }

    /// Returns `true` once [`PipelineController::request_shutdown`] has been
    /// called.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo};
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     SourceInfo {
    ///         name: "in".into(),
    ///         description: "In".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "out".into(),
    ///         description: "Out".into(),
    ///     },
    /// )?;
    /// assert!(!controller.is_shutdown_requested());
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    #[must_use]
    pub fn is_shutdown_requested(&self) -> bool {
        self.inner.shutdown.load(Ordering::Acquire)
    }

    fn current_effect(&self) -> Arc<dyn FrameEffect> {
        self.inner
            .active
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .effect
            .clone()
    }

    fn take_depth_reset(&self) -> bool {
        self.inner.reset_depth.swap(false, Ordering::AcqRel)
    }

    fn set_running(&self, running: bool) {
        let snapshot = {
            let mut status = self
                .inner
                .status
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            status.running = running;
            status.clone()
        };
        self.notify(&snapshot);
    }

    fn record_frame(
        &self,
        dimensions: Dimensions,
        inference_ms: Option<f64>,
        effect_ms: f64,
        focal_length_px: Option<f32>,
    ) {
        let (snapshot, dimensions_changed) = {
            let mut status = self
                .inner
                .status
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let changed = status.frame_dimensions != Some(dimensions);
            status.frames_processed += 1;
            status.last_inference_ms = inference_ms;
            status.last_effect_ms = Some(effect_ms);
            status.last_focal_length_px = focal_length_px;
            status.frame_dimensions = Some(dimensions);
            (status.clone(), changed)
        };

        if dimensions_changed {
            self.notify(&snapshot);
        }
    }

    fn notify(&self, snapshot: &PipelineStatus) {
        let observer = self
            .inner
            .observer
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(observer) = observer {
            observer.on_status_changed(snapshot);
        }
    }
}

/// Result of processing one step in [`Pipeline::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// One frame was read, processed, and published.
    Processed,
    /// The video source reached end-of-stream.
    EndOfStream,
    /// Shutdown was requested via [`PipelineController`].
    Shutdown,
}

/// Core frame-processing pipeline wired entirely through dependency-injection
/// traits.
pub struct Pipeline<S, K, D, C> {
    source: S,
    sink: K,
    estimator: D,
    clock: C,
    controller: PipelineController,
    depth_was_skipped: bool,
}

impl<S, K, D, C> Pipeline<S, K, D, C>
where
    S: VideoSource,
    K: VideoSink,
    D: DepthEstimator,
    C: Clock,
{
    /// Creates a new pipeline from injected source, sink, depth estimator,
    /// clock, and controller.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     EffectConfig, EffectRegistry, Pipeline, PipelineController, SystemClock,
    ///     testing::{FakeDepthEstimator, FakeVideoSource, SpyVideoSink},
    /// };
    ///
    /// let source = FakeVideoSource::empty("in", "Input");
    /// let sink = SpyVideoSink::new("out", "Output");
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     dp2_core::VideoSource::info(&source),
    ///     dp2_core::VideoSink::info(&sink),
    /// )?;
    /// let mut pipeline = Pipeline::new(
    ///     source,
    ///     sink,
    ///     FakeDepthEstimator::constant(2.0, 500.0),
    ///     SystemClock::new(),
    ///     controller,
    /// );
    /// let status = pipeline.run()?;
    /// assert_eq!(status.frames_processed, 0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub const fn new(
        source: S,
        sink: K,
        estimator: D,
        clock: C,
        controller: PipelineController,
    ) -> Self {
        Self {
            source,
            sink,
            estimator,
            clock,
            controller,
            depth_was_skipped: false,
        }
    }

    /// Returns a clone of the pipeline's [`PipelineController`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     EffectConfig, EffectRegistry, Pipeline, PipelineController, SystemClock,
    ///     testing::{FakeDepthEstimator, FakeVideoSource, SpyVideoSink},
    /// };
    ///
    /// let source = FakeVideoSource::empty("in", "Input");
    /// let sink = SpyVideoSink::new("out", "Output");
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     dp2_core::VideoSource::info(&source),
    ///     dp2_core::VideoSink::info(&sink),
    /// )?;
    /// let pipeline = Pipeline::new(
    ///     source,
    ///     sink,
    ///     FakeDepthEstimator::constant(2.0, 500.0),
    ///     SystemClock::new(),
    ///     controller,
    /// );
    /// assert!(!pipeline.controller().is_shutdown_requested());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn controller(&self) -> PipelineController {
        self.controller.clone()
    }

    /// Consumes the pipeline and returns its injected components for
    /// inspection.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     EffectConfig, EffectRegistry, Pipeline, PipelineController, SystemClock,
    ///     testing::{FakeDepthEstimator, FakeVideoSource, SpyVideoSink},
    /// };
    ///
    /// let source = FakeVideoSource::empty("in", "Input");
    /// let sink = SpyVideoSink::new("out", "Output");
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     dp2_core::VideoSource::info(&source),
    ///     dp2_core::VideoSink::info(&sink),
    /// )?;
    /// let pipeline = Pipeline::new(
    ///     source,
    ///     sink,
    ///     FakeDepthEstimator::constant(2.0, 500.0),
    ///     SystemClock::new(),
    ///     controller,
    /// );
    /// let (_source, sink, _estimator) = pipeline.into_parts();
    /// assert!(sink.frames().is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn into_parts(self) -> (S, K, D) {
        (self.source, self.sink, self.estimator)
    }

    /// Processes a single frame from the source through depth estimation, the
    /// active effect, and the sink.
    ///
    /// # Errors
    ///
    /// Returns a [`PipelineError`] if reading, estimating depth, applying the
    /// effect, or publishing fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     Dimensions, EffectConfig, EffectRegistry, Pipeline, PipelineController, PixelFormat,
    ///     StepOutcome, SystemClock, VideoFrame,
    ///     testing::{FakeDepthEstimator, FakeVideoSource, SpyVideoSink},
    /// };
    ///
    /// let frame = VideoFrame::packed(Dimensions::new(2, 2)?, PixelFormat::Rgb8, 0, 1, vec![0; 12])?;
    /// let source = FakeVideoSource::from_frames("in", "Input", vec![frame]);
    /// let sink = SpyVideoSink::new("out", "Output");
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::Passthrough,
    ///     dp2_core::VideoSource::info(&source),
    ///     dp2_core::VideoSink::info(&sink),
    /// )?;
    /// let mut pipeline = Pipeline::new(
    ///     source,
    ///     sink,
    ///     FakeDepthEstimator::constant(2.0, 500.0),
    ///     SystemClock::new(),
    ///     controller,
    /// );
    /// assert_eq!(pipeline.step()?, StepOutcome::Processed);
    /// assert_eq!(pipeline.step()?, StepOutcome::EndOfStream);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn step(&mut self) -> Result<StepOutcome, PipelineError> {
        if self.controller.is_shutdown_requested() {
            return Ok(StepOutcome::Shutdown);
        }

        let Some(frame) = self.source.next_frame()? else {
            return Ok(StepOutcome::EndOfStream);
        };

        let _span = info_span!(
            "process_frame",
            sequence = frame.sequence(),
            width = frame.dimensions().width(),
            height = frame.dimensions().height()
        )
        .entered();

        let effect = self.controller.current_effect();
        let needs_reset = self.controller.take_depth_reset();

        let (depth, inference_ms, focal_length_px) = if effect.requires_depth() {
            if needs_reset || self.depth_was_skipped {
                debug!("resetting depth estimator temporal state");
                self.estimator.reset();
                self.depth_was_skipped = false;
            }

            let t0 = self.clock.now();
            let depth_map = self.estimator.estimate(&frame)?;
            let elapsed_ms = self.clock.now().saturating_sub(t0).as_secs_f64() * 1_000.0;
            let focal = depth_map.focal_length_px();
            (Some(depth_map), Some(elapsed_ms), Some(focal))
        } else {
            self.depth_was_skipped = true;
            if needs_reset {
                self.estimator.reset();
            }
            (None, None, None)
        };

        let t0 = self.clock.now();
        let output = effect.apply(&frame, depth.as_ref())?;
        let effect_ms = self.clock.now().saturating_sub(t0).as_secs_f64() * 1_000.0;

        self.sink.publish_frame(&output)?;
        self.controller.record_frame(
            output.dimensions(),
            inference_ms,
            effect_ms,
            focal_length_px,
        );

        Ok(StepOutcome::Processed)
    }

    /// Runs [`Pipeline::step`] until the source ends or shutdown is requested.
    ///
    /// # Errors
    ///
    /// Returns the first [`PipelineError`] encountered during processing.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     EffectConfig, EffectRegistry, Pipeline, PipelineController, SystemClock,
    ///     testing::{FakeDepthEstimator, FakeVideoSource, SpyVideoSink},
    /// };
    ///
    /// let source = FakeVideoSource::empty("in", "Input");
    /// let sink = SpyVideoSink::new("out", "Output");
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     dp2_core::VideoSource::info(&source),
    ///     dp2_core::VideoSink::info(&sink),
    /// )?;
    /// let mut pipeline = Pipeline::new(
    ///     source,
    ///     sink,
    ///     FakeDepthEstimator::constant(2.0, 500.0),
    ///     SystemClock::new(),
    ///     controller,
    /// );
    /// let final_status = pipeline.run()?;
    /// assert!(!final_status.running);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn run(&mut self) -> Result<PipelineStatus, PipelineError> {
        self.controller.set_running(true);
        let result = loop {
            match self.step() {
                Ok(StepOutcome::Processed) => {}
                Ok(StepOutcome::EndOfStream | StepOutcome::Shutdown) => {
                    break Ok(());
                }
                Err(err) => break Err(err),
            }
        };
        self.controller.set_running(false);
        result?;
        Ok(self.controller.status())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{
        Colormap, DepthMapConfig, PixelFormat, VideoFrame,
        testing::{FakeClock, FakeDepthEstimator, FakeVideoSource, SpyObserver, SpyVideoSink},
    };

    fn test_frame(seq: u64, rgb: [u8; 3]) -> Result<VideoFrame, Box<dyn std::error::Error>> {
        let dims = Dimensions::new(2, 1)?;
        let mut pixels = Vec::with_capacity(6);
        pixels.extend_from_slice(&rgb);
        pixels.extend_from_slice(&rgb);
        Ok(VideoFrame::packed(
            dims,
            PixelFormat::Rgb8,
            seq * 33_000_000,
            seq,
            pixels,
        )?)
    }

    #[test]
    fn pipeline_runs_depth_and_swaps_effects_at_runtime() -> Result<(), Box<dyn std::error::Error>>
    {
        let f1 = test_frame(1, [10, 20, 30])?;
        let f2 = test_frame(2, [40, 50, 60])?;
        let f3 = test_frame(3, [70, 80, 90])?;

        let source = FakeVideoSource::from_frames("cam0", "Test Camera", vec![f1, f2.clone(), f3]);
        let sink = SpyVideoSink::new("dp2-out", "Depth Camera");
        let estimator = FakeDepthEstimator::horizontal_gradient(1.0, 4.0, 600.0);
        let clock = FakeClock::with_step(Duration::from_millis(5));
        let observer = Arc::new(SpyObserver::new());

        let controller = PipelineController::new(
            EffectRegistry::with_builtin_effects(),
            EffectConfig::DepthMap(DepthMapConfig {
                colormap: Colormap::Grayscale,
                min_depth_meters: Some(1.0),
                max_depth_meters: Some(4.0),
                invert: false,
            }),
            source.info(),
            sink.info(),
        )?;
        controller.set_observer(observer.clone());

        let mut pipeline = Pipeline::new(source, sink, estimator, clock, controller.clone());

        assert_eq!(pipeline.step()?, StepOutcome::Processed);

        controller.set_effect(EffectConfig::Passthrough)?;
        assert_eq!(pipeline.step()?, StepOutcome::Processed);

        controller.set_effect(EffectConfig::DepthMap(DepthMapConfig {
            colormap: Colormap::Grayscale,
            min_depth_meters: Some(1.0),
            max_depth_meters: Some(4.0),
            invert: true,
        }))?;
        assert_eq!(pipeline.step()?, StepOutcome::Processed);
        assert_eq!(pipeline.step()?, StepOutcome::EndOfStream);

        let (_src, sink, estimator) = pipeline.into_parts();
        assert_eq!(estimator.estimate_calls(), 2);
        assert_eq!(estimator.reset_calls(), 1);
        assert_eq!(
            sink.frames(),
            &[
                VideoFrame::packed(
                    Dimensions::new(2, 1)?,
                    PixelFormat::Rgb8,
                    33_000_000,
                    1,
                    vec![255, 255, 255, 0, 0, 0],
                )?,
                f2,
                VideoFrame::packed(
                    Dimensions::new(2, 1)?,
                    PixelFormat::Rgb8,
                    99_000_000,
                    3,
                    vec![0, 0, 0, 255, 255, 255],
                )?,
            ]
        );
        assert_eq!(
            controller.status(),
            PipelineStatus {
                running: false,
                source: SourceInfo {
                    name: "cam0".into(),
                    description: "Test Camera".into(),
                },
                sink: SinkInfo {
                    name: "dp2-out".into(),
                    description: "Depth Camera".into(),
                },
                active_effect: EffectConfig::DepthMap(DepthMapConfig {
                    colormap: Colormap::Grayscale,
                    min_depth_meters: Some(1.0),
                    max_depth_meters: Some(4.0),
                    invert: true,
                }),
                frames_processed: 3,
                last_inference_ms: Some(5.0),
                last_effect_ms: Some(5.0),
                last_focal_length_px: Some(600.0),
                frame_dimensions: Some(Dimensions::new(2, 1)?),
            }
        );
        assert_eq!(observer.snapshots().len(), 3);
        Ok(())
    }

    #[test]
    fn shutdown_stops_before_reading_next_frame() -> Result<(), Box<dyn std::error::Error>> {
        let source =
            FakeVideoSource::from_frames("cam0", "Test Camera", vec![test_frame(1, [1, 2, 3])?]);
        let sink = SpyVideoSink::new("dp2-out", "Depth Camera");
        let controller = PipelineController::new(
            EffectRegistry::with_builtin_effects(),
            EffectConfig::Passthrough,
            source.info(),
            sink.info(),
        )?;
        controller.request_shutdown();

        let mut pipeline = Pipeline::new(
            source,
            sink,
            FakeDepthEstimator::constant(2.0, 500.0),
            FakeClock::with_step(Duration::from_millis(1)),
            controller,
        );
        assert_eq!(pipeline.step()?, StepOutcome::Shutdown);
        let (_src, sink, _est) = pipeline.into_parts();
        assert_eq!(sink.frames(), &[]);
        Ok(())
    }

    #[test]
    fn invalid_effect_swap_preserves_existing_effect() -> Result<(), Box<dyn std::error::Error>> {
        let controller = PipelineController::new(
            EffectRegistry::with_builtin_effects(),
            EffectConfig::Passthrough,
            SourceInfo {
                name: "in".into(),
                description: "In".into(),
            },
            SinkInfo {
                name: "out".into(),
                description: "Out".into(),
            },
        )?;

        let err = controller.set_effect(EffectConfig::DepthMap(DepthMapConfig {
            min_depth_meters: Some(5.0),
            max_depth_meters: Some(1.0),
            ..DepthMapConfig::default()
        }));
        assert_matches!(err, Err(EffectError::Depth(_)));
        assert_eq!(controller.active_effect(), EffectConfig::Passthrough);
        Ok(())
    }
}
