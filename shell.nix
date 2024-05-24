let

  nixpkgsPath =
    let
      rev = "269ce7215bb5b436546786e8d354d37903e102a8";
    in
      builtins.fetchTarball {
        url = "https://github.com/NixOS/nixpkgs/archive/${rev}.tar.gz";
        sha256 = "sha256:0lccy0kf2287hmhr38ws9fy1gyxm4wvxrkvca471i57nvfbpjlg0";
      };

  pkgs = import nixpkgsPath {};

  inherit (pkgs) lib;

  z3 =
    let
      arch = "x64";
      version = "4.12.5";
      filename = "z3-${version}-${arch}-glibc-2.35";
    in
      pkgs.stdenv.mkDerivation {
        name = "z3";

        src = pkgs.fetchurl {
          url = "https://github.com/Z3Prover/z3/releases/download/z3-${version}/${filename}.zip";
          sha256 = "sha256-8DZXTV4gKckgT/81A8/mjd9B+m/euzm+7ZnhvzVbf+4=";
        };

        nativeBuildInputs = with pkgs; [
          stdenv.cc.cc.lib
          autoPatchelfHook
          unzip
        ];

        dontConfigure = true;
        dontBuild = true;

        installPhase = ''
          here=$(pwd)
          cd $TMPDIR
          mv $here $out
        '';
      };

  pmdk =
    let
    in
      pkgs.stdenv.mkDerivation rec {
        pname = "pmdk";
        version = "1.11.1";

        src = pkgs.fetchFromGitHub {
          owner = "pmem";
          repo = "pmdk";
          rev = version;
          hash = "sha256-8bnyLtgkKfgIjJkfY/ZS1I9aCYcrz0nrdY7m/TUVWAk=";
        };

        nativeBuildInputs = with pkgs; [
          autoconf pkg-config gnum4 pandoc
        ];

        buildInputs = with pkgs; [
          libndctl
        ];

        enableParallelBuilding = true;

        patchPhase = ''
          patchShebangs utils
        '';

        NIX_CFLAGS_COMPILE = "-Wno-error";

        installPhase = ''
          make install prefix=$out
        '';
      };

in
with pkgs;

mkShell {
  RUSTC_BOOTSTRAP = 1;

  VERUS_Z3_PATH = "${z3}/bin/z3";
  VERUS_SINGULAR_PATH = "${pkgs.singular}/bin/Singular";

  nativeBuildInputs = [
    rustPlatform.bindgenHook
    rustup
  ];

  buildInputs = [
    pmdk
  ];

  shellHook =
    let
      m = toString ../verus-hacking/verus/source/Cargo.toml;
    in ''
      t() {
        cargo build --manifest-path ${m} \
          -p verus-driver --features singular \
            && cargo run  --manifest-path ${m} \
              -p cargo-verus -- "$@"
      }
    '';

  passthru = {
    inherit pkgs z3 pmdk;
  };
}
