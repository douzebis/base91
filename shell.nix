# SPDX-FileCopyrightText: 2025 2025 Frederic Ruget <fred@atlant.is>
#
# SPDX-License-Identifier: MIT

{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = [
    (pkgs.callPackage ./base91.nix {})
  ];
}
