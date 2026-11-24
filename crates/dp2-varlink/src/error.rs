// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use thiserror::Error;
use zlink::introspect;

/// Typed Varlink method errors returned over the `uk.org.orangesquash.dp2`
/// interface.
#[derive(Debug, Clone, PartialEq, Eq, zlink::ReplyError, introspect::ReplyError)]
#[zlink(interface = "uk.org.orangesquash.dp2")]
pub enum Dp2ServiceError {
    /// Requested effect identifier is not registered with the running pipeline.
    UnknownEffect {
        /// Unrecognised effect identifier.
        id: String,
    },

    /// One or more effect parameters failed validation.
    InvalidParameter {
        /// Explanation of the validation failure.
        message: String,
    },

    /// Discovering, switching, importing, or removing a video source failed.
    SourceFailed {
        /// Explanation of the source failure.
        message: String,
    },

    /// Loading or switching a Depth Pro 2 model failed.
    ModelFailed {
        /// Explanation of the model failure.
        message: String,
    },

    /// Generating a preview frame failed.
    PreviewFailed {
        /// Explanation of the preview failure.
        message: String,
    },
}

impl std::fmt::Display for Dp2ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownEffect { id } => write!(f, "unknown effect: {id}"),
            Self::InvalidParameter { message } => write!(f, "invalid parameter: {message}"),
            Self::SourceFailed { message } => write!(f, "source error: {message}"),
            Self::ModelFailed { message } => write!(f, "model error: {message}"),
            Self::PreviewFailed { message } => write!(f, "preview error: {message}"),
        }
    }
}

impl std::error::Error for Dp2ServiceError {}

/// Transport, protocol, and service errors returned by
/// [`crate::Dp2VarlinkClient`] and [`crate::serve_on_socket`].
#[derive(Debug, Error)]
pub enum VarlinkError {
    /// Underlying Varlink transport or framing error.
    #[error("varlink transport error: {0}")]
    Transport(#[from] zlink::Error),

    /// Filesystem or socket setup error.
    #[error("socket I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Error returned by the remote `uk.org.orangesquash.dp2` service.
    #[error(transparent)]
    Service(#[from] Dp2ServiceError),

    /// Effect configuration could not be converted to or from wire types.
    #[error(transparent)]
    Effect(#[from] dp2_core::EffectError),
}
