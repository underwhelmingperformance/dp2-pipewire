// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Application wiring and command-line interface for `dp2`.

mod app;
mod cli;
mod error;
mod observability;

pub use app::{entry, run_command, run_pipeline_with_varlink};
pub use cli::{
    Cli, ColormapArg, Command, CustomEffectArgs, DepthMapEffectArgs, EffectPresetArg,
    PixelFormatArg, RunArgs, SetEffectCommand, SocketArgs, default_varlink_socket,
};
pub use error::AppError;
pub use observability::init_tracing;
