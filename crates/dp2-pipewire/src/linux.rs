// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

use crossbeam_channel::{Receiver, Sender, bounded};
use dp2_core::{
    Dimensions, PixelFormat, SinkError, SinkInfo, SourceError, SourceInfo, VideoDeviceDescriptor,
    VideoFrame, VideoSink, VideoSource,
};
use pipewire as pw;
use pw::{properties::PropertiesBox, spa};
use spa::{
    param::{
        ParamType,
        format::{FormatProperties, MediaSubtype, MediaType},
        format_utils::parse_format,
        video::{VideoFormat, VideoInfoRaw},
    },
    pod::{Object, Pod, Property, PropertyFlags, Value, serialize::PodSerializer},
    utils::{Direction, Fraction, Rectangle, SpaTypes},
};
use tracing::{debug, info, warn};

use crate::{
    PipeWireError, PipeWireSinkConfig, PipeWireSourceConfig, SpaVideoFormat, decode_spa_frame,
    encode_spa_frame,
};

enum SourceCmd {
    Quit,
}

enum SinkCmd {
    Publish(VideoFrame),
    Quit,
}

/// PipeWire video capture adapter implementing [`VideoSource`].
pub struct PipeWireVideoSource {
    info: SourceInfo,
    frames_rx: Receiver<Result<VideoFrame, PipeWireError>>,
    cmd_tx: pw::channel::Sender<SourceCmd>,
    shutdown_check: Option<Box<dyn Fn() -> bool + Send>>,
    worker: Option<JoinHandle<()>>,
}

impl PipeWireVideoSource {
    /// Connects to PipeWire on a dedicated main-loop thread and captures raw
    /// video frames from the configured target node.
    ///
    /// # Errors
    ///
    /// Returns [`PipeWireError`] if PipeWire initialisation, connection, or
    /// stream setup fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_pipewire::{PipeWireSourceConfig, PipeWireVideoSource};
    ///
    /// let _source = PipeWireVideoSource::connect(PipeWireSourceConfig::new(None))?;
    /// # Ok::<(), dp2_pipewire::PipeWireError>(())
    /// ```
    pub fn connect(config: PipeWireSourceConfig) -> Result<Self, PipeWireError> {
        let info = SourceInfo {
            name: config
                .target
                .clone()
                .unwrap_or_else(|| config.stream_name.clone()),
            description: config.target.clone().map_or_else(
                || "Default PipeWire Camera".into(),
                |t| format!("PipeWire source {t}"),
            ),
        };

        let capacity = config.queue_capacity.max(1);
        let (frames_tx, frames_rx) = bounded::<Result<VideoFrame, PipeWireError>>(capacity);
        let frames_rx_evict = frames_rx.clone();
        let (ready_tx, ready_rx) = bounded::<Result<(), PipeWireError>>(1);
        let (cmd_tx, cmd_rx) = pw::channel::channel::<SourceCmd>();

        let worker = thread::Builder::new()
            .name("dp2-pw-source".into())
            .spawn(move || {
                run_source_loop(&config, frames_tx, frames_rx_evict, &ready_tx, cmd_rx);
            })
            .map_err(|e| PipeWireError::Stream(format!("failed to spawn PipeWire thread: {e}")))?;

        ready_rx
            .recv()
            .map_err(|_| PipeWireError::WorkerDisconnected)??;

        Ok(Self {
            info,
            frames_rx,
            cmd_tx,
            shutdown_check: None,
            worker: Some(worker),
        })
    }

    /// Attaches a shutdown predicate polled while waiting for incoming frames
    /// so [`VideoSource::next_frame`] returns `Ok(None)` promptly on
    /// shutdown.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_pipewire::{PipeWireSourceConfig, PipeWireVideoSource};
    ///
    /// let _source = PipeWireVideoSource::connect(PipeWireSourceConfig::new(None))?
    ///     .with_shutdown_check(|| false);
    /// # Ok::<(), dp2_pipewire::PipeWireError>(())
    /// ```
    #[must_use]
    pub fn with_shutdown_check(mut self, check: impl Fn() -> bool + Send + 'static) -> Self {
        self.shutdown_check = Some(Box::new(check));
        self
    }

