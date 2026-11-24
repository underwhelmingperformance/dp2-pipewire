// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use thiserror::Error;

/// Top-level error type for the `dp2` CLI and daemon.
#[derive(Debug, Error)]
pub enum AppError {
    /// Invalid frame configuration.
    #[error(transparent)]
    Frame(#[from] dp2_core::FrameError),

    /// Invalid depth configuration or inference error.
    #[error(transparent)]
    Depth(#[from] dp2_core::DepthError),

    /// Invalid effect configuration.
    #[error(transparent)]
    Effect(#[from] dp2_core::EffectError),

    /// Runtime video pipeline failure.
    #[error(transparent)]
    Pipeline(#[from] dp2_core::PipelineError),

    /// PipeWire setup or streaming failure.
    #[error(transparent)]
    PipeWire(#[from] dp2_pipewire::PipeWireError),

    /// Varlink server or client failure.
    #[error(transparent)]
    Varlink(#[from] dp2_varlink::VarlinkError),

    /// JSON formatting failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Standard I/O failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Varlink socket path was not provided and `XDG_RUNTIME_DIR` is unset.
    #[error("no Varlink socket path configured; pass --socket or set XDG_RUNTIME_DIR")]
    MissingSocketPath,

    /// Pipeline worker thread panicked.
    #[error("pipeline worker thread terminated unexpectedly")]
    PipelinePanicked,
}
