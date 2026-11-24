// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::IsTerminal;

use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

/// Installs the global `tracing` subscriber writing to standard error.
///
/// Interactive terminals receive human-readable pretty output; non-interactive
/// streams receive structured JSON events.
///
/// # Examples
///
/// ```
/// dp2::init_tracing(Some("warn"));
/// ```
pub fn init_tracing(filter_directive: Option<&str>) {
    let filter = filter_directive.map_or_else(|| EnvFilter::new("info"), EnvFilter::new);

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_span_events(FmtSpan::CLOSE);

    if std::io::stderr().is_terminal() {
        let _ = builder.pretty().try_init();
    } else {
        let _ = builder.json().try_init();
    }
}
