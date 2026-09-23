{self}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.metered-usage;
  format = pkgs.formats.toml {};
in {
  options.services.metered-usage = {
    enable = lib.mkEnableOption "metered-usage";

    package = lib.mkPackageOption self.packages.${pkgs.stdenv.hostPlatform.system} "metered-usage" {};

    host = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "Address the HTTP server binds to.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 3000;
      description = "Port the HTTP server binds to.";
    };

    environmentFile = lib.mkOption {
      type = lib.types.path;
      description = ''
        File of KEY=value lines read by systemd when the unit starts, so secrets never enter
        the Nix store. It must set METERED_USAGE_TOKEN, the shared token that protects the
        API, the MCP endpoint and the web UI, and every variable a source names in `key_env`.
      '';
    };

    settings = lib.mkOption {
      inherit (format) type;
      default = {};
      example = lib.literalExpression ''
        {
          source = [
            {
              key = "watch";
              name = "watch cliproxy";
              kind = "cliproxy";
              base_url = "http://127.0.0.1:8317";
              key_env = "CLIPROXY_MANAGEMENT_KEY";
            }
          ];
        }
      '';
      description = ''
        The metered-usage config file as Nix. It lands in the Nix store, so it names the
        environment variable holding each key and never holds a key itself.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.metered-usage = {
      description = "metered-usage LLM usage tracker";
      wantedBy = ["multi-user.target"];
      after = ["network-online.target"];
      wants = ["network-online.target"];

      environment = {
        DATABASE_URL = "sqlite:/var/lib/metered-usage/metered-usage.db";
        BIND_ADDRESS = "${cfg.host}:${toString cfg.port}";
        METERED_USAGE_CONFIG = format.generate "metered-usage.toml" cfg.settings;
      };

      serviceConfig = {
        ExecStart = lib.getExe cfg.package;
        EnvironmentFile = cfg.environmentFile;
        DynamicUser = true;
        StateDirectory = "metered-usage";
        Restart = "on-failure";
        RestartSec = 5;
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectHome = true;
        ProtectSystem = "strict";
      };
    };
  };
}
