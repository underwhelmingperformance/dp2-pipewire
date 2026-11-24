# SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
#
# SPDX-License-Identifier: GPL-3.0-or-later
{self, ...}: {
  flake = {
    nixosModules.default = {
      config,
      lib,
      pkgs,
      ...
    }: let
      cfg = config.services.dp2;
      pkg = self.packages.${pkgs.stdenv.hostPlatform.system}.dp2;
      extPkg = self.packages.${pkgs.stdenv.hostPlatform.system}.gnome-shell-extension;
      modelDirArg =
        if cfg.modelDir != null
        then ["--model-dir" (toString cfg.modelDir)]
        else ["--model-dir" "%h/.local/share/dp2/models"];
      execArgs = lib.escapeShellArgs (["run" "--idle"] ++ modelDirArg ++ cfg.extraArgs);
    in {
      options.services.dp2 = {
        enable = lib.mkEnableOption "Depth Pro 2 PipeWire virtual camera user service";

        package = lib.mkOption {
          type = lib.types.package;
          default = pkg;
          description = "The dp2 package to install and run.";
        };

        extensionPackage = lib.mkOption {
          type = lib.types.package;
          default = extPkg;
          description = "The GNOME Shell extension package for dp2.";
        };

        installGnomeExtension = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Whether to install the dp2 GNOME Shell extension system-wide.";
        };

        modelDir = lib.mkOption {
          type = lib.types.nullOr lib.types.path;
          default = null;
          description = "Directory containing Depth Pro 2 ONNX models. Defaults to ~/.local/share/dp2/models.";
        };

        extraArgs = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [];
          description = "Additional command-line arguments passed to `dp2 run`.";
        };
      };

      config = lib.mkIf cfg.enable {
        environment.systemPackages =
          [
            cfg.package
            pkgs.zenity
          ]
          ++ lib.optional cfg.installGnomeExtension cfg.extensionPackage;

        systemd.user.sockets.dp2 = {
          description = "Depth Pro 2 PipeWire Virtual Camera Varlink Socket";
          wantedBy = ["sockets.target"];
          socketConfig = {
            ListenStream = "%t/uk.org.orangesquash.dp2";
            SocketMode = "0600";
          };
        };

        systemd.user.services.dp2 = {
          description = "Depth Pro 2 PipeWire Virtual Camera Daemon";
          requires = ["dp2.socket"];
          after = ["dp2.socket" "pipewire.service" "wireplumber.service"];
          wants = ["pipewire.service"];
          serviceConfig = {
            Type = "simple";
            ExecStart = "${lib.getExe cfg.package} ${execArgs}";
            Restart = "on-failure";
            RestartSec = "2s";
          };
        };
      };
    };

    homeManagerModules.default = {
      config,
      lib,
      pkgs,
      ...
    }: let
      cfg = config.services.dp2;
      pkg = self.packages.${pkgs.stdenv.hostPlatform.system}.dp2;
      extPkg = self.packages.${pkgs.stdenv.hostPlatform.system}.gnome-shell-extension;
      modelDirArg =
        if cfg.modelDir != null
        then ["--model-dir" (toString cfg.modelDir)]
        else ["--model-dir" "%h/.local/share/dp2/models"];
      execArgs = lib.escapeShellArgs (["run" "--idle"] ++ modelDirArg ++ cfg.extraArgs);
    in {
      options.services.dp2 = {
        enable = lib.mkEnableOption "Depth Pro 2 PipeWire virtual camera user service";

        package = lib.mkOption {
          type = lib.types.package;
          default = pkg;
          description = "The dp2 package to install and run.";
        };

        extensionPackage = lib.mkOption {
          type = lib.types.package;
          default = extPkg;
          description = "The GNOME Shell extension package for dp2.";
        };

        installGnomeExtension = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Whether to install the dp2 GNOME Shell extension in XDG data.";
        };

        modelDir = lib.mkOption {
          type = lib.types.nullOr lib.types.path;
          default = null;
          description = "Directory containing Depth Pro 2 ONNX models. Defaults to ~/.local/share/dp2/models.";
        };

        extraArgs = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [];
          description = "Additional command-line arguments passed to `dp2 run`.";
        };
      };

      config = lib.mkIf cfg.enable {
        home.packages = [
          cfg.package
          pkgs.zenity
        ];

        xdg.dataFile."gnome-shell/extensions/dp2@orangesquash.org.uk" = lib.mkIf cfg.installGnomeExtension {
          source = "${cfg.extensionPackage}/share/gnome-shell/extensions/dp2@orangesquash.org.uk";
          recursive = true;
        };

        systemd.user.sockets.dp2 = {
          Unit.Description = "Depth Pro 2 PipeWire Virtual Camera Varlink Socket";
          Socket = {
            ListenStream = "%t/uk.org.orangesquash.dp2";
            SocketMode = "0600";
          };
          Install.WantedBy = ["sockets.target"];
        };

        systemd.user.services.dp2 = {
          Unit = {
            Description = "Depth Pro 2 PipeWire Virtual Camera Daemon";
            Requires = ["dp2.socket"];
            After = ["dp2.socket" "pipewire.service" "wireplumber.service"];
            Wants = ["pipewire.service"];
          };
          Service = {
            Type = "simple";
            ExecStart = "${lib.getExe cfg.package} ${execArgs}";
            Restart = "on-failure";
            RestartSec = "2s";
          };
        };
      };
    };
  };
}
