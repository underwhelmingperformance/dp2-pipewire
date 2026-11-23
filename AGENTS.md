<!--
SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# Agent rules

## Project

`dp2` reads video frames from an existing PipeWire video device on Linux, runs
Depth Pro 2 depth estimation on each frame, applies a runtime-swappable frame
effect (initially a depth-map visualiser), and exposes the processed feed as a
new PipeWire virtual camera device. A Varlink IPC service controls the active
effect and pipeline state at runtime.

Use British English for project-owned prose.

## Architecture

- `dp2-core` contains the domain model, dependency-injection traits
  (`VideoSource`, `VideoSink`, `DepthEstimator`, `FrameEffect`,
  `EffectRegistry`, `Clock`), built-in effects, and the pipeline engine. It has
  no OS, PipeWire, ONNX, or IPC dependencies, and all unit tests run without
  side effects.
- `dp2-depth` implements `DepthEstimator` with Depth Pro 2 ONNX models via `ort`
  dynamic loading. Preprocessing and postprocessing are pure functions with unit
  tests that require no model files.
- `dp2-pipewire` implements `VideoSource` and `VideoSink` on Linux via
  `pipewire`. Non-Linux platforms compile the shared types and format helpers
  and return `PipeWireError::UnsupportedPlatform` at runtime.
- `dp2-varlink` exposes the `uk.org.orangesquash.dp2` Varlink interface via
  `zlink` over a Unix domain socket, plus a typed client proxy.
- `dp2` is the CLI binary that parses arguments with `clap`, configures
  `tracing` observability, and wires the adapters together.

## Licensing and provenance

- Every file starts with `SPDX-FileCopyrightText` and `SPDX-License-Identifier`
  headers following REUSE 3.0, or is listed in `REUSE.toml`.
- Generated files (`Cargo.lock`, `flake.lock`, `nix/wrapscallion/sources.json`)
  are listed in `REUSE.toml` with `SPDX-FileCopyrightText = "NONE"` and
  `CC0-1.0`.

## Rust style

- `lib.rs`, `mod.rs`, and re-export files contain only module declarations and
  re-exports.
- Clippy is strict via `[workspace.lints]`. Fix what a lint reports, or use
  `#[expect(lint, reason = "...")]` when a lint genuinely does not apply.
- `missing_docs` applies across the workspace. Every public item has a doc
  comment and every public function or method has a doctest.
- Declare every dependency once in `[workspace.dependencies]` and reference it
  with `workspace = true` in member crates.
- Use `thiserror` with `#[from]` for error types.

## Testing

- Unit tests use deterministic fakes and spies with zero side effects.
- Use `pretty_assertions::assert_eq!`, `rstest`, and `assert_matches!`, and
  assert on whole values rather than individual fields.
- Never call `unwrap()` in tests; use `?` or `expect()` with a useful message.
- Integration tests run inside the Linux VM against an isolated PipeWire daemon,
  a video-backed PipeWire source device, a real Depth Pro 2 ONNX model, and the
  live Varlink control socket.
