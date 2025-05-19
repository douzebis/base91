# SPDX-FileCopyrightText: 2025 2025 Frederic Ruget <fred@atlant.is>
#
# SPDX-License-Identifier: MIT

let
  pkgs = import <nixpkgs> {};
in
pkgs.callPackage ./base91.nix {}
