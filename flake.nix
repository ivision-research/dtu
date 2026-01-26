# BUG: This currently messes up `dtu --version` outputting git info.

{
  description = "Android Device Testing Utilities (dtu)";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/25.11";

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

        jdk = pkgs.jdk17_headless;

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
        };

        cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
          doCheck = false;
          cargoExtraArgs = "--workspace --exclude dtu-py";
        });

        mkBin = name:
            craneLib.buildPackage (commonArgs // {
              inherit cargoArtifacts;
              preBuild = ''
              export DTU_GIT_REVISION=${gitRev}
              '';
              pname = "${name}-cli";
              installPhaseCommand = ''
              install -D ./target/release/${name} $out/bin/${name}
              '';
              cargoExtraArgs = "--bin ${name}";
              doCheck = false;
            });

        dtuFs = mkBin "dtu-fs";
        dtuDecompile = mkBin "dtu-decompile";
        dtuCli = mkBin "dtu";

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
            --replace-quiet "-fno-semantic-interposition" ""
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
            --replace-quiet "-Werror" ""
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
          gradle_9
          jadx
          awscli2
          smaliPkg
          apktoolPkg
          baksmaliPkg
          secilc
          vdexExtractor
          jdk
        ];

      in
      {
        packages = rec {
          inherit dtuFs dtuDecompile;
          dtu = dtuCli;
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
              install -D -m 550 "$1/bin/$2" "$out/bin/$2"
              wrapProgram "$out/bin/$2" \
                --set JAVA_HOME "${jdk}" \
                --set DTU_S3_CACHE "1" \
                --prefix PATH : "${lib.makeBinPath envInputs}"
            }

            doWrap "${dtuCli}" "dtu"
            '';
          };

        };

        apps.default = flake-utils.lib.mkApp {
          name = "dtu";
          drv = dtuCli;
        };

        devShells.default = let
        in pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            mdbook
            kotlin
            sqlite
            awscli2
          ];
        };
      });
}
