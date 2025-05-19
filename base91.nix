#let
#  pkgs = import <nixpkgs> {};
#in
#pkgs.stdenv.mkDerivation rec {
{ stdenv, gcc, lib, ... }:

stdenv.mkDerivation {
  pname = "base91";
  version = "0.1.0";

  src = ./.;

  buildInputs = [ gcc ];

  buildPhase = ''
    make -C src all
  '';

  installPhase = ''
    make -C src install prefix=$out
  '';

  meta = with lib; {
    description = "Base91 CLI tool";
    license = licenses.mit;
    # In // update maintainers/maintainer-list.nix with:
    # douzebis = {
    #   email = "fred@atlant.is";
    #   github = "douzebis";
    #   name = "Frédéric Ruget";
    # };
    maintainers = with maintainers; [ maintainers.douzebis ];
    platforms = platforms.unix;
  };
}
