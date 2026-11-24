// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use dp2_core::{Dimensions, PixelFormat, VideoFrame, VideoSink};
use tracing::debug;

use crate::PipeWireError;

/// Decoded RGBA8 frames ready to loop into a virtual PipeWire camera device.
#[derive(Debug, Clone)]
pub struct DecodedVideo {
    /// Decoded frame dimensions.
    pub dimensions: Dimensions,
    /// Target playback interval between consecutive frames.
    pub frame_interval: Duration,
    /// Decoded frames in display order.
    pub frames: Vec<VideoFrame>,
}

/// Decodes a video file into RGBA8 frames for looping PipeWire publication.
pub trait VideoFileDecoder: Send + Sync {
    /// Decodes up to `max_frames` frames from `path`, scaled to `target_dims`.
    ///
    /// # Errors
    ///
    /// Returns [`PipeWireError`] if the file cannot be read or decoded.
    fn decode(
        &self,
        path: &Path,
        target_dims: Dimensions,
        max_frames: usize,
    ) -> Result<DecodedVideo, PipeWireError>;
}

/// [`VideoFileDecoder`] backed by the `ffmpeg` command-line tool.
#[derive(Debug, Clone)]
pub struct FfmpegVideoDecoder {
    ffmpeg_bin: String,
    framerate_fps: u32,
}

impl Default for FfmpegVideoDecoder {
    fn default() -> Self {
        Self::new("ffmpeg", 15)
    }
}

impl FfmpegVideoDecoder {
    /// Creates an `FFmpeg` decoder invoking `ffmpeg_bin` at `framerate_fps`.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_pipewire::FfmpegVideoDecoder;
    ///
    /// let _decoder = FfmpegVideoDecoder::new("ffmpeg", 15);
    /// ```
    #[must_use]
    pub fn new(ffmpeg_bin: impl Into<String>, framerate_fps: u32) -> Self {
        Self {
            ffmpeg_bin: ffmpeg_bin.into(),
            framerate_fps: framerate_fps.max(1),
        }
    }
}

