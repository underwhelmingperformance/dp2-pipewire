# SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
#
# SPDX-License-Identifier: GPL-3.0-or-later
{
  inputs,
  lib,
  flake-parts-lib,
  ...
}: {
  options.perSystem = flake-parts-lib.mkPerSystemOption (_: let
    t = lib.types;
  in {
    options.dp2 = {
      rustToolchain = lib.mkOption {
        type = t.package;
        description = "Rust toolchain for building";
      };

      rustfmtNightly = lib.mkOption {
        type = t.package;
        description = "Nightly rustfmt for unstable options";
      };

      rustfmtBin = lib.mkOption {
        type = t.str;
        description = "Path to nightly rustfmt binary";
      };

      craneLib = lib.mkOption {
        type = t.raw;
        description = "Crane library configured with the toolchain";
      };

      commonArgs = lib.mkOption {
        type = t.attrsOf t.raw;
        description = "Shared Crane build arguments";
      };

      cargoArtifacts = lib.mkOption {
        type = t.package;
        description = "Pre-built Cargo dependencies for the release package";
      };

      checkArgs = lib.mkOption {
        type = t.attrsOf t.raw;
        description = "Crane build arguments for the checks";
      };

      checkArtifacts = lib.mkOption {
        type = t.package;
        description = "Pre-built Cargo dependencies for the checks";
      };

      wrapscallion = lib.mkOption {
        type = t.package;
        description = "Commit message linter";
      };

      devShellDigest = lib.mkOption {
        type = t.package;
        description = "Program that prints a digest of the files that define the development shell";
      };
    };
  });

  config.perSystem = {
    config,
    pkgs,
    system,
    ...
  }: let
    fenixPkgs = inputs.fenix.packages.${system};
    rustfmtNightly = fenixPkgs.latest.rustfmt;

    rustToolchain = fenixPkgs.stable.withComponents [
      "cargo"
      "clippy"
      "rust-analyzer"
      "rust-src"
      "rustc"
    ];

    craneLib = (inputs.crane.mkLib pkgs).overrideToolchain rustToolchain;

    linuxBuildInputs = lib.optionals pkgs.stdenv.hostPlatform.isLinux [
      pkgs.pipewire
    ];

    linuxNativeBuildInputs = lib.optionals pkgs.stdenv.hostPlatform.isLinux [
      pkgs.pkg-config
      pkgs.rustPlatform.bindgenHook
    ];
  in {
    dp2 = {
      inherit craneLib rustfmtNightly rustToolchain;

      commonArgs =
        {
          src = craneLib.cleanCargoSource inputs.self;
          strictDeps = true;
          cargoExtraArgs = "--locked --workspace";
          nativeBuildInputs = linuxNativeBuildInputs;
          buildInputs = linuxBuildInputs;
        }
        // lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
          NIX_CC_WRAPPER_SUPPRESS_TARGET_WARNING = 1;
        };

      cargoArtifacts = craneLib.buildDepsOnly config.dp2.commonArgs;

      checkArgs = config.dp2.commonArgs // {CARGO_PROFILE = "dev";};
      checkArtifacts = craneLib.buildDepsOnly config.dp2.checkArgs;

      rustfmtBin = lib.getExe' rustfmtNightly "rustfmt";

      wrapscallion = pkgs.callPackage ./wrapscallion/package.nix {};

      devShellDigest = pkgs.writeShellApplication {
        name = "dev-shell-digest";
        runtimeInputs = [pkgs.coreutils pkgs.findutils pkgs.git];
        text = ''
          cd "$(git rev-parse --show-toplevel)"
          git ls-files -z flake.nix flake.lock nix |
            LC_ALL=C sort -z |
            while IFS= read -r -d "" file; do
              printf '%s\n' "$file"
              cat "$file"
              printf '\n'
            done |
            sha256sum |
            cut -d ' ' -f 1
        '';
      };
    };
  };
}
