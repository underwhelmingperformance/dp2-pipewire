<!--
SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# dp2

`dp2` reads frames from an existing PipeWire video device on Linux, estimates
metric depth with [Depth Pro 2][depth-pro-2], applies a runtime-swappable
effect, and publishes the result as a new PipeWire virtual camera device. A
Varlink IPC service (`uk.org.orangesquash.dp2`) and a GNOME Shell top-bar
extension control effects, models, input cameras, looping video files, and a
live inline preview at runtime.

[depth-pro-2]: https://github.com/apple-aiml-research/ml-depth-pro-2

## Demo

Recorded in a NixOS GNOME 50 Wayland session running `dp2` via systemd user
socket activation with the Depth Pro 2 S ONNX model and a looping CC0 video
imported as a PipeWire camera device:

![GNOME Shell extension live demo switching depth palettes, models, and passthrough][demo-gif]

|               Depth Map (Turbo)               |               Depth Map (Magma)               |                     Passthrough                      |
| :-------------------------------------------: | :-------------------------------------------: | :--------------------------------------------------: |
| ![Turbo depth map in GNOME Shell][shot-turbo] | ![Magma depth map in GNOME Shell][shot-magma] | ![Passthrough feed in GNOME Shell][shot-passthrough] |

[demo-gif]: docs/demos/gnome-depth-map-demo.gif
[shot-turbo]: docs/demos/gnome-depth-turbo.png
[shot-magma]: docs/demos/gnome-depth-magma-stream.png
[shot-passthrough]: docs/demos/gnome-passthrough.png

## Architecture

The workspace separates domain logic from OS, ML, and IPC adapters through
dependency-injection traits:

- `crates/dp2-core`: video frame and depth map types, DI traits (`VideoSource`,
  `VideoSink`, `DepthEstimator`, `FrameEffect`, `EffectFactory`,
  `SourceFactory`, `ModelFactory`, `Clock`, `StateObserver`), built-in effects,
  preview downsampling, and the pipeline engine.
- `crates/dp2-depth`: Depth Pro 2 ONNX Runtime estimator supporting single-frame
  and temporally consistent streaming graphs, optional focal-length estimation,
  and a `ModelCatalog` for runtime model-size switching.
- `crates/dp2-pipewire`: Linux PipeWire capture source, virtual camera sink,
  PipeWire `Video/Source` discovery, and looping video file import via FFmpeg.
- `crates/dp2-varlink`: Varlink control server (with systemd `LISTEN_FDS` socket
  activation) and typed client for `uk.org.orangesquash.dp2`.
- `crates/dp2`: command-line daemon and Varlink control client.
- `extensions/dp2@orangesquash.org.uk`: GNOME Shell 47–50 top-bar extension with
  inline `St.ImageContent` preview, linked model and palette controls, input
  source switching, and video file import.

## Installing with Nix

Run directly from the flake:

```sh
nix run github:underwhelmingperformance/dp2 -- --help
```

Install declaratively on NixOS with systemd user socket activation and the GNOME
Shell extension:

```nix
{
  imports = [inputs.dp2.nixosModules.default];

  services.dp2 = {
    enable = true;
    installGnomeExtension = true;
  };
}
```

A matching `homeManagerModules.default` module is also exported by the flake.
When enabled, `dp2.socket` listens on `$XDG_RUNTIME_DIR/uk.org.orangesquash.dp2`
and starts `dp2.service` on demand when the GNOME Shell extension or CLI
connects.

## Running manually

Start the daemon directly against an existing PipeWire camera or in `--idle`
mode waiting for a source over Varlink:

```sh
dp2 run \
  --source my-camera \
  --sink-name dp2-depth \
  --model-dir "${HOME}/.local/share/dp2/models"
```

Inspect status, switch effects, change models, or import a looping video over
Varlink:

```sh
dp2 status
dp2 list-effects
dp2 set-effect depth-map --colormap viridis
dp2 set-effect passthrough
dp2 list-sources
dp2 import-video /path/to/clip.webm
dp2 list-models
dp2 set-model s-stream4
```

Standard `varlinkctl` works against the same socket:

```sh
varlinkctl introspect "${XDG_RUNTIME_DIR}/uk.org.orangesquash.dp2" uk.org.orangesquash.dp2
varlinkctl call "${XDG_RUNTIME_DIR}/uk.org.orangesquash.dp2" uk.org.orangesquash.dp2.GetStatus '{}'
```

## Development, testing, and recording demos

Run unit tests, clippy, rustdoc, licence checks, and Nix flake checks:

```sh
just check
```

Run the NixOS VM integration tests (systemd socket activation, PipeWire looping
video import, real Depth Pro 2 ONNX inference, Varlink hot-swapping, and the
GNOME Shell 50 extension):

```sh
just vm-test
```

Re-record and update the GNOME desktop screenshots and screencast in
`docs/demos/`:

```sh
just record-demo
```