    /// Attempts to receive the next frame within `timeout`, returning
    /// `Ok(None)` if no frame arrives before the deadline or the stream
    /// disconnects.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] if frame decoding failed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    ///
    /// use dp2_pipewire::{PipeWireSourceConfig, PipeWireVideoSource};
    ///
    /// let mut source = PipeWireVideoSource::connect(PipeWireSourceConfig::new(None))?;
    /// let _frame = source.recv_frame_timeout(Duration::from_secs(1))?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn recv_frame_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<VideoFrame>, SourceError> {
        match self.frames_rx.recv_timeout(timeout) {
            Ok(Ok(frame)) => Ok(Some(frame)),
            Ok(Err(err)) => Err(err.into()),
            Err(
                crossbeam_channel::RecvTimeoutError::Timeout
                | crossbeam_channel::RecvTimeoutError::Disconnected,
            ) => Ok(None),
        }
    }
}

impl VideoSource for PipeWireVideoSource {
    fn info(&self) -> SourceInfo {
        self.info.clone()
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, SourceError> {
        loop {
            if self.shutdown_check.as_ref().is_some_and(|check| check()) {
                return Ok(None);
            }
            match self
                .frames_rx
                .recv_timeout(std::time::Duration::from_millis(50))
            {
                Ok(Ok(frame)) => return Ok(Some(frame)),
                Ok(Err(err)) => return Err(err.into()),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return Ok(None),
            }
        }
    }
}

impl Drop for PipeWireVideoSource {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(SourceCmd::Quit);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

struct CaptureUserData {
    format: VideoInfoRaw,
    format_ready: bool,
    sequence: u64,
    frames_tx: Sender<Result<VideoFrame, PipeWireError>>,
    frames_rx_drop: Receiver<Result<VideoFrame, PipeWireError>>,
}

fn run_source_loop(
    config: &PipeWireSourceConfig,
    frames_tx: Sender<Result<VideoFrame, PipeWireError>>,
    frames_rx_evict: Receiver<Result<VideoFrame, PipeWireError>>,
    ready_tx: &Sender<Result<(), PipeWireError>>,
    cmd_rx: pw::channel::Receiver<SourceCmd>,
) {
    if let Err(err) = run_source_loop_inner(config, frames_tx, frames_rx_evict, ready_tx, cmd_rx) {
        let _ = ready_tx.send(Err(err));
    }
}

fn run_source_loop_inner(
    config: &PipeWireSourceConfig,
    frames_tx: Sender<Result<VideoFrame, PipeWireError>>,
    frames_rx_evict: Receiver<Result<VideoFrame, PipeWireError>>,
    ready_tx: &Sender<Result<(), PipeWireError>>,
    cmd_rx: pw::channel::Receiver<SourceCmd>,
) -> Result<(), PipeWireError> {
    pw::init();

    let mainloop =
        pw::main_loop::MainLoopRc::new(None).map_err(|e| PipeWireError::Connect(e.to_string()))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|e| PipeWireError::Connect(e.to_string()))?;
    let core = connect_core(&context, config.remote_name.as_deref())?;

    let mut props = PropertiesBox::new();
    props.insert(*pw::keys::MEDIA_TYPE, "Video");
    props.insert(*pw::keys::MEDIA_CATEGORY, "Capture");
    props.insert(*pw::keys::MEDIA_ROLE, "Camera");
    if let Some(ref target) = config.target {
        props.insert(*pw::keys::TARGET_OBJECT, target.as_str());
    }

    let stream = pw::stream::StreamRc::new(core.clone(), &config.stream_name, props)
        .map_err(|e| PipeWireError::Stream(e.to_string()))?;

    let user_data = CaptureUserData {
        format: VideoInfoRaw::default(),
        format_ready: false,
        sequence: 0,
        frames_tx,
        frames_rx_drop: frames_rx_evict,
    };