impl VideoFileDecoder for FfmpegVideoDecoder {
    fn decode(
        &self,
        path: &Path,
        target_dims: Dimensions,
        max_frames: usize,
    ) -> Result<DecodedVideo, PipeWireError> {
        let Some(path_str) = path.to_str() else {
            return Err(PipeWireError::Buffer(
                "video path must be valid UTF-8".into(),
            ));
        };

        let vf = format!(
            "fps={},scale={}:{}",
            self.framerate_fps,
            target_dims.width(),
            target_dims.height()
        );
        let output = Command::new(&self.ffmpeg_bin)
            .args([
                "-v",
                "error",
                "-i",
                path_str,
                "-frames:v",
                &max_frames.max(1).to_string(),
                "-vf",
                &vf,
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-",
            ])
            .output()
            .map_err(|e| PipeWireError::Buffer(format!("failed to invoke ffmpeg: {e}")))?;

        if !output.status.success() {
            return Err(PipeWireError::Buffer(format!(
                "ffmpeg failed decoding '{}': {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let frame_bytes = target_dims.pixel_count() * PixelFormat::Rgba8.bytes_per_pixel();
        if output.stdout.len() < frame_bytes {
            return Err(PipeWireError::Buffer(format!(
                "video '{}' produced no full frames",
                path.display()
            )));
        }

        let interval_ns = 1_000_000_000u64 / u64::from(self.framerate_fps);
        let mut frames = Vec::new();
        for (idx, chunk) in output.stdout.chunks_exact(frame_bytes).enumerate() {
            let seq = u64::try_from(idx + 1).unwrap_or(u64::MAX);
            frames.push(VideoFrame::packed(
                target_dims,
                PixelFormat::Rgba8,
                seq.saturating_mul(interval_ns),
                seq,
                chunk.to_vec(),
            )?);
        }

        Ok(DecodedVideo {
            dimensions: target_dims,
            frame_interval: Duration::from_nanos(interval_ns),
            frames,
        })
    }
}

/// Converts a video file path into a deterministic PipeWire `node.name` slug.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use dp2_pipewire::slugify_video_node_name;
///
/// assert_eq!(
///     slugify_video_node_name(Path::new("/tmp/My Flower (1).webm")),
///     "dp2-video-my-flower-1"
/// );
/// ```
#[must_use]
pub fn slugify_video_node_name(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("clip");

    let mut slug = String::from("dp2-video-");
    let mut prev_dash = true;
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug == "dp2-video" {
        slug.push_str("-clip");
    }
    slug
}

/// Background thread handle that continuously loops a [`DecodedVideo`] into a
/// [`VideoSink`] until dropped.
pub struct LoopingVideoPublisher {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LoopingVideoPublisher {
    /// Starts looping `video.frames` into `sink` on a background thread.
    ///
    /// # Errors
    ///
    /// Returns [`PipeWireError::Buffer`] if `video.frames` is empty or spawning
    /// the background thread fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame, testing::SpyVideoSink};
    /// use dp2_pipewire::{DecodedVideo, LoopingVideoPublisher};
    ///
    /// let frame = VideoFrame::packed(
    ///     Dimensions::new(2, 2)?,
    ///     PixelFormat::Rgba8,
    ///     0,
    ///     1,
    ///     vec![0; 16],
    /// )?;
    /// let video = DecodedVideo {
    ///     dimensions: Dimensions::new(2, 2)?,
    ///     frame_interval: Duration::from_millis(5),
    ///     frames: vec![frame],
    /// };
    /// let publisher = LoopingVideoPublisher::start(video, Box::new(SpyVideoSink::new("v", "V")))?;
    /// drop(publisher);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn start(video: DecodedVideo, mut sink: Box<dyn VideoSink>) -> Result<Self, PipeWireError> {
        if video.frames.is_empty() {
            return Err(PipeWireError::Buffer(
                "cannot publish a looping video with zero frames".into(),
            ));
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = stop.clone();
        let interval = video.frame_interval.max(Duration::from_millis(1));
        let frames = video.frames;

        let worker = thread::Builder::new()
            .name("dp2-video-loop".into())
            .spawn(move || {
                let mut idx = 0usize;
                while !stop_bg.load(Ordering::Acquire) {
                    let frame = &frames[idx % frames.len()];
                    if sink.publish_frame(frame).is_err() {
                        debug!("looping video sink disconnected");
                        break;
                    }
                    idx = idx.wrapping_add(1);
                    let mut remaining = interval;
                    while !remaining.is_zero() && !stop_bg.load(Ordering::Acquire) {
                        let slice = remaining.min(Duration::from_millis(20));
                        thread::sleep(slice);
                        remaining = remaining.saturating_sub(slice);
                    }
                }
            })
            .map_err(|e| {
                PipeWireError::Stream(format!("failed to spawn video loop thread: {e}"))
            })?;

        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for LoopingVideoPublisher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use dp2_core::{SinkError, SinkInfo};
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::simple("/tmp/flower.webm", "dp2-video-flower")]
    #[case::punctuation("/home/me/My Clip (1)!.mp4", "dp2-video-my-clip-1")]
    #[case::symbols("/tmp/---.mkv", "dp2-video-clip")]
    fn slugifies_video_paths(#[case] path: &str, #[case] expected: &str) {
        assert_eq!(slugify_video_node_name(Path::new(path)), expected);
    }

    struct CountingSink {
        count: Arc<Mutex<usize>>,
    }

    impl VideoSink for CountingSink {
        fn info(&self) -> SinkInfo {
            SinkInfo {
                name: "test".into(),
                description: "Test".into(),
            }
        }

        fn publish_frame(&mut self, _frame: &VideoFrame) -> Result<(), SinkError> {
            *self
                .count
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            Ok(())
        }
    }

    #[test]
    fn looping_publisher_publishes_multiple_cycles_and_stops_on_drop()
    -> Result<(), Box<dyn std::error::Error>> {
        let dims = Dimensions::new(2, 2)?;
        let f1 = VideoFrame::packed(dims, PixelFormat::Rgba8, 0, 1, vec![1; 16])?;
        let f2 = VideoFrame::packed(dims, PixelFormat::Rgba8, 0, 2, vec![2; 16])?;
        let count = Arc::new(Mutex::new(0usize));

        let publisher = LoopingVideoPublisher::start(
            DecodedVideo {
                dimensions: dims,
                frame_interval: Duration::from_millis(5),
                frames: vec![f1, f2],
            },
            Box::new(CountingSink {
                count: count.clone(),
            }),
        )?;

        thread::sleep(Duration::from_millis(40));
        drop(publisher);

        let total_frames = *count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            total_frames >= 3,
            "expected looped playback, got {total_frames} frames"
        );
        Ok(())
    }
}
