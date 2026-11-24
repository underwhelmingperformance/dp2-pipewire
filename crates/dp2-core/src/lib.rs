// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Domain types, dependency-injection traits, frame effects, and pipeline
//! orchestration for `dp2`.

mod depth;
mod dynamic;
mod effect;
mod error;
mod frame;
mod pipeline;
mod ports;
mod preview;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use depth::{DepthMap, DepthRange};
pub use dynamic::{DynamicDepthEstimator, DynamicVideoSource};
pub use effect::{
    Colormap, DepthMapConfig, DepthMapEffect, EffectConfig, EffectDescriptor, EffectFactory,
    EffectId, EffectRegistry, FrameEffect, PassthroughEffect,
};
pub use error::{DepthError, EffectError, FrameError, PipelineError, SinkError, SourceError};
pub use frame::{Dimensions, PixelFormat, VideoFrame};
pub use pipeline::{Pipeline, PipelineController, PipelineStatus, StateObserver, StepOutcome};
pub use ports::{
    Clock, DepthEstimator, ModelDescriptor, ModelFactory, ModelId, SinkInfo, SourceFactory,
    SourceInfo, SystemClock, VideoDeviceDescriptor, VideoSink, VideoSource,
};
pub use preview::{PreviewFrame, downsample_preview};