    let _listener = stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(|_, _, old, new| {
            debug!(?old, ?new, "PipeWire capture stream state changed");
        })
        .param_changed(|_, data, id, param| handle_capture_param_changed(data, id, param))
        .process(handle_capture_process)
        .register()
        .map_err(|e| PipeWireError::Stream(e.to_string()))?;

    let pod_bytes = build_capture_format_pod()?;
    let pod = Pod::from_bytes(&pod_bytes)
        .ok_or_else(|| PipeWireError::Pod("failed to construct capture format pod".into()))?;
    let mut params = [pod];

    let numeric_target = config.target.as_deref().and_then(|t| t.parse::<u32>().ok());
    stream
        .connect(
            Direction::Input,
            numeric_target,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| PipeWireError::Stream(e.to_string()))?;

    let mainloop_quit = mainloop.clone();
    let _attached = cmd_rx.attach(mainloop.loop_(), move |cmd| match cmd {
        SourceCmd::Quit => mainloop_quit.quit(),
    });

    let _ = ready_tx.send(Ok(()));
    mainloop.run();
    Ok(())
}

fn handle_capture_param_changed(data: &mut CaptureUserData, id: u32, param: Option<&Pod>) {
    let Some(param) = param else {
        return;
    };
    if id != ParamType::Format.as_raw() {
        return;
    }
    let Ok((media_type, media_subtype)) = parse_format(param) else {
        return;
    };
    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
        return;
    }
    if data.format.parse(param).is_ok() {
        data.format_ready = true;
        let size = data.format.size();
        info!(
            width = size.width,
            height = size.height,
            format = ?data.format.format(),
            "negotiated PipeWire capture video format"
        );
    }
}

fn handle_capture_process(stream: &pw::stream::Stream, data: &mut CaptureUserData) {
    if !data.format_ready {
        let _ = stream.dequeue_buffer();
        return;
    }
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let datas = buffer.datas_mut();
    let Some(plane) = datas.first_mut() else {
        return;
    };
    let chunk = plane.chunk();
    let offset = usize::try_from(chunk.offset()).unwrap_or(0);
    let size = usize::try_from(chunk.size()).unwrap_or(0);
    let stride = usize::try_from(chunk.stride()).unwrap_or(0);
    let Some(slice) = plane.data() else {
        return;
    };
    let Some(valid) = slice.get(offset..offset.saturating_add(size)) else {
        return;
    };

    let video_size = data.format.size();
    let Ok(dimensions) = Dimensions::new(video_size.width, video_size.height) else {
        return;
    };
    let Some(spa_fmt) = map_pw_video_format(data.format.format()) else {
        warn!(format = ?data.format.format(), "unsupported PipeWire capture format");
        return;
    };

    data.sequence += 1;
    let frame = decode_spa_frame(valid, stride, dimensions, spa_fmt, 0, data.sequence);
    if let Err(crossbeam_channel::TrySendError::Full(frame)) = data.frames_tx.try_send(frame) {
        let _ = data.frames_rx_drop.try_recv();
        let _ = data.frames_tx.try_send(frame);
    }
}

/// PipeWire virtual camera adapter implementing [`VideoSink`].
pub struct PipeWireVideoSink {
    info: SinkInfo,
    cmd_tx: pw::channel::Sender<SinkCmd>,
    alive: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PipeWireVideoSink {
    /// Connects to PipeWire on a dedicated main-loop thread and exposes a
    /// virtual `Video/Source` camera node with the configured properties.
    ///
    /// # Errors
    ///
    /// Returns [`PipeWireError`] if PipeWire initialisation, connection, or
    /// stream setup fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_core::Dimensions;
    /// use dp2_pipewire::{PipeWireSinkConfig, PipeWireVideoSink};
    ///
    /// let cfg = PipeWireSinkConfig::new(
    ///     "dp2-depth",
    ///     "Depth Pro 2 Camera",
    ///     Dimensions::new(640, 480)?,
    /// );
    /// let _sink = PipeWireVideoSink::connect(cfg)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn connect(config: PipeWireSinkConfig) -> Result<Self, PipeWireError> {
        let info = SinkInfo {
            name: config.node_name.clone(),
            description: config.node_description.clone(),
        };

        let alive = Arc::new(AtomicBool::new(true));
        let alive_worker = alive.clone();
        let (ready_tx, ready_rx) = bounded::<Result<(), PipeWireError>>(1);
        let (cmd_tx, cmd_rx) = pw::channel::channel::<SinkCmd>();

        let worker = thread::Builder::new()
            .name("dp2-pw-sink".into())
            .spawn(move || {
                run_sink_loop(&config, &ready_tx, cmd_rx);
                alive_worker.store(false, Ordering::Release);
            })
            .map_err(|e| PipeWireError::Stream(format!("failed to spawn PipeWire thread: {e}")))?;

        ready_rx
            .recv()
            .map_err(|_| PipeWireError::WorkerDisconnected)??;

        Ok(Self {
            info,
            cmd_tx,
            alive,
            worker: Some(worker),
        })
    }
}

