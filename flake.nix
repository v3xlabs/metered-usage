{
  description = "metered-usage, self-hosted LLM usage tracking";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
  }: let
    systems = [
      "x86_64-linux"
      "aarch64-linux"
      "aarch64-darwin"
    ];

    forEachSystem = f:
      nixpkgs.lib.genAttrs systems (
        system:
          f (import nixpkgs {
            inherit system;
            overlays = [rust-overlay.overlays.default];
          })
      );
  in {
    formatter = forEachSystem (pkgs: pkgs.alejandra);

    packages = forEachSystem (pkgs: rec {
      metered-usage = pkgs.callPackage ./nix/package.nix {};
      default = metered-usage;
    });

    nixosModules.default = import ./nix/module.nix {inherit self;};

    devShells = forEachSystem (pkgs: {
      default = pkgs.mkShell {
        packages = [
          (pkgs.rust-bin.stable.latest.default.override {
            extensions = [
              "rust-src"
              "rust-analyzer"
            ];
          })
          pkgs.cargo-nextest
          pkgs.sqlite
          pkgs.alejandra
          pkgs.nodejs
          pkgs.pnpm
        ];
      };
    });
  };
}
