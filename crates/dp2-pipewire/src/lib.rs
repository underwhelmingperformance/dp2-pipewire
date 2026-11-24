// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Linux PipeWire video source and virtual camera sink adapter for `dp2`.

mod config;
mod error;
mod format;
#[cfg(target_os = "linux")]
mod linux;
mod manager;
#[cfg(not(target_os = "linux"))]
mod stub;
mod video_import;

pub use config::{PipeWireSinkConfig, PipeWireSourceConfig};
pub use error::PipeWireError;
pub use format::{EncodedBuffer, SpaVideoFormat, decode_spa_frame, encode_spa_frame};
#[cfg(target_os = "linux")]
pub use linux::{PipeWireVideoSink, PipeWireVideoSource, list_pipewire_video_sources};
pub use manager::PipeWireSourceManager;
#[cfg(not(target_os = "linux"))]
pub use stub::{PipeWireVideoSink, PipeWireVideoSource, list_pipewire_video_sources};
pub use video_import::{
    DecodedVideo, FfmpegVideoDecoder, LoopingVideoPublisher, VideoFileDecoder,
    slugify_video_node_name,
};
