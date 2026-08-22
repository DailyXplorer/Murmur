# Home-manager module for Murmur speech-to-text
#
# Provides a systemd user service for autostart.
# Usage: imports = [ murmur.homeManagerModules.default ];
#        services.murmur.enable = true;
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.murmur;
in
{
  options.services.murmur = {
    enable = lib.mkEnableOption "Murmur speech-to-text user service";

    package = lib.mkOption {
      type = lib.types.package;
      defaultText = lib.literalExpression "murmur.packages.\${system}.murmur";
      description = "The Murmur package to use.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.murmur = {
      Unit = {
        Description = "Murmur speech-to-text";
        After = [ "graphical-session.target" ];
        PartOf = [ "graphical-session.target" ];
      };
      Service = {
        ExecStart = "${cfg.package}/bin/murmur";
        Restart = "on-failure";
        RestartSec = 5;
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
