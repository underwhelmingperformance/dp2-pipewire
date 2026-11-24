// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use dp2_core::{Colormap, DepthMapConfig, Dimensions, EffectConfig, EffectId, PixelFormat};
use dp2_depth::{DepthPro2Config, FocalLengthPolicy};
use dp2_pipewire::{PipeWireSinkConfig, PipeWireSourceConfig};
use dp2_varlink::INTERFACE_NAME;

use crate::AppError;

/// PipeWire virtual camera daemon powered by Depth Pro 2 and Varlink.
#[derive(Debug, Clone, PartialEq, Parser)]
#[command(name = "dp2", version, about)]
pub struct Cli {
    /// Operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level `dp2` subcommands.
#[derive(Debug, Clone, PartialEq, Subcommand)]
pub enum Command {
    /// Start the PipeWire capture, Depth Pro 2 inference, and virtual camera
    /// daemon.
    Run(Box<RunArgs>),
    /// Query status from a running daemon over Varlink.
    Status(SocketArgs),
    /// List registered effects on a running daemon over Varlink.
    ListEffects(SocketArgs),
    /// Show the currently active effect configuration over Varlink.
    GetEffect(SocketArgs),
    /// Hot-swap the active effect at runtime over Varlink.
    SetEffect {
        /// Shared Varlink socket option.
        #[command(flatten)]
        socket: SocketArgs,
        /// Target effect and parameters.
        #[command(subcommand)]
        effect: SetEffectCommand,
    },
    /// Reset Depth Pro 2 temporal streaming state over Varlink.
    ResetDepth(SocketArgs),
    /// Request clean shutdown of a running daemon over Varlink.
    Shutdown(SocketArgs),
}

/// Shared Varlink socket path option for control subcommands.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct SocketArgs {
    /// Path to the daemon's Varlink Unix socket.
    #[arg(long, env = "DP2_VARLINK_SOCKET")]
    pub socket: Option<PathBuf>,
}

/// Configuration flags for `dp2 run`.
#[derive(Debug, Clone, PartialEq, Args)]
pub struct RunArgs {
    /// Target PipeWire video node name, serial, or object ID.
    #[arg(long)]
    pub source: Option<String>,

    /// PipeWire `node.name` for the virtual camera output device.
    #[arg(long, default_value = "dp2-depth")]
    pub sink_name: String,

    /// Human-readable PipeWire `node.description` for the virtual camera output
    /// device.
    #[arg(long, default_value = "Depth Pro 2 Camera")]
    pub sink_description: String,

    /// Output virtual camera frame width in pixels.
    #[arg(long, default_value_t = 640)]
    pub width: u32,

    /// Output virtual camera frame height in pixels.
    #[arg(long, default_value_t = 480)]
    pub height: u32,

    /// Output virtual camera nominal framerate in frames per second.
    #[arg(long, default_value_t = 30)]
    pub framerate: u32,

    /// Output virtual camera pixel format.
    #[arg(long, value_enum, default_value_t = PixelFormatArg::Rgba)]
    pub format: PixelFormatArg,

    /// Optional PipeWire remote socket name (for example an isolated test
    /// daemon).
    #[arg(long, env = "PIPEWIRE_REMOTE")]
    pub pipewire_remote: Option<String>,

    /// Path to the Depth Pro 2 ONNX model file.
    #[arg(long, env = "DP2_MODEL_PATH")]
    pub model: PathBuf,

    /// Optional path to the Depth Pro 2 focal-length estimator ONNX model.
    #[arg(long, env = "DP2_FOCAL_MODEL_PATH")]
    pub focal_model: Option<PathBuf>,

    /// Known horizontal focal length in pixels for the input camera.
    #[arg(long)]
    pub focal_length_px: Option<f32>,

    /// Assumed horizontal field of view in degrees when neither focal length
    /// nor focal model is set.
    #[arg(long, default_value_t = 60.0)]
    pub fov_degrees: f32,

    /// Path to the ONNX Runtime shared library (`libonnxruntime.so`).
    #[arg(long, env = "ORT_DYLIB_PATH")]
    pub ort_dylib: Option<PathBuf>,

