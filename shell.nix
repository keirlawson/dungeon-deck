let
  pkgs = import <nixpkgs> {};
in
  pkgs.mkShell {
    buildInputs = with pkgs; [
        cmake
        openssl
        alsa-lib
        libusb1
    ];
    nativeBuildInputs = with pkgs; [
        pkg-config
    ];
  }
