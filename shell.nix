let
  pkgs = import <nixpkgs> {};
in
  pkgs.mkShell {
    buildInputs = with pkgs; [
        cmake
        openssl
    ];
    nativeBuildInputs = with pkgs; [
        pkg-config
    ];
  }