    /// Optional ONNX Runtime intra-op thread count.
    #[arg(long)]
    pub ort_threads: Option<usize>,

    /// Path where the Varlink control Unix socket will be bound.
    #[arg(long, env = "DP2_VARLINK_SOCKET")]
    pub varlink_socket: Option<PathBuf>,

    /// Initial frame effect active at start-up.
    #[arg(long, value_enum, default_value_t = EffectPresetArg::DepthMap)]
    pub effect: EffectPresetArg,

    /// Initial colour palette when `--effect depth-map` is selected.
    #[arg(long, value_enum, default_value_t = ColormapArg::Turbo)]
    pub colormap: ColormapArg,

    /// Optional fixed near depth bound in metres.
    #[arg(long)]
    pub min_depth: Option<f32>,

    /// Optional fixed far depth bound in metres.
    #[arg(long)]
    pub max_depth: Option<f32>,

    /// Reverse near and far depth colours.
    #[arg(long, default_value_t = false)]
    pub invert_depth: bool,
}

impl RunArgs {
    /// Builds the [`PipeWireSourceConfig`] corresponding to these CLI flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use clap::Parser;
    /// use dp2::{Cli, Command};
    ///
    /// let cli = Cli::try_parse_from(["dp2", "run", "--model", "m.onnx", "--source", "cam0"])?;
    /// let Command::Run(args) = cli.command else {
    ///     panic!("expected run")
    /// };
    /// assert_eq!(args.source_config().target.as_deref(), Some("cam0"));
    /// # Ok::<(), clap::Error>(())
    /// ```
    #[must_use]
    pub fn source_config(&self) -> PipeWireSourceConfig {
        let mut cfg = PipeWireSourceConfig::new(self.source.clone());
        cfg.remote_name.clone_from(&self.pipewire_remote);
        cfg
    }

    /// Builds the validated [`PipeWireSinkConfig`] corresponding to these CLI
    /// flags.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Frame`] if `width` or `height` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use clap::Parser;
    /// use dp2::{Cli, Command};
    ///
    /// let cli = Cli::try_parse_from([
    ///     "dp2", "run", "--model", "m.onnx", "--width", "320", "--height", "240",
    /// ])?;
    /// let Command::Run(args) = cli.command else {
    ///     panic!("expected run")
    /// };
    /// let cfg = args.sink_config()?;
    /// assert_eq!(cfg.dimensions.width(), 320);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn sink_config(&self) -> Result<PipeWireSinkConfig, AppError> {
        let dimensions = Dimensions::new(self.width, self.height)?;
        let mut cfg = PipeWireSinkConfig::new(
            self.sink_name.clone(),
            self.sink_description.clone(),
            dimensions,
        );
        cfg.format = self.format.into();
        cfg.framerate_num = self.framerate.max(1);
        cfg.remote_name.clone_from(&self.pipewire_remote);
        Ok(cfg)
    }

    /// Builds the [`DepthPro2Config`] corresponding to these CLI flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use clap::Parser;
    /// use dp2::{Cli, Command};
    /// use dp2_depth::FocalLengthPolicy;
    ///
    /// let cli = Cli::try_parse_from([
    ///     "dp2",
    ///     "run",
    ///     "--model",
    ///     "m.onnx",
    ///     "--focal-length-px",
    ///     "525.0",
    /// ])?;
    /// let Command::Run(args) = cli.command else {
    ///     panic!("expected run")
    /// };
    /// assert_eq!(
    ///     args.depth_config().focal_length,
    ///     FocalLengthPolicy::FixedPx(525.0)
    /// );
    /// # Ok::<(), clap::Error>(())
    /// ```
    #[must_use]
    pub fn depth_config(&self) -> DepthPro2Config {
        let focal_length = if let Some(f_px) = self.focal_length_px {
            FocalLengthPolicy::FixedPx(f_px)
        } else if let Some(ref focal_model) = self.focal_model {
            FocalLengthPolicy::EstimateWithModel(focal_model.clone())
        } else {
            FocalLengthPolicy::HorizontalFovDegrees(self.fov_degrees)
        };

        DepthPro2Config {
            model_path: self.model.clone(),
            ort_dylib_path: self.ort_dylib.clone(),
            focal_length,
            intra_threads: self.ort_threads,
        }
    }

    /// Builds the initial [`EffectConfig`] selected by these CLI flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use clap::Parser;
    /// use dp2::{Cli, Command};
    /// use dp2_core::EffectConfig;
    ///
    /// let cli = Cli::try_parse_from(["dp2", "run", "--model", "m.onnx", "--effect", "passthrough"])?;
    /// let Command::Run(args) = cli.command else {
    ///     panic!("expected run")
    /// };
    /// assert_eq!(args.initial_effect(), EffectConfig::Passthrough);
    /// # Ok::<(), clap::Error>(())
    /// ```
    #[must_use]
    pub const fn initial_effect(&self) -> EffectConfig {
        match self.effect {
            EffectPresetArg::DepthMap => EffectConfig::DepthMap(DepthMapConfig {
                colormap: self.colormap.into_colormap(),
                min_depth_meters: self.min_depth,
                max_depth_meters: self.max_depth,
                invert: self.invert_depth,
            }),
            EffectPresetArg::Passthrough => EffectConfig::Passthrough,
        }
    }
}

