<!--
SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# dp2

`dp2` reads frames from an existing PipeWire video device on Linux, estimates
metric depth with [Depth Pro 2][depth-pro-2], applies a runtime-swappable
effect, and publishes the result as a new PipeWire virtual camera device.

[depth-pro-2]: https://github.com/apple-aiml-research/ml-depth-pro-2

## Architecture

The workspace separates domain logic from OS, ML, and IPC adapters through
dependency injection traits:

- `crates/dp2-core`: video frame and depth map types, DI traits, built-in
  effects (`depth-map`, `passthrough`), effect registry, and pipeline engine.
- `crates/dp2-depth`: Depth Pro 2 ONNX Runtime estimator supporting single-frame
  and temporally consistent streaming graphs plus optional focal-length
  estimation.
- `crates/dp2-pipewire`: Linux PipeWire capture source and virtual camera sink
  streams.
- `crates/dp2-varlink`: Varlink control server and client for the
  `uk.org.orangesquash.dp2` interface, supporting runtime effect switching and
  streaming status updates.
- `crates/dp2`: command-line daemon and Varlink control client.

## Running

Start the daemon with an existing PipeWire video device and a Depth Pro 2 ONNX
model:

```sh
dp2 run \
  --source my-camera \
  --sink-name dp2-depth \
  --model models/model_stream4.onnx \
  --varlink-socket "${XDG_RUNTIME_DIR}/uk.org.orangesquash.dp2"
```

Inspect status or switch effects at runtime over Varlink:

```sh
dp2 status
dp2 list-effects
dp2 set-effect depth-map --colormap viridis
dp2 set-effect passthrough
```

Standard `varlinkctl` works against the same socket:

```sh
varlinkctl introspect "${XDG_RUNTIME_DIR}/uk.org.orangesquash.dp2" uk.org.orangesquash.dp2
varlinkctl call "${XDG_RUNTIME_DIR}/uk.org.orangesquash.dp2" uk.org.orangesquash.dp2.GetStatus '{}'
```

## Development and testing

Run unit tests, clippy, docs, and licence checks:

```sh
just check
```

Provision the OrbStack Linux VM and run end-to-end PipeWire + Depth Pro 2 +
Varlink integration tests:

```sh
just vm-setup
just vm-test
```
