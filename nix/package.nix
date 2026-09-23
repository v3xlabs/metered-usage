{
  lib,
  rustPlatform,
  stdenvNoCC,
  fetchPnpmDeps,
  pnpmConfigHook,
  pnpm,
  nodejs,
  cacert,
}: let
  version = (lib.importTOML ../Cargo.toml).package.version;

  web = stdenvNoCC.mkDerivation (finalAttrs: {
    pname = "metered-usage-web";
    inherit version;

    src = lib.fileset.toSource {
      root = ../web;
      fileset = lib.fileset.difference ../web (lib.fileset.unions [
        (lib.fileset.maybeMissing ../web/node_modules)
        (lib.fileset.maybeMissing ../web/dist)
      ]);
    };

    pnpmDeps = fetchPnpmDeps {
      inherit (finalAttrs) pname version src;
      inherit pnpm;
      fetcherVersion = 4;
      hash = "sha256-IG4qqIXpS35UI8+zQCivENu/nsvd15ncjaTiaGbIglc=";
    };

    nativeBuildInputs = [
      nodejs
      pnpm
      pnpmConfigHook
    ];

    buildPhase = ''
      runHook preBuild
      pnpm build
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      cp -r dist $out
      runHook postInstall
    '';
  });
in
  rustPlatform.buildRustPackage {
    pname = "metered-usage";
    inherit version;

    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        ../Cargo.toml
        ../Cargo.lock
        ../clippy.toml
        ../migrations
        ../src
        ../tests
      ];
    };

    cargoLock.lockFile = ../Cargo.lock;

    # rust-embed reads web/dist at compile time, so the built frontend has to be in the tree first.
    preBuild = ''
      mkdir -p web
      cp -r ${web} web/dist
    '';

    # The build sandbox has no certificate store, and building an HTTP client needs one even
    # when every request the tests make is plain HTTP to localhost.
    preCheck = ''
      export SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt
    '';

    passthru = {inherit web;};

    meta = {
      description = "Self-hosted usage tracking for OpenAI-compatible LLM gateways";
      homepage = "https://github.com/v3xlabs/metered-usage";
      mainProgram = "metered-usage";
      platforms = lib.platforms.linux ++ lib.platforms.darwin;
    };
  }
