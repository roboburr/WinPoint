# Win11 icon theme: origin and changes

The icon themes `Win11Shell` (folder `icons/Win11`) and `Win11Shell-dark` (folder `icons/Win11-dark`) that Win11 for GNOME installs are the Win11 icon theme by yeyushengfan258, commit `a5b460a407da143b32f19a503d7fcebb3edf2371` (<https://github.com/yeyushengfan258/Win11-icon-theme>). They are licensed under GPL-3.0-only: upstream ships the GPLv3 text as `COPYING` with no "or later" statement. Each theme folder carries upstream's `COPYING` and `AUTHORS` unchanged, and the unmodified upstream source archive is in `icon-source/`. `icon-bundle.json` records the upstream commit and the hash or link target of every file.

The MIT licence of Win11 for GNOME does not apply to these themes. They are a separate work that is only read at runtime.

## Changes to the icon theme

This is the notice of modification that GPLv3 section 5(a) asks for. Win11 for GNOME changed the Win11 icon theme of upstream commit `a5b460a4` as follows. The result was recorded in `vendor/icon-bundle.json`, which entered the Win11 for GNOME repository in baseline commit `3b898e62` of 2026-11-05 (work up to 2026-11-04).

- `vendor/icons/Win11-dark` was generated from the upstream sources with upstream's own `install.sh` dark transform: recoloured icons, dark trash icons and `Name=Win11-dark` in `index.theme`.
- `vendor/icons/Win11/preferences/22`: an absolute symbolic link that pointed back into the theme was replaced by the same target as a relative link (`32`).
- 296 entries were left out: 294 optional upstream aliases whose targets do not exist (147 per theme, for example `places/scalable/folder-green.svg`) and the 2 generated `icon-theme.cache` files.
- All other 16,839 regular files match upstream exactly (light variant) or the output of the documented transform (dark variant); `vendor/icon-bundle.json` has the per-file details.

When it installs the themes and `gtk-update-icon-cache` is available, Win11 for GNOME generates a fresh `icon-theme.cache` for each theme.

## Credits inside the icon theme

The upstream `AUTHORS` file names the Yaru icon theme as its source: <https://github.com/ubuntu/yaru>, by Sam Hewitt and Canonical. Yaru's icons are licensed CC-BY-SA-4.0. Upstream distributes its derived theme under GPL-3.0, which CC BY-SA 4.0 allows for adaptations.

The metadata of a few SVG files also names other icon themes: the GNOME Symbolic Icon Theme, the Adwaita Icon Template, the Paper Symbolic Icon Theme and deepin. Upstream does not credit these; they are named here as a courtesy.

The full overview of third-party material in Win11 for GNOME is in `THIRD_PARTY_NOTICES.md` of its source and release archive.
