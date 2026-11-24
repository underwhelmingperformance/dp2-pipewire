// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end integration tests running `dp2` inside a Linux VM against an
//! isolated PipeWire daemon, a video-backed PipeWire source device, a real
//! Depth Pro 2 ONNX model, and the live Varlink control API.

#![cfg(target_os = "linux")]

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use dp2_core::{
    Colormap, DepthMapConfig, Dimensions, EffectConfig, PixelFormat, VideoFrame, VideoSink,
};
use dp2_pipewire::{
    PipeWireSinkConfig, PipeWireSourceConfig, PipeWireVideoSink, PipeWireVideoSource,
};
use dp2_varlink::Dp2VarlinkClient;
use pretty_assertions::assert_eq;

const REMOTE_NAME: &str = "pipewire-dp2-test";
const SOURCE_NODE: &str = "dp2-test-camera";
const SINK_NODE: &str = "dp2-depth-test";
const FRAME_WIDTH: u32 = 640;
const FRAME_HEIGHT: u32 = 480;

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn decode_video_frames(
    video_path: &Path,
    dims: Dimensions,
    max_frames: usize,
) -> Result<Vec<VideoFrame>, Box<dyn std::error::Error>> {
    let scale = format!("scale={}:{}", dims.width(), dims.height());
    let output = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            video_path.to_str().expect("UTF-8 video path"),
            "-frames:v",
            &max_frames.to_string(),
            "-vf",
            &scale,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .output()?;

    assert!(
        output.status.success(),
        "ffmpeg failed to decode {}: {}",
        video_path.display(),
        String::from_utf8_lossy(&output.stderr)
    );

    let frame_bytes = dims.pixel_count() * PixelFormat::Rgba8.bytes_per_pixel();
    assert!(
        output.stdout.len() >= frame_bytes,
        "expected at least one decoded frame ({frame_bytes} bytes), got {}",
        output.stdout.len()
    );

    let mut frames = Vec::new();
    for (idx, chunk) in output.stdout.chunks_exact(frame_bytes).enumerate() {
        let seq = u64::try_from(idx + 1).expect("frame index fits u64");
        frames.push(VideoFrame::packed(
            dims,
            PixelFormat::Rgba8,
            seq * 33_333_333,
            seq,
            chunk.to_vec(),
        )?);
    }

    Ok(frames)
}

fn start_isolated_pipewire(
    runtime_dir: &Path,
) -> Result<(ChildGuard, ChildGuard), Box<dyn std::error::Error>> {
    let pipewire = Command::new("pipewire")
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .env("PIPEWIRE_RUNTIME_DIR", runtime_dir)
        .env("PIPEWIRE_CORE", REMOTE_NAME)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let pipewire = ChildGuard(pipewire);

    let socket = runtime_dir.join(REMOTE_NAME);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket.exists() {
        assert!(
            Instant::now() < deadline,
            "PipeWire socket {} did not appear in time",
            socket.display()
        );
        thread::sleep(Duration::from_millis(50));
    }

    let wireplumber = Command::new("wireplumber")
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .env("PIPEWIRE_RUNTIME_DIR", runtime_dir)
        .env("PIPEWIRE_REMOTE", REMOTE_NAME)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let wireplumber = ChildGuard(wireplumber);

    thread::sleep(Duration::from_millis(600));
    Ok((pipewire, wireplumber))
}

fn wait_for_pipewire_node(runtime_dir: &Path, node_name: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(output) = Command::new("pw-dump")
            .env("XDG_RUNTIME_DIR", runtime_dir)
            .env("PIPEWIRE_RUNTIME_DIR", runtime_dir)
            .env("PIPEWIRE_REMOTE", REMOTE_NAME)
            .output()
            && output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains(node_name)
        {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn is_grayscale_frame(frame: &VideoFrame) -> bool {
    let rgb = frame.to_rgb8();
    let (pixels, _) = rgb.as_chunks::<3>();
    pixels
        .iter()
        .all(|&[r, g, b]| r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1)
}

fn luma_std_dev(frame: &VideoFrame) -> f64 {
    let rgb = frame.to_rgb8();
    let (pixels, _) = rgb.as_chunks::<3>();
    #[expect(
        clippy::cast_precision_loss,
        reason = "frame pixel count fits comfortably in f64 mantissa"
    )]
    let count = pixels.len() as f64;
    let mean: f64 = pixels
        .iter()
        .map(|&[r, g, b]| (f64::from(r) + f64::from(g) + f64::from(b)) / 3.0)
        .sum::<f64>()
        / count;
    let variance: f64 = pixels
        .iter()
        .map(|&[r, g, b]| {
            let luma = (f64::from(r) + f64::from(g) + f64::from(b)) / 3.0;
            (luma - mean).powi(2)
        })
        .sum::<f64>()
        / count;
    variance.sqrt()
}