/// CLI selector for the initial built-in effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EffectPresetArg {
    /// Depth-map visualiser.
    DepthMap,
    /// Unmodified passthrough.
    Passthrough,
}

/// CLI colour palette selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColormapArg {
    /// Turbo rainbow palette.
    Turbo,
    /// Viridis palette.
    Viridis,
    /// Magma palette.
    Magma,
    /// Linear greyscale.
    Grayscale,
}

impl ColormapArg {
    /// Converts this CLI value into a [`Colormap`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2::ColormapArg;
    /// use dp2_core::Colormap;
    ///
    /// assert_eq!(ColormapArg::Viridis.into_colormap(), Colormap::Viridis);
    /// ```
    #[must_use]
    pub const fn into_colormap(self) -> Colormap {
        match self {
            Self::Turbo => Colormap::Turbo,
            Self::Viridis => Colormap::Viridis,
            Self::Magma => Colormap::Magma,
            Self::Grayscale => Colormap::Grayscale,
        }
    }
}

/// CLI pixel format selector for the virtual camera sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PixelFormatArg {
    /// 32-bit RGBA.
    Rgba,
    /// 32-bit BGRA.
    Bgra,
    /// 24-bit RGB.
    Rgb,
    /// 24-bit BGR.
    Bgr,
}

impl From<PixelFormatArg> for PixelFormat {
    fn from(value: PixelFormatArg) -> Self {
        match value {
            PixelFormatArg::Rgba => Self::Rgba8,
            PixelFormatArg::Bgra => Self::Bgra8,
            PixelFormatArg::Rgb => Self::Rgb8,
            PixelFormatArg::Bgr => Self::Bgr8,
        }
    }
}

/// Subcommands under `dp2 set-effect`.
#[derive(Debug, Clone, PartialEq, Subcommand)]
pub enum SetEffectCommand {
    /// Switch to the depth-map visualiser effect.
    DepthMap(DepthMapEffectArgs),
    /// Switch to unmodified passthrough (skips depth inference).
    Passthrough,
    /// Switch to a custom registered effect by identifier and `KEY=VALUE`
    /// parameters.
    Custom(CustomEffectArgs),
}

impl SetEffectCommand {
    /// Converts the parsed subcommand into an [`EffectConfig`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2::{ColormapArg, DepthMapEffectArgs, SetEffectCommand};
    /// use dp2_core::{Colormap, DepthMapConfig, EffectConfig};
    ///
    /// let cmd = SetEffectCommand::DepthMap(DepthMapEffectArgs {
    ///     colormap: ColormapArg::Magma,
    ///     min_depth: Some(0.5),
    ///     max_depth: Some(5.0),
    ///     invert: false,
    /// });
    /// assert_eq!(
    ///     cmd.into_effect_config(),
    ///     EffectConfig::DepthMap(DepthMapConfig {
    ///         colormap: Colormap::Magma,
    ///         min_depth_meters: Some(0.5),
    ///         max_depth_meters: Some(5.0),
    ///         invert: false,
    ///     })
    /// );
    /// ```
    #[must_use]
    pub fn into_effect_config(self) -> EffectConfig {
        match self {
            Self::DepthMap(args) => EffectConfig::DepthMap(DepthMapConfig {
                colormap: args.colormap.into_colormap(),
                min_depth_meters: args.min_depth,
                max_depth_meters: args.max_depth,
                invert: args.invert,
            }),
            Self::Passthrough => EffectConfig::Passthrough,
            Self::Custom(args) => EffectConfig::Custom {
                id: EffectId::new(args.id),
                parameters: args.param.into_iter().collect::<BTreeMap<_, _>>(),
            },
        }
    }
}

