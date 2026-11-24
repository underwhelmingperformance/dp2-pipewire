# SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
#
# SPDX-License-Identifier: GPL-3.0-or-later
{self, ...}: {
  perSystem = {
    config,
    pkgs,
    ...
  }: let
    inherit (pkgs) lib;
    isLinux = pkgs.stdenv.hostPlatform.isLinux;

    modelStream4 = pkgs.fetchurl {
      name = "depthpro2-s-stream4.onnx";
      url = "https://huggingface.co/christianengel/DepthPro2-S-ONNX/resolve/main/onnx/model_stream4.onnx";
      hash = "sha256-kijf8FyIh1TdJMZ0h+zZDOOy+FhxGbn+ZyPQEjUeTkU=";
    };

    modelSingle = pkgs.fetchurl {
      name = "depthpro2-s.onnx";
      url = "https://huggingface.co/christianengel/DepthPro2-S-ONNX/resolve/main/onnx/model.onnx";
      hash = "sha256-hihlV45D9bIS8ANAD2HOahjr70Dj/hlhssR+QcwPXy8=";
    };

    testVideoFile = pkgs.fetchurl {
      name = "flower.webm";
      url = "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.webm";
      hash = "sha256-xvijSJUzlVmKmnO5urFnZDZBB5e86fOY9L4VMdbnbdo=";
    };

    testVideoDir = pkgs.runCommandLocal "dp2-test-videos" {} ''
      mkdir -p "$out"
      ln -s ${testVideoFile} "$out/flower.webm"
    '';

    testVideo = "${testVideoDir}/flower.webm";

    modelDir = pkgs.runCommandLocal "dp2-test-models" {} ''
      mkdir -p "$out"
      ln -s ${modelStream4} "$out/depthpro2-s-stream4.onnx"
      ln -s ${modelSingle} "$out/depthpro2-s.onnx"
    '';

    vmPipelineTest = pkgs.testers.runNixOSTest {
      name = "dp2-pipewire-pipeline";

      nodes.machine = {pkgs, ...}: {
        imports = [self.nixosModules.default];

        virtualisation = {
          memorySize = 4096;
          cores = 4;
        };

        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
          linger = true;
          extraGroups = ["audio" "video"];
        };

        services.pipewire = {
          enable = true;
          wireplumber.enable = true;
        };

        services.dp2 = {
          enable = true;
          package = config.packages.dp2;
          inherit modelDir;
        };

        environment.systemPackages = [
          pkgs.jq
          pkgs.pipewire
        ];
      };

      testScript = ''
        import base64
        import json

        def as_alice(cmd):
            return machine.succeed(
                f"su - alice -c 'export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; {cmd}'"
            )

        machine.wait_for_unit("multi-user.target")
        machine.wait_for_unit("user@1000.service")
        machine.wait_for_file("/run/user/1000/pipewire-0")
        machine.wait_for_file("/run/user/1000/uk.org.orangesquash.dp2")

        with subtest("systemd socket activation starts dp2 on first Varlink call"):
            state = as_alice("systemctl --user is-active dp2.service || true").strip()
            assert state == "inactive", f"expected dp2.service inactive before activation, got {state}"

            idl = as_alice("varlinkctl introspect /run/user/1000/uk.org.orangesquash.dp2 uk.org.orangesquash.dp2")
            assert "method GetStatus" in idl
            assert "method ImportVideo" in idl
            assert "method SetModel" in idl
            assert "method GetPreview" in idl

            state = as_alice("systemctl --user is-active dp2.service").strip()
            assert state == "active", f"expected dp2.service active after socket activation, got {state}"

        with subtest("model catalog exposes multiple Depth Pro 2 ONNX models"):
            models = json.loads(as_alice("dp2 list-models"))
            model_ids = sorted(m["id"] for m in models)
            assert model_ids == ["s", "s-stream4"], f"unexpected model catalog: {models}"

        with subtest("import looping video as PipeWire device and produce real depth map frames"):
            imported = json.loads(as_alice("dp2 import-video ${testVideo}"))
            assert imported["name"] == "dp2-video-flower", f"unexpected imported source: {imported}"

            sources = json.loads(as_alice("dp2 list-sources"))
            names = [s["name"] for s in sources]
            assert "dp2-video-flower" in names, f"imported video missing from PipeWire sources: {sources}"

            dump = as_alice("pw-dump")
            assert "dp2-video-flower" in dump
            assert "dp2-depth" in dump

            preview = None
            for _ in range(40):
                raw = json.loads(as_alice("dp2 preview --max-width 160"))
                if raw is not None:
                    preview = raw
                    break
                machine.sleep(1)
            assert preview is not None, "timed out waiting for Depth Pro 2 preview frame"
            assert preview["width"] == 160
            assert preview["height"] == 120
            rgba = base64.b64decode(preview["rgba_base64"])
            assert len(rgba) == 160 * 120 * 4
            assert len(set(rgba)) > 16, "expected non-flat Turbo depth map output"

        with subtest("hot-swap effects and models over Varlink at runtime"):
            as_alice("dp2 set-effect depth-map --colormap grayscale")
            saw_gray = False
            for _ in range(40):
                raw = json.loads(as_alice("dp2 preview --max-width 80"))
                if raw is not None:
                    px = base64.b64decode(raw["rgba_base64"])
                    if all(abs(px[i] - px[i + 1]) <= 1 and abs(px[i + 1] - px[i + 2]) <= 1 for i in range(0, len(px), 4)):
                        saw_gray = True
                        break
                machine.sleep(1)
            assert saw_gray, "did not observe grayscale depth map frame after Varlink hot-swap"

            swapped_model = json.loads(as_alice("dp2 set-model s"))
            assert swapped_model["model"] == "s"

            as_alice("dp2 set-effect passthrough")
            status = json.loads(as_alice("dp2 status"))
            assert status["active_effect"]["id"] == "passthrough"
            assert status["active_model"] == "s"
            assert status["frames_processed"] >= 2

        with subtest("clean Varlink shutdown keeps socket listening"):
            as_alice("dp2 shutdown")
            machine.wait_until_succeeds(
                "su - alice -c 'export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; [ \"$(systemctl --user is-active dp2.service || true)\" = \"inactive\" ]'"
            )
            sock_state = as_alice("systemctl --user is-active dp2.socket").strip()
            assert sock_state == "active"
      '';
    };

    vmGnomeExtensionTest = pkgs.testers.runNixOSTest {
      name = "dp2-gnome-extension";

      nodes.machine = {pkgs, ...}: {
        imports = [self.nixosModules.default];

        virtualisation = {
          memorySize = 6144;
          cores = 4;
          resolution = {
            x = 1280;
            y = 800;
          };
        };

        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
          extraGroups = ["audio" "video" "wheel"];
        };

        services.displayManager.gdm.enable = true;
        services.displayManager.autoLogin = {
          enable = true;
          user = "alice";
        };
        services.desktopManager.gnome.enable = true;

        services.pipewire = {
          enable = true;
          wireplumber.enable = true;
        };

        services.dp2 = {
          enable = true;
          package = config.packages.dp2;
          extensionPackage = config.packages.gnome-shell-extension;
          installGnomeExtension = true;
          inherit modelDir;
        };

        systemd.user.services."org.gnome.Shell@".serviceConfig.ExecStart = [
          ""
          "${pkgs.gnome-shell}/bin/gnome-shell --unsafe-mode"
        ];

        environment.systemPackages = [
          pkgs.jq
          pkgs.pipewire
        ];
      };

      testScript = ''
        import json
        import shlex

        def as_alice(cmd):
            env_cmd = f"export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; {cmd}"
            return machine.succeed(f"su - alice -c {shlex.quote(env_cmd)}")

        def shell_eval(js):
            out = as_alice(
                f"gdbus call --session -d org.gnome.Shell -o /org/gnome/Shell -m org.gnome.Shell.Eval {shlex.quote(js)}"
            ).strip()
            assert out.startswith("(true,"), f"GNOME Shell eval failed: {out}"
            return out

        machine.wait_for_unit("display-manager.service")
        machine.wait_for_file("/run/user/1000/wayland-0")
        machine.wait_for_unit("default.target", "alice")
        machine.wait_for_file("/run/user/1000/uk.org.orangesquash.dp2")

        machine.wait_until_succeeds(
            "su - alice -c 'export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; gdbus call --session -d org.gnome.Shell -o /org/gnome/Shell -m org.gnome.Shell.Eval Main.layoutManager._startingUp' | grep -q 'true,..false'"
        )
        machine.send_key("esc")

        with subtest("enable dp2 GNOME Shell extension without errors"):
            as_alice("gsettings set org.gnome.shell disable-user-extensions false")
            as_alice("gnome-extensions enable dp2@orangesquash.org.uk")
            machine.wait_until_succeeds(
                "su - alice -c 'export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; gnome-extensions info dp2@orangesquash.org.uk' | grep -q 'State: ACTIVE'"
            )
            shell_eval("Boolean(Main.panel.statusArea['dp2-camera'])")

        with subtest("extension connects via socket activation, imports looping video, and previews inline"):
            shell_eval("Main.panel.statusArea['dp2-camera'].menu.open(false); 'ok'")
            machine.wait_until_succeeds(
                "su - alice -c 'export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; [ \"$(systemctl --user is-active dp2.service)\" = \"active\" ]'"
            )

            shell_eval("Main.panel.statusArea['dp2-camera'].importVideoPath('${testVideo}'); 'ok'")

            machine.wait_until_succeeds(
                "su - alice -c 'export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; gdbus call --session -d org.gnome.Shell -o /org/gnome/Shell -m org.gnome.Shell.Eval \"Main.panel.statusArea[\\\"dp2-camera\\\"]._lastPreviewSeq > 0\"' | grep -q 'true,..true'",
                timeout=60,
            )
            machine.sleep(1)
            machine.screenshot("gnome-extension-depth-map")

            shell_eval("Main.panel.statusArea['dp2-camera'].selectEffect('depth-map', {colormap: 'viridis'}); 'ok'")
            machine.sleep(3)
            machine.screenshot("gnome-extension-viridis")

            status = json.loads(as_alice("dp2 status"))
            assert status["active_effect"]["colormap"] == "viridis"
            assert status["source_name"] == "dp2-video-flower"
      '';
    };
  in {
    checks = lib.optionalAttrs isLinux {
      vm-pipeline = vmPipelineTest;
      vm-gnome-extension = vmGnomeExtensionTest;
    };
  };
}
