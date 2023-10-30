let
  pkgs = import <nixpkgs> {};
in
  pkgs.mkShell {
    buildInputs = with pkgs; [
        openssl
        alsa-lib
        libusb1
    ];
    nativeBuildInputs = with pkgs; [
        cmake
        pkg-config
        cargo-cross
    ];
  }
