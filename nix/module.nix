# NixOS module for Murmur speech-to-text
#
# Handles system-level configuration that the package wrapper cannot:
#   - udev rule for /dev/uinput (rdev grab() needs it for virtual input)
#
# Note: users must add themselves to the "input" group for evdev hotkey access.
#
# Usage in your flake:
#
#   inputs.murmur.url = "github:DailyXplorer/Murmur";
#
#   nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
#     modules = [
#       murmur.nixosModules.default
#       { programs.murmur.enable = true; }
#     ];
#   };
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.murmur;
in
{
  options.programs.murmur = {
    enable = lib.mkEnableOption "Murmur speech-to-text";

    package = lib.mkOption {
      type = lib.types.package;
      defaultText = lib.literalExpression "murmur.packages.\${system}.murmur";
      description = "The Murmur package to use.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    # rdev grab() creates virtual input devices via /dev/uinput.
    # Default permissions are crw------- root root — open it to the input group.
    services.udev.extraRules = ''
      KERNEL=="uinput", GROUP="input", MODE="0660"
    '';
  };
}