impl VideoSink for PipeWireVideoSink {
    fn info(&self) -> SinkInfo {
        self.info.clone()
    }

    fn publish_frame(&mut self, frame: &VideoFrame) -> Result<(), SinkError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(SinkError::Disconnected);
        }
        self.cmd_tx
            .send(SinkCmd::Publish(frame.clone()))
            .map_err(|_| SinkError::Disconnected)
    }
}

impl Drop for PipeWireVideoSink {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(SinkCmd::Quit);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

struct SinkSharedState {
    latest_frame: Option<VideoFrame>,
    negotiated_dims: Dimensions,
    negotiated_format: PixelFormat,
    streaming: bool,
}

fn run_sink_loop(
    config: &PipeWireSinkConfig,
    ready_tx: &Sender<Result<(), PipeWireError>>,
    cmd_rx: pw::channel::Receiver<SinkCmd>,
) {
    if let Err(err) = run_sink_loop_inner(config, ready_tx, cmd_rx) {
        let _ = ready_tx.send(Err(err));
    }
}

fn run_sink_loop_inner(
    config: &PipeWireSinkConfig,
    ready_tx: &Sender<Result<(), PipeWireError>>,
    cmd_rx: pw::channel::Receiver<SinkCmd>,
) -> Result<(), PipeWireError> {
    pw::init();

    let mainloop =
        pw::main_loop::MainLoopRc::new(None).map_err(|e| PipeWireError::Connect(e.to_string()))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|e| PipeWireError::Connect(e.to_string()))?;
    let core = connect_core(&context, config.remote_name.as_deref())?;

    let mut props = PropertiesBox::new();
    props.insert(*pw::keys::MEDIA_TYPE, "Video");
    props.insert(*pw::keys::MEDIA_CATEGORY, "Source");
    props.insert(*pw::keys::MEDIA_ROLE, "Camera");
    props.insert(*pw::keys::MEDIA_CLASS, "Video/Source");
    props.insert(*pw::keys::NODE_NAME, config.node_name.as_str());
    props.insert(
        *pw::keys::NODE_DESCRIPTION,
        config.node_description.as_str(),
    );
    props.insert(*pw::keys::NODE_VIRTUAL, "true");
    props.insert(*pw::keys::NODE_DRIVER, "true");

    let stream = pw::stream::StreamRc::new(core.clone(), &config.node_name, props)
        .map_err(|e| PipeWireError::Stream(e.to_string()))?;

    let shared = std::rc::Rc::new(std::cell::RefCell::new(SinkSharedState {
        latest_frame: None,
        negotiated_dims: config.dimensions,
        negotiated_format: config.format,
        streaming: false,
    }));

    let shared_cb = shared.clone();
    let _listener = stream
        .add_local_listener_with_user_data(())
        .state_changed({
            let shared = shared_cb.clone();
            move |stream, (), old, new| {
                debug!(?old, ?new, "PipeWire virtual camera stream state changed");
                let is_streaming = matches!(new, pw::stream::StreamState::Streaming);
                let has_frame = {
                    let mut st = shared.borrow_mut();
                    st.streaming = is_streaming;
                    st.latest_frame.is_some()
                };
                if is_streaming && has_frame {
                    let _ = stream.trigger_process();
                }
            }
        })
        .param_changed({
            let shared = shared_cb.clone();
            move |stream, (), id, param| {
                handle_sink_param_changed(stream, &shared, id, param);
            }
        })
        .process(move |stream, ()| handle_sink_process(stream, &shared_cb))
        .register()
        .map_err(|e| PipeWireError::Stream(e.to_string()))?;

