# SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
#
# SPDX-License-Identifier: GPL-3.0-or-later
_: {
  perSystem = {
    config,
    pkgs,
    ...
  }: let
    inherit (pkgs) lib;
    inherit (config.dp2) rustToolchain rustfmtNightly rustfmtBin;

    shellFiles = let
      root = toString ../.;
      relative = path: lib.removePrefix "${root}/" (toString path);
    in
      lib.sort (a: b: a.name < b.name) (map (path: {
        name = relative path;
        inherit path;
      }) ([../flake.nix ../flake.lock] ++ lib.filesystem.listFilesRecursive ./.));

    ortLibName =
      if pkgs.stdenv.hostPlatform.isDarwin
      then "libonnxruntime.dylib"
      else "libonnxruntime.so";
  in {
    devShells.default = config.pre-commit.devShell.overrideAttrs (previous:
      {
        nativeBuildInputs =
          previous.nativeBuildInputs
          ++ [
            rustToolchain
            rustfmtNightly
            pkgs.cargo-audit
            pkgs.cargo-deny
            pkgs.just
            pkgs.onnxruntime
            pkgs.reuse
          ]
          ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
            pkgs.pipewire
            pkgs.pkg-config
            pkgs.rustPlatform.bindgenHook
          ];

        RUSTFMT = rustfmtBin;
        ORT_DYLIB_PATH = "${pkgs.onnxruntime}/lib/${ortLibName}";

        DP2_DEV_SHELL = builtins.hashString "sha256" (lib.concatMapStrings
          (file: "${file.name}\n${builtins.readFile file.path}\n")
          shellFiles);

        shellHook =
          previous.shellHook
          + ''
            ${lib.getExe pkgs.git} config --local core.hooksPath \
              "$(${lib.getExe pkgs.git} rev-parse --path-format=absolute --git-common-dir)/hooks"
          '';
      }
      // lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
        NIX_CC_WRAPPER_SUPPRESS_TARGET_WARNING = 1;
      });
  };
}
