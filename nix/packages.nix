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
    inherit (config.dp2) craneLib commonArgs cargoArtifacts;

    ortLibName =
      if pkgs.stdenv.hostPlatform.isDarwin
      then "libonnxruntime.dylib"
      else "libonnxruntime.so";

    runtimePath = lib.optionals pkgs.stdenv.hostPlatform.isLinux [
      pkgs.ffmpeg-headless
    ];

    dp2Unwrapped = craneLib.buildPackage (commonArgs
      // {
        inherit cargoArtifacts;
        doCheck = false;
        meta = {
          description = "PipeWire virtual camera powered by Depth Pro 2 and Varlink";
          homepage = "https://github.com/underwhelmingperformance/dp2";
          license = pkgs.lib.licenses.gpl3Plus;
          platforms = pkgs.lib.platforms.unix;
          mainProgram = "dp2";
        };
      });

    dp2Wrapped = pkgs.symlinkJoin {
      name = "dp2-${dp2Unwrapped.version}";
      paths = [dp2Unwrapped];
      nativeBuildInputs = [pkgs.makeWrapper];
      postBuild = ''
        wrapProgram "$out/bin/dp2" \
          --set-default ORT_DYLIB_PATH "${pkgs.onnxruntime}/lib/${ortLibName}" \
          ${lib.optionalString pkgs.stdenv.hostPlatform.isLinux "--prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [pkgs.pipewire]}"} \
          ${lib.optionalString (runtimePath != []) "--prefix PATH : ${lib.makeBinPath runtimePath}"}

        ${lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
          mkdir -p "$out/share/systemd/user"
          cat > "$out/share/systemd/user/dp2.socket" <<'EOF'
          [Unit]
          Description=Depth Pro 2 PipeWire Virtual Camera Varlink Socket
          PartOf=graphical-session.target

          [Socket]
          ListenStream=%t/uk.org.orangesquash.dp2
          SocketMode=0600

          [Install]
          WantedBy=sockets.target
          EOF

          cat > "$out/share/systemd/user/dp2.service" <<EOF
          [Unit]
          Description=Depth Pro 2 PipeWire Virtual Camera Daemon
          Requires=dp2.socket
          After=dp2.socket pipewire.service wireplumber.service
          Wants=pipewire.service

          [Service]
          Type=simple
          ExecStart=$out/bin/dp2 run --idle --model-dir %h/.local/share/dp2/models
          Restart=on-failure
          RestartSec=2s
          EOF
        ''}
      '';
      inherit (dp2Unwrapped) meta;
    };

    gnomeShellExtension = pkgs.stdenvNoCC.mkDerivation {
      pname = "gnome-shell-extension-dp2";
      version = "0.1.0";
      src = builtins.path {
        name = "dp2-gnome-shell-extension-src";
        path = ../extensions + "/dp2@orangesquash.org.uk";
      };

      installPhase = ''
        runHook preInstall
        mkdir -p "$out/share/gnome-shell/extensions/dp2@orangesquash.org.uk"
        cp -r ./* "$out/share/gnome-shell/extensions/dp2@orangesquash.org.uk/"
        runHook postInstall
      '';

      passthru.extensionUuid = "dp2@orangesquash.org.uk";

      meta = {
        description = "GNOME Shell top-bar extension for Depth Pro 2 virtual camera";
        homepage = "https://github.com/underwhelmingperformance/dp2";
        license = pkgs.lib.licenses.gpl3Plus;
        platforms = pkgs.lib.platforms.linux;
      };
    };
  in {
    packages = {
      default = dp2Wrapped;
      dp2 = dp2Wrapped;
      dp2-unwrapped = dp2Unwrapped;
      gnome-shell-extension = gnomeShellExtension;
    };

    apps.default = {
      type = "app";
      program = lib.getExe dp2Wrapped;
    };
  };
}
