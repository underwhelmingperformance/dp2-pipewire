// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    thread,
};

use clap::Parser;
use dp2_core::{
    Clock, DepthEstimator, DynamicDepthEstimator, DynamicVideoSource, EffectRegistry, ModelFactory,
    Pipeline, PipelineController, PipelineStatus, SourceFactory, SourceInfo, SystemClock,
    VideoSink, VideoSource,
};
use dp2_pipewire::{PipeWireSourceManager, PipeWireVideoSink};
use dp2_varlink::{Dp2VarlinkClient, serve_from_systemd_or_socket};
use tokio::sync::oneshot;
use tracing::{error, info, warn};

use crate::{AppError, Cli, Command, RunArgs, default_varlink_socket, init_tracing};

/// Entry point invoked by `main`.
///
/// # Examples
///
/// ```no_run
/// let _code = dp2::entry();
/// ```
pub fn entry() -> ExitCode {
    let filter = std::env::var("DP2_LOG")
        .ok()
        .or_else(|| std::env::var("RUST_LOG").ok());
    init_tracing(filter.as_deref());

    let cli = Cli::parse();
    let xdg_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            error!(error = %err, "failed to build Tokio runtime");
            return ExitCode::FAILURE;
        }
    };

    let mut stdout = std::io::stdout().lock();
    match runtime.block_on(run_command(cli, xdg_runtime_dir.as_deref(), &mut stdout)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!(error = %err, "command failed");
            ExitCode::FAILURE
        }
    }
}

/// Executes a parsed [`Cli`] command with injected runtime directory and output
/// stream.
///
/// # Errors
///
/// Returns [`AppError`] when configuration, PipeWire, depth inference, or
/// Varlink IPC fails.
///
/// # Examples
///
/// ```no_run
/// use clap::Parser;
/// use dp2::{Cli, run_command};
///
/// # async fn example() -> Result<(), dp2::AppError> {
/// let cli = Cli::try_parse_from(["dp2", "status", "--socket", "/tmp/dp2.sock"]).unwrap();
/// let mut out = Vec::new();
/// run_command(cli, None, &mut out).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run_command<W: Write>(
    cli: Cli,
    xdg_runtime_dir: Option<&Path>,
    writer: &mut W,
) -> Result<(), AppError> {
    if let Command::Run(args) = cli.command {
        return run_daemon(&args, xdg_runtime_dir).await;
    }

    run_control_command(cli.command, xdg_runtime_dir, writer).await
}

async fn connect_client(
    socket: Option<&Path>,
    xdg_runtime_dir: Option<&Path>,
) -> Result<Dp2VarlinkClient, AppError> {
    let path = default_varlink_socket(socket, xdg_runtime_dir)?;
    Ok(Dp2VarlinkClient::connect(&path).await?)
}

fn command_socket(command: &Command) -> Option<&Path> {
    match command {
        Command::Run(args) => args.varlink_socket.as_deref(),
        Command::Status(args)
        | Command::ListEffects(args)
        | Command::GetEffect(args)
        | Command::ListSources(args)
        | Command::ListModels(args)
        | Command::ResetDepth(args)
        | Command::Shutdown(args) => args.socket.as_deref(),
        Command::SetEffect { socket, .. }
        | Command::SetSource { socket, .. }
        | Command::ImportVideo { socket, .. }
        | Command::RemoveVideo { socket, .. }
        | Command::SetModel { socket, .. }
        | Command::Preview { socket, .. } => socket.socket.as_deref(),
    }
}

async fn run_control_command<W: Write>(
    command: Command,
    xdg_runtime_dir: Option<&Path>,
    writer: &mut W,
) -> Result<(), AppError> {
    let mut client = connect_client(command_socket(&command), xdg_runtime_dir).await?;
    match command {
        Command::Run(_) => unreachable!("handled by run_command"),
        Command::Status(_) => write_json_line(writer, &client.get_status().await?),
        Command::ListEffects(_) => write_json_line(writer, &client.list_effects().await?),
        Command::GetEffect(_) => write_json_line(writer, &client.get_effect().await?),
        Command::SetEffect { effect, .. } => write_json_line(
            writer,
            &client.set_effect(&effect.into_effect_config()).await?,
        ),
        Command::ListSources(_) => write_json_line(writer, &client.list_sources().await?),
        Command::SetSource { name, .. } => {
            write_json_line(writer, &client.set_source(name.as_deref()).await?)
        }
        Command::ImportVideo {
            path, no_select, ..
        } => {
            let imported = client
                .import_video(&path.to_string_lossy(), !no_select)
                .await?;
            write_json_line(writer, &imported)
        }
        Command::RemoveVideo { name, .. } => {
            client.remove_video(&name).await?;
            writeln!(writer, "{{\"ok\":true}}")?;
            Ok(())
        }
        Command::ListModels(_) => write_json_line(writer, &client.list_models().await?),
        Command::SetModel { id, .. } => {
            let applied = client.set_model(&id).await?;
            write_json_line(writer, &serde_json::json!({ "model": applied }))
        }
        Command::Preview { max_width, .. } => {
            write_json_line(writer, &client.get_preview(Some(max_width)).await?)
        }
        Command::ResetDepth(_) => {
            client.reset_depth_state().await?;
            writeln!(writer, "{{\"ok\":true}}")?;
            Ok(())
        }
        Command::Shutdown(_) => {
            client.shutdown().await?;
            writeln!(writer, "{{\"ok\":true}}")?;
            Ok(())
        }
    }
}