fn mean_abs_diff(a: &VideoFrame, b: &VideoFrame) -> f64 {
    let a_rgb = a.to_rgb8();
    let b_rgb = b.to_rgb8();
    let len = a_rgb.len().min(b_rgb.len());
    #[expect(
        clippy::cast_precision_loss,
        reason = "byte count fits comfortably in f64 mantissa"
    )]
    let denom = len as f64;
    a_rgb[..len]
        .iter()
        .zip(&b_rgb[..len])
        .map(|(&x, &y)| f64::from(x.abs_diff(y)))
        .sum::<f64>()
        / denom
}

fn spawn_dp2_daemon(
    runtime_dir: &Path,
    model_path: &Path,
    ort_dylib: &str,
    varlink_socket: &Path,
) -> Result<ChildGuard, Box<dyn std::error::Error>> {
    let dp2_bin = env!("CARGO_BIN_EXE_dp2");
    let child = Command::new(dp2_bin)
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .env("PIPEWIRE_RUNTIME_DIR", runtime_dir)
        .env("DP2_LOG", "info")
        .args([
            "run",
            "--source",
            SOURCE_NODE,
            "--sink-name",
            SINK_NODE,
            "--sink-description",
            "Depth Pro 2 Integration Camera",
            "--width",
            &FRAME_WIDTH.to_string(),
            "--height",
            &FRAME_HEIGHT.to_string(),
            "--format",
            "rgba",
            "--pipewire-remote",
            REMOTE_NAME,
            "--model",
            model_path.to_str().expect("UTF-8 model path"),
            "--ort-dylib",
            ort_dylib,
            "--varlink-socket",
            varlink_socket.to_str().expect("UTF-8 varlink socket"),
            "--effect",
            "depth-map",
            "--colormap",
            "turbo",
        ])
        .spawn()?;
    Ok(ChildGuard(child))
}

fn assert_varlink_introspection(varlink_socket: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let introspect = Command::new("varlinkctl")
        .args([
            "introspect",
            varlink_socket.to_str().expect("UTF-8 socket path"),
            dp2_varlink::INTERFACE_NAME,
        ])
        .output()?;
    assert!(
        introspect.status.success(),
        "varlinkctl introspect failed: {}",
        String::from_utf8_lossy(&introspect.stderr)
    );
    let idl = String::from_utf8_lossy(&introspect.stdout);
    assert!(idl.contains("method GetStatus"));
    assert!(idl.contains("method SetEffect"));
    assert!(idl.contains("method WatchStatus"));
    Ok(())
}

