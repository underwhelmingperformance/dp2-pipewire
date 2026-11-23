# SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
#
# SPDX-License-Identifier: GPL-3.0-or-later
_: {
  perSystem = {
    config,
    pkgs,
    ...
  }: let
    inherit (config.dp2) craneLib commonArgs cargoArtifacts;
  in {
    packages.default = craneLib.buildPackage (commonArgs
      // {
        inherit cargoArtifacts;
        doCheck = false;
        meta = {
          description = "PipeWire virtual camera powered by Depth Pro 2 and Varlink";
          homepage = "https://github.com/underwhelmingperformance/dp2";
          license = pkgs.lib.licenses.gpl3Plus;
          platforms = pkgs.lib.platforms.unix;
        };
      });
  };
}