async fn run_daemon(args: &RunArgs, xdg_runtime_dir: Option<&Path>) -> Result<(), AppError> {
    let fallback_socket =
        default_varlink_socket(args.varlink_socket.as_deref(), xdg_runtime_dir).ok();
    let sink_cfg = args.sink_config()?;
    let sink_dims = sink_cfg.dimensions;
    let sink = PipeWireVideoSink::connect(sink_cfg)?;

    let source_manager = Arc::new(PipeWireSourceManager::new(
        args.pipewire_remote.clone(),
        args.sink_name.clone(),
        sink_dims,
    ));

    let (catalog, primary_model_id) = args.model_catalog();
    let model_factory = Arc::new(catalog);

    let initial_estimator = if let Some(ref id) = primary_model_id {
        Some(model_factory.load_model(id)?)
    } else {
        warn!("starting without a loaded Depth Pro 2 model; select one via Varlink");
        None
    };

    let initial_source_info = SourceInfo {
        name: args.source.clone().unwrap_or_else(|| "none".into()),
        description: args.source.as_ref().map_or_else(
            || "No video source selected".into(),
            |name| format!("PipeWire source {name}"),
        ),
    };

    let controller = PipelineController::new(
        EffectRegistry::with_builtin_effects(),
        args.initial_effect(),
        initial_source_info,
        sink.info(),
    )?;
    controller.set_source_factory(source_manager.clone());
    controller.set_model_factory(model_factory);
    controller.set_stay_alive_when_idle(args.idle || args.source.is_none());
    if let Some(id) = primary_model_id {
        controller.set_active_model(id);
    }

    let initial_source = if args.idle {
        None
    } else {
        let stop_ctrl = controller.clone();
        let initial_gen = controller.source_generation();
        let should_stop = Box::new(move || {
            stop_ctrl.is_shutdown_requested() || stop_ctrl.source_generation() != initial_gen
        });
        match source_manager.open_source(args.source.as_deref(), should_stop) {
            Ok(source) => Some(source),
            Err(err) if args.source.is_none() => {
                warn!(error = %err, "no initial PipeWire camera available; waiting for selection");
                None
            }
            Err(err) => return Err(AppError::Pipeline(err.into())),
        }
    };

    let source = DynamicVideoSource::new(initial_source, controller.clone());
    let estimator = DynamicDepthEstimator::new(initial_estimator, controller.clone());

    let status = run_pipeline_with_varlink_opt(
        source,
        sink,
        estimator,
        SystemClock::new(),
        controller,
        fallback_socket.as_deref(),
    )
    .await?;

    info!(frames = status.frames_processed, "pipeline stopped cleanly");
    Ok(())
}

/// Runs an injected [`Pipeline`] on a dedicated worker thread concurrently with
/// the Varlink control server and `Ctrl+C` shutdown handler.
///
/// # Errors
///
/// Returns [`AppError`] if the pipeline or Varlink server encounters an error.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use dp2::run_pipeline_with_varlink;
/// use dp2_core::{
///     EffectConfig, EffectRegistry, PipelineController, VideoSink, VideoSource,
///     testing::{FakeClock, FakeDepthEstimator, FakeVideoSource, SpyVideoSink},
/// };
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let dir = tempfile::tempdir()?;
/// let socket = dir.path().join("dp2.sock");
/// let source = FakeVideoSource::empty("in", "Input");
/// let sink = SpyVideoSink::new("out", "Output");
/// let controller = PipelineController::new(
///     EffectRegistry::with_builtin_effects(),
///     EffectConfig::Passthrough,
///     source.info(),
///     sink.info(),
/// )?;
/// let status = run_pipeline_with_varlink(
///     source,
///     sink,
///     FakeDepthEstimator::constant(2.0, 500.0),
///     FakeClock::with_step(Duration::from_millis(1)),
///     controller,
///     &socket,
/// )
/// .await?;
/// assert_eq!(status.frames_processed, 0);
/// # Ok(())
/// # }
/// ```
pub async fn run_pipeline_with_varlink<S, K, D, C>(
    source: S,
    sink: K,
    estimator: D,
    clock: C,
    controller: PipelineController,
    varlink_socket: &Path,
) -> Result<PipelineStatus, AppError>
where
    S: VideoSource + 'static,
    K: VideoSink + 'static,
    D: DepthEstimator + 'static,
    C: Clock + 'static,
{
    run_pipeline_with_varlink_opt(
        source,
        sink,
        estimator,
        clock,
        controller,
        Some(varlink_socket),
    )
    .await
}