async fn verify_depth_and_hot_swap(
    varlink_socket: &Path,
    consumer: &mut PipeWireVideoSource,
    first_source_frame: &VideoFrame,
    dims: Dimensions,
) -> Result<(), Box<dyn std::error::Error>> {
    let depth_turbo_frame = consumer
        .recv_frame_timeout(Duration::from_secs(30))?
        .expect("timed out waiting for Turbo depth map frame from dp2 PipeWire device");
    assert_eq!(depth_turbo_frame.dimensions(), dims);
    assert!(
        luma_std_dev(&depth_turbo_frame) > 10.0,
        "expected non-flat depth map output, got std_dev={}",
        luma_std_dev(&depth_turbo_frame)
    );
    assert!(
        !is_grayscale_frame(&depth_turbo_frame),
        "expected coloured Turbo depth map"
    );
    assert!(
        mean_abs_diff(&depth_turbo_frame, first_source_frame) > 20.0,
        "depth map should differ substantially from raw input RGB frame"
    );

    let mut client = Dp2VarlinkClient::connect(varlink_socket).await?;
    let status = client.get_status().await?;
    assert!(status.running);
    assert_eq!(status.source_name, SOURCE_NODE);
    assert_eq!(status.sink_name, SINK_NODE);
    assert_eq!(status.active_effect.id, "depth-map");
    assert!(status.frames_processed >= 1);
    assert!(status.last_inference_ms.is_some());
    assert!(status.last_focal_length_px.is_some());

    client
        .set_effect(&EffectConfig::DepthMap(DepthMapConfig {
            colormap: Colormap::Grayscale,
            min_depth_meters: None,
            max_depth_meters: None,
            invert: false,
        }))
        .await?;
    client.reset_depth_state().await?;

    let mut saw_grayscale = false;
    let swap_deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < swap_deadline {
        if let Some(frame) = consumer.recv_frame_timeout(Duration::from_secs(5))?
            && is_grayscale_frame(&frame)
            && luma_std_dev(&frame) > 5.0
        {
            saw_grayscale = true;
            break;
        }
    }
    assert!(
        saw_grayscale,
        "did not observe grayscale depth map frame after Varlink hot-swap"
    );

    client.set_effect(&EffectConfig::Passthrough).await?;
    let mut saw_passthrough_colour = false;
    let passthrough_deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < passthrough_deadline {
        if let Some(frame) = consumer.recv_frame_timeout(Duration::from_secs(5))?
            && !is_grayscale_frame(&frame)
        {
            saw_passthrough_colour = true;
            break;
        }
    }
    assert!(
        saw_passthrough_colour,
        "did not observe colour frame after Varlink switch to passthrough"
    );

    client.shutdown().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Linux VM with PipeWire, WirePlumber, ffmpeg, ONNX Runtime, model, and test video"]
async fn pipewire_depth_pro_2_end_to_end_and_varlink_hot_swap()
-> Result<(), Box<dyn std::error::Error>> {
    let model_path = PathBuf::from(
        std::env::var("DP2_MODEL_PATH")
            .expect("DP2_MODEL_PATH must point to the Depth Pro 2 ONNX model"),
    );
    let video_path = PathBuf::from(
        std::env::var("DP2_TEST_VIDEO").expect("DP2_TEST_VIDEO must point to a test video file"),
    );
    let ort_dylib =
        std::env::var("ORT_DYLIB_PATH").expect("ORT_DYLIB_PATH must point to libonnxruntime.so");

    let dims = Dimensions::new(FRAME_WIDTH, FRAME_HEIGHT)?;
    let source_frames = decode_video_frames(&video_path, dims, 24)?;

    let runtime_dir = tempfile::tempdir()?;
    let (_pipewire, _wireplumber) = start_isolated_pipewire(runtime_dir.path())?;

    let varlink_socket = runtime_dir.path().join("uk.org.orangesquash.dp2");
    let remote_full_path = runtime_dir.path().join(REMOTE_NAME);

    let mut producer_cfg =
        PipeWireSinkConfig::new(SOURCE_NODE, "Integration Test Video Camera", dims);
    producer_cfg.remote_name = Some(remote_full_path.to_string_lossy().into_owned());
    let mut test_camera = PipeWireVideoSink::connect(producer_cfg)?;

    let producer_stop = Arc::new(AtomicBool::new(false));
    let producer_stop_bg = producer_stop.clone();
    let producer_frames = source_frames.clone();
    let producer_handle = thread::spawn(move || {
        let mut idx = 0usize;
        while !producer_stop_bg.load(Ordering::Acquire) {
            let frame = &producer_frames[idx % producer_frames.len()];
            let _ = test_camera.publish_frame(frame);
            idx = idx.wrapping_add(1);
            thread::sleep(Duration::from_millis(66));
        }
    });

    assert!(
        wait_for_pipewire_node(runtime_dir.path(), SOURCE_NODE, Duration::from_secs(5)),
        "test source PipeWire device did not register"
    );

    let mut dp2_child =
        spawn_dp2_daemon(runtime_dir.path(), &model_path, &ort_dylib, &varlink_socket)?;

    assert!(
        wait_for_pipewire_node(runtime_dir.path(), SINK_NODE, Duration::from_secs(30)),
        "dp2 virtual camera PipeWire node did not register"
    );

    let sock_deadline = Instant::now() + Duration::from_secs(15);
    while !varlink_socket.exists() {
        assert!(
            Instant::now() < sock_deadline,
            "Varlink socket did not appear"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert_varlink_introspection(&varlink_socket)?;

    let mut consumer_cfg = PipeWireSourceConfig::new(Some(SINK_NODE.into()));
    consumer_cfg.remote_name = Some(remote_full_path.to_string_lossy().into_owned());
    let mut consumer = PipeWireVideoSource::connect(consumer_cfg)?;

    verify_depth_and_hot_swap(&varlink_socket, &mut consumer, &source_frames[0], dims).await?;

    let exit_deadline = Instant::now() + Duration::from_secs(15);
    let exit_status = loop {
        if let Some(status) = dp2_child.0.try_wait()? {
            break status;
        }
        assert!(
            Instant::now() < exit_deadline,
            "dp2 daemon did not exit cleanly after Varlink shutdown"
        );
        thread::sleep(Duration::from_millis(100));
    };
    assert!(
        exit_status.success(),
        "dp2 exited with non-zero status: {exit_status}"
    );

    producer_stop.store(true, Ordering::Release);
    let _ = producer_handle.join();
    Ok(())
}
