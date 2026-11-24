// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use tracing::{debug, info};

use crate::{
    DepthError, DepthEstimator, DepthMap, PipelineController, SourceError, SourceInfo, VideoFrame,
    VideoSource,
};

/// [`VideoSource`] decorator that supports runtime source switching and idle
/// waiting (for socket-activated start-up before a camera is selected).
pub struct DynamicVideoSource {
    current: Option<Box<dyn VideoSource>>,
    controller: PipelineController,
}

impl DynamicVideoSource {
    /// Creates a dynamic source with an optional initial [`VideoSource`] wired
    /// to `controller`.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     DynamicVideoSource, EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo,
    ///     VideoSource,
    /// };
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::Passthrough,
    ///     SourceInfo {
    ///         name: "none".into(),
    ///         description: "No source".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "out".into(),
    ///         description: "Output".into(),
    ///     },
    /// )?;
    /// controller.request_shutdown();
    /// let mut src = DynamicVideoSource::new(None, controller);
    /// assert_eq!(src.next_frame()?, None);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn new(initial: Option<Box<dyn VideoSource>>, controller: PipelineController) -> Self {
        Self {
            current: initial,
            controller,
        }
    }
}

impl VideoSource for DynamicVideoSource {
    fn info(&self) -> SourceInfo {
        self.current
            .as_ref()
            .map_or_else(|| self.controller.status().source, |src| src.info())
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, SourceError> {
        loop {
            if self.controller.is_shutdown_requested() {
                return Ok(None);
            }

            if let Some(replacement) = self.controller.take_pending_source() {
                info!(source = %replacement.info().name, "activated switched video source");
                self.current = Some(replacement);
                self.controller.request_depth_reset();
            }

            if let Some(ref mut source) = self.current {
                if let Some(frame) = source.next_frame()? {
                    return Ok(Some(frame));
                }
                if self.controller.is_shutdown_requested() {
                    return Ok(None);
                }
                if self.controller.has_pending_source() {
                    debug!("old video source ended while replacement is queued");
                    self.current = None;
                    continue;
                }
                if !self.controller.stays_alive_when_idle() {
                    return Ok(None);
                }
                debug!("video source reached end-of-stream; waiting for new source");
                self.current = None;
            } else if !self.controller.wait_for_source_or_shutdown() {
                return Ok(None);
            }
        }
    }
}

/// [`DepthEstimator`] decorator that supports runtime model switching through
/// [`PipelineController`].
pub struct DynamicDepthEstimator {
    current: Option<Box<dyn DepthEstimator>>,
    controller: PipelineController,
}

impl DynamicDepthEstimator {
    /// Creates a dynamic depth estimator with an optional initial backend wired
    /// to `controller`.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{
    ///     DepthEstimator, Dimensions, DynamicDepthEstimator, EffectConfig, EffectRegistry,
    ///     PipelineController, PixelFormat, SinkInfo, SourceInfo, VideoFrame,
    ///     testing::FakeDepthEstimator,
    /// };
    ///
    /// let controller = PipelineController::new(
    ///     EffectRegistry::with_builtin_effects(),
    ///     EffectConfig::default(),
    ///     SourceInfo {
    ///         name: "in".into(),
    ///         description: "Input".into(),
    ///     },
    ///     SinkInfo {
    ///         name: "out".into(),
    ///         description: "Output".into(),
    ///     },
    /// )?;
    /// let mut est = DynamicDepthEstimator::new(
    ///     Some(Box::new(FakeDepthEstimator::constant(2.0, 500.0))),
    ///     controller,
    /// );
    /// let frame = VideoFrame::packed(Dimensions::new(1, 1)?, PixelFormat::Rgb8, 0, 1, vec![0; 3])?;
    /// assert_eq!(est.estimate(&frame)?.values_meters(), &[2.0]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn new(initial: Option<Box<dyn DepthEstimator>>, controller: PipelineController) -> Self {
        Self {
            current: initial,
            controller,
        }
    }
}

impl DepthEstimator for DynamicDepthEstimator {
    fn estimate(&mut self, frame: &VideoFrame) -> Result<DepthMap, DepthError> {
        if let Some(replacement) = self.controller.take_pending_estimator() {
            info!("activated switched depth estimator model");
            self.current = Some(replacement);
        }

        let Some(ref mut estimator) = self.current else {
            return Err(DepthError::Inference(
                "no Depth Pro 2 model is currently loaded".into(),
            ));
        };

        estimator.estimate(frame)
    }

    fn reset(&mut self) {
        if let Some(ref mut estimator) = self.current {
            estimator.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{
        Colormap, DepthMapConfig, Dimensions, EffectConfig, EffectRegistry, ModelId, Pipeline,
        PixelFormat, SinkInfo, StepOutcome,
        testing::{FakeClock, FakeModelFactory, FakeSourceFactory, SpyVideoSink},
    };

    #[test]
    fn dynamic_pipeline_switches_source_model_and_captures_preview()
    -> Result<(), Box<dyn std::error::Error>> {
        let dims = Dimensions::new(4, 2)?;
        let frame_a = VideoFrame::packed(dims, PixelFormat::Rgb8, 1, 1, vec![10; 24])?;
        let frame_b = VideoFrame::packed(dims, PixelFormat::Rgb8, 2, 2, vec![90; 24])?;

        let source_factory = Arc::new(FakeSourceFactory::new());
        source_factory.register_frames("cam0", "Camera 0", vec![frame_a]);
        source_factory.register_frames("cam1", "Camera 1", vec![frame_b]);

        let model_factory = Arc::new(FakeModelFactory::new());
        model_factory.register_constant("s", "Depth Pro 2 S", "S (24M)", 1.0);
        model_factory.register_constant("b", "Depth Pro 2 B", "B (97M)", 4.0);

        let controller = PipelineController::new(
            EffectRegistry::with_builtin_effects(),
            EffectConfig::DepthMap(DepthMapConfig {
                colormap: Colormap::Grayscale,
                min_depth_meters: Some(1.0),
                max_depth_meters: Some(4.0),
                invert: false,
            }),
            SourceInfo {
                name: "none".into(),
                description: "No source".into(),
            },
            SinkInfo {
                name: "dp2".into(),
                description: "Depth".into(),
            },
        )?;
        controller.set_source_factory(source_factory);
        controller.set_model_factory(model_factory);

        controller.switch_source(Some("cam0"))?;
        controller.switch_model(&ModelId::new("s"))?;

        let mut pipeline = Pipeline::new(
            DynamicVideoSource::new(None, controller.clone()),
            SpyVideoSink::new("dp2", "Depth"),
            DynamicDepthEstimator::new(None, controller.clone()),
            FakeClock::with_step(Duration::from_millis(5)),
            controller.clone(),
        );

        assert_eq!(pipeline.step()?, StepOutcome::Processed);
        let preview1 = controller
            .preview_frame(2)?
            .expect("preview should exist after first frame");
        assert_eq!(preview1.dimensions, Dimensions::new(2, 1)?);
        assert_eq!(preview1.rgba, vec![255, 255, 255, 255, 255, 255, 255, 255]);

        controller.switch_source(Some("cam1"))?;
        controller.switch_model(&ModelId::new("b"))?;
        assert_eq!(pipeline.step()?, StepOutcome::Processed);

        let preview2 = controller
            .preview_frame(2)?
            .expect("preview should update after second frame");
        assert_eq!(preview2.rgba, vec![0, 0, 0, 255, 0, 0, 0, 255]);
        assert_eq!(controller.status().source.name, "cam1");
        assert_eq!(controller.status().active_model, Some(ModelId::new("b")));
        Ok(())
    }
}
