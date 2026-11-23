// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Domain types, dependency-injection traits, frame effects, and pipeline
//! orchestration for `dp2`.

mod depth;
mod effect;
mod error;
mod frame;
mod pipeline;
mod ports;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use depth::{DepthMap, DepthRange};
pub use effect::{
    Colormap, DepthMapConfig, DepthMapEffect, EffectConfig, EffectDescriptor, EffectFactory,
    EffectId, EffectRegistry, FrameEffect, PassthroughEffect,
};
pub use error::{DepthError, EffectError, FrameError, PipelineError, SinkError, SourceError};
pub use frame::{Dimensions, PixelFormat, VideoFrame};
pub use pipeline::{Pipeline, PipelineController, PipelineStatus, StateObserver, StepOutcome};
pub use ports::{Clock, DepthEstimator, SinkInfo, SourceInfo, SystemClock, VideoSink, VideoSource};