async fn run_pipeline_with_varlink_opt<S, K, D, C>(
    source: S,
    sink: K,
    estimator: D,
    clock: C,
    controller: PipelineController,
    varlink_socket: Option<&Path>,
) -> Result<PipelineStatus, AppError>
where
    S: VideoSource + 'static,
    K: VideoSink + 'static,
    D: DepthEstimator + 'static,
    C: Clock + 'static,
{
    let (done_tx, done_rx) = oneshot::channel::<Result<PipelineStatus, dp2_core::PipelineError>>();
    let pipeline_controller = controller.clone();

    let worker = thread::Builder::new()
        .name("dp2-pipeline".into())
        .spawn(move || {
            let mut pipeline = Pipeline::new(source, sink, estimator, clock, pipeline_controller);
            let _ = done_tx.send(pipeline.run());
        })?;

    let ctrl_c_controller = controller.clone();
    let mut done_rx = done_rx;
    let mut pipeline_result: Option<Result<PipelineStatus, dp2_core::PipelineError>> = None;

    let shutdown_signal = async {
        tokio::select! {
            res = &mut done_rx => {
                pipeline_result = res.ok();
            }
            _ = tokio::signal::ctrl_c() => {
                info!("received interrupt signal, stopping pipeline");
                ctrl_c_controller.request_shutdown();
            }
        }
    };

    let server_result =
        serve_from_systemd_or_socket(varlink_socket, controller, shutdown_signal).await;

    let status = if let Some(res) = pipeline_result {
        res?
    } else {
        done_rx.await.map_err(|_| AppError::PipelinePanicked)??
    };

    worker.join().map_err(|_| AppError::PipelinePanicked)?;
    server_result?;
    Ok(status)
}

fn write_json_line<W: Write, T: serde::Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<(), AppError> {
    serde_json::to_writer(&mut *writer, value)?;
    writeln!(writer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use dp2_core::{
        Dimensions, EffectConfig, PixelFormat, SourceError, SourceInfo, VideoFrame, VideoSource,
        testing::{FakeClock, FakeDepthEstimator, SpyVideoSink},
    };
    use pretty_assertions::assert_eq;

    use super::*;

    struct ChannelSource {
        info: SourceInfo,
        rx: mpsc::Receiver<VideoFrame>,
    }

    impl VideoSource for ChannelSource {
        fn info(&self) -> SourceInfo {
            self.info.clone()
        }

        fn next_frame(&mut self) -> Result<Option<VideoFrame>, SourceError> {
            Ok(self.rx.recv().ok())
        }
    }

    #[tokio::test]
    async fn cli_commands_control_running_pipeline_over_varlink()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let socket_path = dir.path().join("dp2-app-test.sock");

        let (frame_tx, frame_rx) = mpsc::channel();
        let source = ChannelSource {
            info: SourceInfo {
                name: "fake-cam".into(),
                description: "Fake Camera".into(),
            },
            rx: frame_rx,
        };
        let sink = SpyVideoSink::new("dp2-depth", "Depth Camera");
        let controller = PipelineController::new(
            EffectRegistry::with_builtin_effects(),
            EffectConfig::default(),
            source.info(),
            sink.info(),
        )?;

        let daemon_socket = socket_path.clone();
        let daemon = run_pipeline_with_varlink(
            source,
            sink,
            FakeDepthEstimator::horizontal_gradient(1.0, 4.0, 500.0),
            FakeClock::with_step(Duration::from_millis(2)),
            controller,
            &daemon_socket,
        );

        let driver = async {
            while !socket_path.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            let frame1 = VideoFrame::packed(
                Dimensions::new(2, 1)?,
                PixelFormat::Rgb8,
                1,
                1,
                vec![10, 20, 30, 40, 50, 60],
            )?;
            frame_tx.send(frame1)?;

            let mut out = Vec::new();
            let swap_cli = Cli::try_parse_from([
                "dp2",
                "set-effect",
                "--socket",
                socket_path.to_str().expect("UTF-8 socket path"),
                "passthrough",
            ])?;
            run_command(swap_cli, None, &mut out).await?;
            let swap_json: serde_json::Value = serde_json::from_slice(&out)?;
            assert_eq!(swap_json["id"], "passthrough");

            let frame2 = VideoFrame::packed(
                Dimensions::new(2, 1)?,
                PixelFormat::Rgb8,
                2,
                2,
                vec![70, 80, 90, 100, 110, 120],
            )?;
            frame_tx.send(frame2)?;
            drop(frame_tx);

            Ok::<(), Box<dyn std::error::Error>>(())
        };

        let (daemon_res, driver_res) = tokio::join!(daemon, driver);
        driver_res?;
        let final_status = daemon_res?;
        assert_eq!(final_status.frames_processed, 2);
        assert_eq!(final_status.active_effect, EffectConfig::Passthrough);
        Ok(())
    }
}