    let pod_bytes = build_sink_format_pod(config)?;
    let pod = Pod::from_bytes(&pod_bytes)
        .ok_or_else(|| PipeWireError::Pod("failed to construct sink format pod".into()))?;
    let mut params = [pod];

    stream
        .connect(
            Direction::Output,
            None,
            pw::stream::StreamFlags::DRIVER | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| PipeWireError::Stream(e.to_string()))?;

    let mainloop_quit = mainloop.clone();
    let stream_cmd = stream.clone();
    let shared_cmd = shared.clone();
    let _attached = cmd_rx.attach(mainloop.loop_(), move |cmd| match cmd {
        SinkCmd::Publish(frame) => {
            let is_streaming = {
                let mut st = shared_cmd.borrow_mut();
                st.latest_frame = Some(frame);
                st.streaming
            };
            if is_streaming {
                let _ = stream_cmd.trigger_process();
            }
        }
        SinkCmd::Quit => mainloop_quit.quit(),
    });

    let _ = ready_tx.send(Ok(()));
    mainloop.run();
    Ok(())
}

fn handle_sink_param_changed(
    stream: &pw::stream::Stream,
    shared: &std::cell::RefCell<SinkSharedState>,
    id: u32,
    param: Option<&Pod>,
) {
    let Some(param) = param else {
        return;
    };
    if id != ParamType::Format.as_raw() {
        return;
    }
    let Ok((media_type, media_subtype)) = parse_format(param) else {
        return;
    };
    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
        return;
    }
    let mut info = VideoInfoRaw::default();
    if info.parse(param).is_err() {
        return;
    }
    let size = info.size();
    let Ok(dims) = Dimensions::new(size.width, size.height) else {
        return;
    };
    let fmt = map_pw_to_pixel_format(info.format()).unwrap_or(PixelFormat::Rgba8);
    {
        let mut st = shared.borrow_mut();
        st.negotiated_dims = dims;
        st.negotiated_format = fmt;
    }
    info!(
        width = dims.width(),
        height = dims.height(),
        format = ?fmt,
        "negotiated PipeWire virtual camera format"
    );
    if let Ok(buf_bytes) = build_buffers_pod(dims, fmt)
        && let Some(pod) = Pod::from_bytes(&buf_bytes)
    {
        let mut params = [pod];
        if let Err(err) = stream.update_params(&mut params) {
            warn!(error = %err, "failed to update PipeWire sink buffer params");
        }
    }
}

fn handle_sink_process(stream: &pw::stream::Stream, shared: &std::cell::RefCell<SinkSharedState>) {
    let st = shared.borrow();
    let Some(ref frame) = st.latest_frame else {
        return;
    };
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let datas = buffer.datas_mut();
    let Some(plane) = datas.first_mut() else {
        return;
    };
    let Some(slice) = plane.data() else {
        return;
    };
    match encode_spa_frame(frame, st.negotiated_format, st.negotiated_dims, slice) {
        Ok(encoded) => {
            let chunk = plane.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = i32::try_from(encoded.stride).unwrap_or(0);
            *chunk.size_mut() = u32::try_from(encoded.size).unwrap_or(0);
        }
        Err(err) => {
            warn!(error = %err, "failed to encode frame into PipeWire output buffer");
        }
    }
}

fn connect_core(
    context: &pw::context::ContextRc,
    remote_name: Option<&str>,
) -> Result<pw::core::CoreRc, PipeWireError> {
    let props = remote_name.map(|name| {
        let mut p = PropertiesBox::new();
        p.insert(*pw::keys::REMOTE_NAME, name);
        p
    });
    context
        .connect_rc(props)
        .map_err(|e| PipeWireError::Connect(e.to_string()))
}

