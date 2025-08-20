# BUG: This currently messes up `dtu --version` outputting git info.

{
  description = "Android Device Testing Utilities (dtu)";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/25.05";

    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
      };
    };

    apktool = {
      url = "https://bitbucket.org/iBotPeaches/apktool/downloads/apktool_2.12.0.jar";
      flake = false;
    };

    smali = {
      url = "file+https://dtu-public-nix-xdapadghy1vsvipd2t50hfvg56fv.s3.us-west-2.amazonaws.com/smali-3.0.7-5320adf0-dirty-fat.jar";
      flake = false;
    };

    baksmali = {
      url = "file+https://dtu-public-nix-xdapadghy1vsvipd2t50hfvg56fv.s3.us-west-2.amazonaws.com/baksmali-3.0.7-5320adf0-dirty-fat.jar";
      flake = false;
    };

    selinux = {
      url = "github:SELinuxProject/selinux";
      flake = false;
    };

    vdexExtractorSrc = {
      url = "github:anestisb/vdexExtractor";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, rust-overlay, smali, baksmali, selinux, vdexExtractorSrc, apktool, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        inherit (pkgs) lib;

        jdk = pkgs.jdk24_headless;

        rustPkg = pkgs.rust-bin.stable.latest.minimal;

        craneLib = (crane.mkLib pkgs).overrideToolchain rustPkg;
        gitRev = self.rev or self.dirtyRev;

        src = lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
                (lib.hasInfix "/migrations/" path) ||
                (lib.hasInfix "/templates/" path) ||
                (lib.hasInfix "/app/files/" path) ||
                (craneLib.filterCargoSources path type)
            ;
        };

        commonArgs = {
          inherit src;
          pname = "dtu";
          strictDeps = true;
          doCheck = false;

          NIX_OUTPATH_USED_AS_RANDOM_SEED = "aaaaaaaaaa";

          buildInputs = [
            pkgs.sqlite
          ] ++ lib.optionals(pkgs.stdenv.isDarwin) [
            pkgs.darwin.apple_sdk.frameworks.Security
          ];

          # TODO: I don't actually understand why I need clang here. There was
          # an error building zstd-sys for cozo and these things seemed to fix
          # it.
          nativeBuildInputs = [
            pkgs.clang
            pkgs.libclang.lib
            pkgs.pkg-config
          ];
          preConfigure = ''
          export LIBCLANG_PATH="${pkgs.libclang.lib}/lib/libclang.so"
          '';
        };

        cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
          doCheck = false;
        });

        libDtuDrv = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          pname = "dtu-lib";
          cargoExtraArgs = "--libs";
        });

        dtuCli = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          preBuild = ''
          export DTU_GIT_REVISION=${gitRev}
          '';
          pname = "dtu-cli";
          installPhaseCommand = ''
          install -D ./target/release/dtu $out/bin/dtu
          install -D ./target/release/dtu-decompile $out/bin/dtu-decompile
          install -D ./target/release/dtu-decompile $out/bin/dtu-fs
          '';
          cargoExtraArgs = "-F decompile-bin --bin dtu --bin dtu-decompile --bin dtu-fs";
          doCheck = false;
        });

        dtuCliNeo4j = dtuCli.overrideAttrs (prev: {
          cargoExtraArgs = prev ++ " -F neo4j";
        });

        jarInstall = prog: src:
          ''
            install -D ${src} "$out/libexec/${prog}/${prog}.jar"
            mkdir -p "$out/bin"
            makeWrapper "${jdk}/bin/java" "$out/bin/${prog}" \
                --add-flags "-jar $out/libexec/${prog}/${prog}.jar" \
          '';

        jarCommon = {
          dontUnpack = true;
          dontConfigure = true;
          dontBuild = true;
          dontPatch = true;
          nativeBuildInputs = [ pkgs.makeWrapper ];
          sourceRoot = ".";
        };

        apktoolPkg = pkgs.stdenv.mkDerivation (jarCommon // rec {
          pname = "apktool";
          version = "2.12.0";
          src = apktool;
          installPhase = jarInstall pname src;
        });

        smaliPkg = pkgs.stdenv.mkDerivation (jarCommon // rec {
          pname = "smali";
          version = "2.5.2";
          src = smali;
          installPhase = jarInstall pname src;
        });

        baksmaliPkg = pkgs.stdenv.mkDerivation (jarCommon // rec {
          pname = "baksmali";
          version = "2.5.2";
          src = baksmali;
          installPhase = jarInstall pname src;
        });

        sepol = pkgs.stdenv.mkDerivation {
          name = "libsepol";
          src = selinux;
          nativeBuildInputs = with pkgs; [
            flex
            bison
            pcre2
          ];
          # Needed for Darwin
          postPatch = ''
          substituteInPlace ./libsepol/src/Makefile \
            --replace "-fno-semantic-interposition" ""
          '';
          buildPhase = ''
          make -C libsepol/src libsepol.so.2
          '';
          installPhase = ''
          mkdir -p "$out/lib"
          install -m 550 -t "$out/lib" ./libsepol/src/libsepol.so.2 ./libsepol/src/libsepol.so
          make -C libsepol/include "PREFIX=$out" install
          '';
        };

        secilc = pkgs.stdenv.mkDerivation {
          name = "secilc";
          src = selinux;
          nativeBuildInputs = [
            sepol
          ];
          buildPhase = ''
          cd secilc && make secilc
          '';
          installPhase = ''
          mkdir -p "$out/bin"
          install -m 550 -t "$out/bin" ./secilc
          '';
        };


        vdexExtractor = pkgs.stdenv.mkDerivation {
          name = "vdexExtractor";
          src = vdexExtractorSrc;
          buildInputs = with pkgs; [
            zlib
          ];

          postPatch = ''
          substituteInPlace src/Makefile \
            --replace "-Werror" ""
          '';

          buildPhase = ''
          cd src && make
          '';

          installPhase = ''
          install -D -m 550 -t "$out/bin" ../bin/vdexExtractor
          '';
        };

        envInputs = with pkgs; [
          android-tools
          gradle
          jadx
          awscli2
          smaliPkg
          apktoolPkg
          baksmaliPkg
          secilc
          vdexExtractor
        ];

      in
      {
        packages = rec {
          dtu = dtuCli;
          dtuNeo4j = dtuCliNeo4j;
          libDtu = libDtuDrv;
          default = dtu;

          # Provide a consistent environment
          dtuEnv = pkgs.stdenv.mkDerivation {
            name = "dtu";

            nativeBuildInputs = [
              pkgs.makeBinaryWrapper
            ];

            sourceRoot = ".";
            dontConfigure = true;
            dontUnpack = true;
            dontBuild = true;
            installPhase = ''
            doWrap() {
              install -D -m 550 "${dtuCli}/bin/$1" "$out/bin/$1"
              wrapProgram "$out/bin/$1" \
                --set JAVA_HOME "${jdk}" \
                --set DTU_S3_CACHE "1" \
                --prefix PATH : "${lib.makeBinPath envInputs}"
            }

            doWrap "dtu"
            doWrap "dtu-fs"
            doWrap "dtu-decompile"
            '';
          };

          dtuEnvNeo4j = pkgs.stdenv.mkDerivation {
            name = "dtu";

            nativeBuildInputs = [
              pkgs.makeBinaryWrapper
            ];

            sourceRoot = ".";
            dontConfigure = true;
            dontUnpack = true;
            dontBuild = true;
            installPhase = ''
            doWrap() {
              install -D -m 550 "${dtuCliNeo4j}/bin/$1" "$out/bin/$1"
              wrapProgram "$out/bin/$1" \
                --prefix PATH : "${lib.makeBinPath envInputs}"
            }

            doWrap "dtu"
            doWrap "dtu-fs"
            doWrap "dtu-decompile"
            '';
          };

        };

        apps.default = flake-utils.lib.mkApp {
          name = "dtu";
          drv = dtuCli;
        };

        devShells.default = let

        neo4jConf = pkgs.writeTextFile {
          name = "neo4j.conf";
          text = ''
            dbms.security.auth_enabled=false
            server.directories.data=/tmp/dtu_neo4j/data
            server.directories.plugins=/tmp/dtu_neo4j/plugins
            server.directories.logs=/tmp/dtu_neo4j/logs
            server.directories.lib=/tmp/dtu_neo4j/lib
            server.directories.run=/tmp/dtu_neo4j/run
            server.directories.import=/tmp/dtu_neo4j/import
            server.bolt.enabled=true
            server.http.enabled=true
            server.jvm.additional=-XX:+UseG1GC
            server.jvm.additional=-XX:-OmitStackTraceInFastThrow
            server.jvm.additional=-XX:+AlwaysPreTouch
            server.jvm.additional=-XX:+UnlockExperimentalVMOptions
            server.jvm.additional=-XX:+TrustFinalNonStaticFields
            server.jvm.additional=-XX:+DisableExplicitGC
            server.jvm.additional=-Djdk.nio.maxCachedBufferSize=1024
            server.jvm.additional=-Dio.netty.tryReflectionSetAccessible=true
            server.jvm.additional=-Djdk.tls.ephemeralDHKeySize=2048
            server.jvm.additional=-Djdk.tls.rejectClientInitiatedRenegotiation=true
            server.jvm.additional=-XX:FlightRecorderOptions=stackdepth=256
            server.jvm.additional=-XX:+UnlockDiagnosticVMOptions
            server.jvm.additional=-XX:+DebugNonSafepoints
            server.jvm.additional=--add-opens=java.base/java.nio=ALL-UNNAMED
            server.jvm.additional=--add-opens=java.base/java.io=ALL-UNNAMED
            server.jvm.additional=--add-opens=java.base/sun.nio.ch=ALL-UNNAMED
            server.jvm.additional=-Dlog4j2.disable.jmx=true
            server.windows_service_name=neo4j
          '';
        };

        in pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            mdbook
            kotlin
            sqlite
            neo4j
            awscli2
          ];

          shellHook = ''
          if [ -d "/tmp/dtu_neo4j" ]; then
            rm -r "/tmp/dtu_neo4j/data"
            rm -r "/tmp/dtu_neo4j/import"
          fi

          mkdir -p /tmp/dtu_neo4j/import
          mkdir -p /tmp/dtu_neo4j/plugins

          cp ${neo4jConf} /tmp/dtu_neo4j/neo4j.conf
          export NEO4J_CONF=/tmp/dtu_neo4j

          local_path=''${XDG_DATA_HOME:-$HOME/.local/share}/dtu
          plugin_path=$local_path/neo4j/plugins

          apoc=apoc-5.18.0-core.jar

          if [ ! -f "/tmp/dtu_neo4j/plugins/$apoc" ]; then

            if [ -f "$plugin_path/$apoc" ]; then
              cp "$plugin_path/$apoc" "/tmp/dtu_neo4j/plugins/$apoc"
            fi


          fi
          '';
        };
      });
}
