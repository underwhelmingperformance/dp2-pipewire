// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use dp2_core::{FrameError, SinkError, SourceError};
use thiserror::Error;

/// Errors returned by the PipeWire video source and virtual camera sink
/// adapters.
#[derive(Debug, Error)]
pub enum PipeWireError {
    /// PipeWire runtime support is only available on Linux.
    #[error("PipeWire video streams are only supported on Linux")]
    UnsupportedPlatform,

    /// Connecting to the PipeWire daemon failed.
    #[error("failed to connect to PipeWire: {0}")]
    Connect(String),

    /// Creating or connecting a PipeWire stream failed.
    #[error("PipeWire stream error: {0}")]
    Stream(String),

    /// Serialising or parsing a SPA pod failed.
    #[error("SPA pod error: {0}")]
    Pod(String),

    /// Buffer decoding or encoding encountered a short buffer or unsupported
    /// pixel format.
    #[error("video buffer conversion error: {0}")]
    Buffer(String),

    /// Background PipeWire worker thread stopped unexpectedly.
    #[error("PipeWire worker thread disconnected")]
    WorkerDisconnected,

    /// Frame validation failed.
    #[error(transparent)]
    Frame(#[from] FrameError),
}

impl From<PipeWireError> for SourceError {
    fn from(value: PipeWireError) -> Self {
        match value {
            PipeWireError::WorkerDisconnected => Self::Disconnected,
            other => Self::Backend(other.to_string()),
        }
    }
}

impl From<PipeWireError> for SinkError {
    fn from(value: PipeWireError) -> Self {
        match value {
            PipeWireError::WorkerDisconnected => Self::Disconnected,
            other => Self::Backend(other.to_string()),
        }
    }
}