fn build_capture_format_pod() -> Result<Vec<u8>, PipeWireError> {
    let obj = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::RGBA,
            VideoFormat::RGBA,
            VideoFormat::RGBx,
            VideoFormat::BGRA,
            VideoFormat::BGRx,
            VideoFormat::RGB,
            VideoFormat::BGR,
            VideoFormat::YUY2,
            VideoFormat::NV12,
            VideoFormat::I420,
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 640,
                height: 480
            },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: 4096,
                height: 4096
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 30, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction {
                num: 1000,
                denom: 1
            }
        ),
    );
    serialize_object(obj)
}

fn build_sink_format_pod(config: &PipeWireSinkConfig) -> Result<Vec<u8>, PipeWireError> {
    let preferred = pixel_format_to_pw(config.format);
    let rect = Rectangle {
        width: config.dimensions.width(),
        height: config.dimensions.height(),
    };
    let rate = Fraction {
        num: config.framerate_num,
        denom: config.framerate_denom.max(1),
    };

    let obj = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            preferred,
            VideoFormat::RGBA,
            VideoFormat::BGRA,
            VideoFormat::RGB,
            VideoFormat::BGR,
        ),
        spa::pod::property!(FormatProperties::VideoSize, Rectangle, rect),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            rate,
            Fraction { num: 0, denom: 1 },
            Fraction { num: 120, denom: 1 }
        ),
    );
    serialize_object(obj)
}

fn build_buffers_pod(
    dimensions: Dimensions,
    format: PixelFormat,
) -> Result<Vec<u8>, PipeWireError> {
    let stride = i32::try_from((dimensions.width() as usize) * format.bytes_per_pixel())
        .map_err(|_| PipeWireError::Pod("stride overflow".into()))?;
    let size = i32::try_from((dimensions.height() as usize) * usize::try_from(stride).unwrap_or(0))
        .map_err(|_| PipeWireError::Pod("buffer size overflow".into()))?;

    let data_types =
        i32::try_from((1u32 << spa::sys::SPA_DATA_MemPtr) | (1u32 << spa::sys::SPA_DATA_MemFd))
            .unwrap_or(1);

    let obj = Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: spa::sys::SPA_PARAM_Buffers,
        properties: vec![
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_buffers,
                flags: PropertyFlags::empty(),
                value: Value::Choice(spa::pod::ChoiceValue::Int(spa::utils::Choice(
                    spa::utils::ChoiceFlags::empty(),
                    spa::utils::ChoiceEnum::Range {
                        default: 4,
                        min: 2,
                        max: 8,
                    },
                ))),
            },
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_blocks,
                flags: PropertyFlags::empty(),
                value: Value::Int(1),
            },
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_size,
                flags: PropertyFlags::empty(),
                value: Value::Int(size),
            },
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_stride,
                flags: PropertyFlags::empty(),
                value: Value::Int(stride),
            },
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_dataType,
                flags: PropertyFlags::empty(),
                value: Value::Int(data_types),
            },
        ],
    };

    serialize_object(obj)
}

fn serialize_object(obj: Object) -> Result<Vec<u8>, PipeWireError> {
    let (cursor, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))
        .map_err(|e| PipeWireError::Pod(format!("{e:?}")))?;
    Ok(cursor.into_inner())
}

const fn map_pw_video_format(format: VideoFormat) -> Option<SpaVideoFormat> {
    match format {
        VideoFormat::RGBA => Some(SpaVideoFormat::Rgba),
        VideoFormat::RGBx => Some(SpaVideoFormat::Rgbx),
        VideoFormat::BGRA => Some(SpaVideoFormat::Bgra),
        VideoFormat::BGRx => Some(SpaVideoFormat::Bgrx),
        VideoFormat::RGB => Some(SpaVideoFormat::Rgb),
        VideoFormat::BGR => Some(SpaVideoFormat::Bgr),
        VideoFormat::YUY2 => Some(SpaVideoFormat::Yuy2),
        VideoFormat::I420 => Some(SpaVideoFormat::I420),
        VideoFormat::NV12 => Some(SpaVideoFormat::Nv12),
        _ => None,
    }
}