/// Arguments for `dp2 set-effect depth-map`.
#[derive(Debug, Clone, PartialEq, Args)]
pub struct DepthMapEffectArgs {
    /// Colour palette to apply.
    #[arg(long, value_enum, default_value_t = ColormapArg::Turbo)]
    pub colormap: ColormapArg,
    /// Optional fixed near depth bound in metres.
    #[arg(long)]
    pub min_depth: Option<f32>,
    /// Optional fixed far depth bound in metres.
    #[arg(long)]
    pub max_depth: Option<f32>,
    /// Reverse near and far colours.
    #[arg(long, default_value_t = false)]
    pub invert: bool,
}

/// Arguments for `dp2 set-effect custom`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct CustomEffectArgs {
    /// Registered custom effect identifier.
    pub id: String,
    /// Key-value parameter pair (`KEY=VALUE`).
    #[arg(long = "param", value_parser = parse_key_val)]
    pub param: Vec<(String, String)>,
}

fn parse_key_val(raw: &str) -> Result<(String, String), String> {
    let Some((key, value)) = raw.split_once('=') else {
        return Err(format!("expected KEY=VALUE parameter, got '{raw}'"));
    };
    if key.is_empty() {
        return Err("parameter key must not be empty".into());
    }
    Ok((key.to_string(), value.to_string()))
}

/// Resolves the Varlink socket path from an explicit option or
/// `xdg_runtime_dir`.
///
/// # Errors
///
/// Returns [`AppError::MissingSocketPath`] if neither `explicit` nor
/// `xdg_runtime_dir` is supplied.
///
/// # Examples
///
/// ```
/// use std::path::{Path, PathBuf};
///
/// use dp2::default_varlink_socket;
///
/// let sock = default_varlink_socket(None, Some(Path::new("/run/user/1000")))?;
/// assert_eq!(
///     sock,
///     PathBuf::from("/run/user/1000/uk.org.orangesquash.dp2")
/// );
/// # Ok::<(), dp2::AppError>(())
/// ```
pub fn default_varlink_socket(
    explicit: Option<&Path>,
    xdg_runtime_dir: Option<&Path>,
) -> Result<PathBuf, AppError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }

    let Some(runtime_dir) = xdg_runtime_dir else {
        return Err(AppError::MissingSocketPath);
    };

    Ok(runtime_dir.join(INTERFACE_NAME))
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parses_set_effect_custom_parameters() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from([
            "dp2",
            "set-effect",
            "--socket",
            "/tmp/dp2.sock",
            "custom",
            "depth-of-field",
            "--param",
            "aperture=2.8",
            "--param",
            "focus=1.5",
        ])?;

        let Command::SetEffect { socket, effect } = cli.command else {
            panic!("expected SetEffect command");
        };
        assert_eq!(socket.socket, Some(PathBuf::from("/tmp/dp2.sock")));
        assert_eq!(
            effect.into_effect_config(),
            EffectConfig::Custom {
                id: EffectId::new("depth-of-field"),
                parameters: BTreeMap::from([
                    ("aperture".into(), "2.8".into()),
                    ("focus".into(), "1.5".into()),
                ]),
            }
        );
        Ok(())
    }

    #[test]
    fn default_varlink_socket_requires_explicit_or_runtime_dir() {
        assert_matches!(
            default_varlink_socket(None, None),
            Err(AppError::MissingSocketPath)
        );
    }
}
