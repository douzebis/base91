# SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
#
# SPDX-License-Identifier: MIT

# Entry point for `nix-shell` — delegates to the dev-shell output of default.nix.
(import ./default.nix {}).dev-shell