const fn map_pw_to_pixel_format(format: VideoFormat) -> Option<PixelFormat> {
    match format {
        VideoFormat::RGBA | VideoFormat::RGBx => Some(PixelFormat::Rgba8),
        VideoFormat::BGRA | VideoFormat::BGRx => Some(PixelFormat::Bgra8),
        VideoFormat::RGB => Some(PixelFormat::Rgb8),
        VideoFormat::BGR => Some(PixelFormat::Bgr8),
        _ => None,
    }
}

const fn pixel_format_to_pw(format: PixelFormat) -> VideoFormat {
    match format {
        PixelFormat::Rgba8 => VideoFormat::RGBA,
        PixelFormat::Bgra8 => VideoFormat::BGRA,
        PixelFormat::Rgb8 => VideoFormat::RGB,
        PixelFormat::Bgr8 => VideoFormat::BGR,
    }
}

/// Queries the PipeWire registry for all available `Video/Source` nodes,
/// excluding `exclude_node_name` so `dp2` does not list its own virtual camera.
///
/// # Errors
///
/// Returns [`PipeWireError`] if connecting to PipeWire fails.
///
/// # Examples
///
/// ```no_run
/// use dp2_pipewire::list_pipewire_video_sources;
///
/// let _devices = list_pipewire_video_sources(None, "dp2-depth")?;
/// # Ok::<(), dp2_pipewire::PipeWireError>(())
/// ```
pub fn list_pipewire_video_sources(
    remote_name: Option<&str>,
    exclude_node_name: &str,
) -> Result<Vec<VideoDeviceDescriptor>, PipeWireError> {
    let remote = remote_name.map(ToOwned::to_owned);
    let exclude = exclude_node_name.to_owned();

    let handle = thread::Builder::new()
        .name("dp2-pw-list".into())
        .spawn(move || list_sources_on_thread(remote.as_deref(), &exclude))
        .map_err(|e| PipeWireError::Stream(format!("failed to spawn PipeWire list thread: {e}")))?;

    handle
        .join()
        .map_err(|_| PipeWireError::WorkerDisconnected)?
}

fn list_sources_on_thread(
    remote_name: Option<&str>,
    exclude_node_name: &str,
) -> Result<Vec<VideoDeviceDescriptor>, PipeWireError> {
    pw::init();

    let mainloop =
        pw::main_loop::MainLoopRc::new(None).map_err(|e| PipeWireError::Connect(e.to_string()))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|e| PipeWireError::Connect(e.to_string()))?;
    let core = connect_core(&context, remote_name)?;
    let registry = core
        .get_registry()
        .map_err(|e| PipeWireError::Connect(e.to_string()))?;

    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let devices = std::rc::Rc::new(std::cell::RefCell::new(Vec::<VideoDeviceDescriptor>::new()));

    let pending = core
        .sync(0)
        .map_err(|e| PipeWireError::Connect(e.to_string()))?;

    let done_cb = done.clone();
    let loop_cb = mainloop.clone();
    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                done_cb.set(true);
                loop_cb.quit();
            }
        })
        .register();

    let devices_cb = devices.clone();
    let exclude = exclude_node_name.to_owned();
    let _reg_listener = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ != pw::types::ObjectType::Node {
                return;
            }
            let Some(props) = global.props else {
                return;
            };
            let Some(media_class) = props.get(*pw::keys::MEDIA_CLASS) else {
                return;
            };
            if !media_class.starts_with("Video/Source") {
                return;
            }
            let Some(name) = props.get(*pw::keys::NODE_NAME) else {
                return;
            };
            if name == exclude {
                return;
            }
            let description = props
                .get(*pw::keys::NODE_DESCRIPTION)
                .or_else(|| props.get(*pw::keys::NODE_NICK))
                .unwrap_or(name);
            devices_cb.borrow_mut().push(VideoDeviceDescriptor {
                name: name.to_owned(),
                description: description.to_owned(),
                is_imported_video: false,
                video_path: None,
            });
        })
        .register();

    while !done.get() {
        mainloop.run();
    }

    let result = devices.borrow().clone();
    Ok(result)
}
