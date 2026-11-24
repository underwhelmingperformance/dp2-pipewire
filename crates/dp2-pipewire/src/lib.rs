// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Linux PipeWire video source and virtual camera sink adapter for `dp2`.

mod config;
mod error;
mod format;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod stub;

pub use config::{PipeWireSinkConfig, PipeWireSourceConfig};
pub use error::PipeWireError;
pub use format::{EncodedBuffer, SpaVideoFormat, decode_spa_frame, encode_spa_frame};
#[cfg(target_os = "linux")]
pub use linux::{PipeWireVideoSink, PipeWireVideoSource};
#[cfg(not(target_os = "linux"))]
pub use stub::{PipeWireVideoSink, PipeWireVideoSource};
