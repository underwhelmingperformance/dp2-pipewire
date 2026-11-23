// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Varlink control plane (`uk.org.orangesquash.dp2`) for inspecting and
//! modifying a running `dp2` pipeline.

mod client;
mod error;
mod service;
mod types;

pub use client::Dp2VarlinkClient;
pub use error::{Dp2ServiceError, VarlinkError};
pub use service::{INTERFACE_NAME, serve_on_socket};
pub use types::{
    AckReply, DimensionsInfo, EffectInfo, EffectReply, EffectSettings, EffectsReply, StatusReply,
    StatusSnapshot,
};
