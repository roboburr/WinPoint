# Third-party notices

Win11 for GNOME's own code and assets are licensed under the MIT licence in [LICENSE](LICENSE) (the same text is in [LICENSES/MIT.txt](LICENSES/MIT.txt)). That includes `vendor/ui/`, `vendor/host/` and `vendor/config/`: they are the author's own earlier work from the Win11 Shell project and fall under the same MIT licence, except the third-party parts listed below. This file lists the material in this repository and in the release archive that comes from other people or is not under MIT: where it is, where it comes from, its licence, and what that licence asks for. The full licence texts are in [LICENSES/](LICENSES/). The MIT licence of this project gives no rights to the material listed here; each part stays under its own licence.

This overview was made on 2026-11-13 from the files tracked in git at commit 07cb8014, without network access, and updated the same day for the author's decisions of 2026-11-13: the default wallpapers renamed to Lotus and dedicated under CC0, `vendor/` confirmed as own work, and the trademark material and the unused lock screen image removed. The evidence for each entry is a file in this repository or on an Ubuntu 26.04 system (`/usr/share/doc/<package>/copyright`, the local cargo registry, the Rust toolchain). Entries marked "per upstream" come from the licence declarations the upstream projects publish and could not be checked against their sources on this machine.

## Licence texts

| File | Licence | Needed for |
|---|---|---|
| [LICENSES/MIT.txt](LICENSES/MIT.txt) | MIT | Win11 for GNOME's own code and assets, `vendor/ui`, `vendor/host` and `vendor/config` included; Rust crates used under MIT |
| [LICENSES/GPL-3.0.txt](LICENSES/GPL-3.0.txt) | GPL-3.0 (byte-identical to the icon theme's own `COPYING`) | The Win11 icon theme (section 1) |
| [LICENSES/CC0-1.0.txt](LICENSES/CC0-1.0.txt) | CC0 1.0 Universal (copied from Ubuntu's `/usr/share/common-licenses/CC0-1.0`) | The Lotus wallpapers (section 1) |
| [LICENSES/PSF-2.0.txt](LICENSES/PSF-2.0.txt) | Python Software Foundation License Version 2 (copied from CPython 3.14's `LICENSE.txt`) | Two Rust files translated from CPython (section 2) |
| [LICENSES/Unicode-3.0.txt](LICENSES/Unicode-3.0.txt) | Unicode License v3 (the copy shipped with the ICU4X crates) | ICU4X crates and the Unicode tables of the Rust standard library (section 3) |
| [LICENSES/Apache-2.0.txt](LICENSES/Apache-2.0.txt) | Apache License 2.0 | The `dpi` crate, and `compiler_builtins` of the Rust standard library with the LLVM exception (section 3) |
| [LICENSES/MPL-2.0.txt](LICENSES/MPL-2.0.txt) | Mozilla Public License 2.0 | The `option-ext` crate (section 3) |
| [LICENSES/Zlib.txt](LICENSES/Zlib.txt) | zlib licence (the copy shipped with `foldhash`) | The `foldhash` crate (section 3) |
| [LICENSES/RUST-CRATES.md](LICENSES/RUST-CRATES.md) | (list) | Every Rust crate in the binaries, with its copyright notice |

## 1. Icons and other bundled assets

| Path | What | Origin | Licence | Obligation and how it is met |
|---|---|---|---|---|
| `vendor/icons/Win11/**` | Win11 icon theme, light variant (apps, actions, status, mimes, places, devices and more). `scripts/icon_bundle.py` installs it privately as the icon theme `Win11Shell`. | yeyushengfan258/Win11-icon-theme, commit `a5b460a407da143b32f19a503d7fcebb3edf2371` (<https://github.com/yeyushengfan258/Win11-icon-theme>). Recorded in `vendor/icon-bundle.json`. | GPL-3.0-only. Upstream ships the GPLv3 text as `COPYING` with no "or later" statement, so it is treated as version 3 only. | Licence text and authors ship in the theme folder (`COPYING`, `AUTHORS`, byte-identical to upstream) and in [LICENSES/GPL-3.0.txt](LICENSES/GPL-3.0.txt). The SVG files are their own source form, and the upstream source archive is included (below). Changes are listed under "Changes to the icon theme", and the same text ships with the installed themes as `ICON_PROVENANCE.md` (from [vendor/ICON_PROVENANCE.md](vendor/ICON_PROVENANCE.md), copied by `scripts/icon_bundle.py`). The theme is a separate work that is only read at runtime, so it sits next to the MIT code as an aggregate (GPLv3 section 5). |
| `vendor/icons/Win11-dark/**` | Dark variant of the same theme, installed as `Win11Shell-dark`. | Generated from the same upstream commit with upstream's own `install.sh` dark transform. | GPL-3.0-only | As above, with its own `COPYING` and `AUTHORS`. This is a modified version: see "Changes to the icon theme". |
| `vendor/icon-source/Win11-icon-theme-a5b460a407da143b32f19a503d7fcebb3edf2371.tar.gz` | The unmodified upstream source archive (9,208,319 bytes, sha256 `358479b3...981a`), checked by `scripts/icon_bundle.py`. | GitHub archive of the upstream commit above. | GPL-3.0-only (the whole archive, including `install.sh`) | This is the corresponding source. It contains upstream's `COPYING` and `AUTHORS`. |
| `vendor/ui/assets/icons/settings.svg` | The Settings icon of the interface. | `src/apps/scalable/preferences-system.svg` of the same upstream commit (byte-identical). | GPL-3.0-only | [vendor/ui/assets/icons/NOTICE.txt](vendor/ui/assets/icons/NOTICE.txt), `licenses/Win11-icon-theme.COPYING` and `licenses/Win11-icon-theme.AUTHORS` sit next to it and must stay with it. This is the only GPL file under `vendor/ui`; apart from it and the Lotus wallpapers (CC0, below), everything under `vendor/ui` is MIT. |
| `vendor/ui/assets/wallpapers/lotus-dark.svg`, `vendor/ui/assets/wallpapers/lotus-light.svg` | The default wallpapers (dark and light). Each SVG wraps an unchanged PNG. Called "Bloom" (`bloom-dark`, `bloom-light`) until 2026-11-13; they were renamed so the name does not refer to Microsoft's Windows 11 wallpaper of that name, and the old `builtin:bloom-*` settings still resolve to these files. | AI-generated: made with OpenAI ChatGPT image generation (gpt-image) on 2026-11-02 and supplied by the author. The embedded PNGs carry the signed C2PA manifest of that generation. Recorded in `vendor/ui/assets/wallpapers/SOURCES.json` (`aiGenerated: true`, generator, source and asset hashes). | CC0-1.0 | None. To the extent the author holds rights in them, they are dedicated to the public domain under CC0 1.0 ([LICENSES/CC0-1.0.txt](LICENSES/CC0-1.0.txt)). `SOURCES.json` keeps the AI-generated mark with them. |
| `vendor/icon-bundle.json` | Verification manifest of the two themes: hash or link target per entry, upstream commit, exclusions. | Own tooling. | MIT | None beyond [LICENSE](LICENSE). |

### Changes to the icon theme

This is the notice of modification that GPLv3 section 5(a) asks for. Win11 for GNOME changed the Win11 icon theme of upstream commit `a5b460a4` as follows. The result was recorded in `vendor/icon-bundle.json`, which entered this repository in baseline commit `3b898e62` of 2026-11-05 (work up to 2026-11-04).

- `vendor/icons/Win11-dark` was generated from the upstream sources with upstream's own `install.sh` dark transform: recoloured icons, dark trash icons and `Name=Win11-dark` in `index.theme`.
- `vendor/icons/Win11/preferences/22`: an absolute symbolic link that pointed back into the theme was replaced by the same target as a relative link (`32`).
- 296 entries were left out: 294 optional upstream aliases whose targets do not exist (147 per theme, for example `places/scalable/folder-green.svg`) and the 2 generated `icon-theme.cache` files.
- All other 16,839 regular files match upstream exactly (light variant) or the output of the documented transform (dark variant); `vendor/icon-bundle.json` has the per-file details.

### Credits inside the icon theme

The upstream `AUTHORS` file names the Yaru icon theme as its source: <https://github.com/ubuntu/yaru>, by Sam Hewitt and Canonical. Yaru's icons are licensed CC-BY-SA-4.0 (`/usr/share/doc/yaru-theme-icon/copyright`, `Files: icons/*`). Upstream distributes its derived theme under GPL-3.0, which CC BY-SA 4.0 allows for adaptations, and Win11 for GNOME receives it under that GPL-3.0.

The metadata of a few SVG files also names other icon themes: the GNOME Symbolic Icon Theme and the Adwaita Icon Template (Adwaita is "CC-BY-SA-3.0 or LGPL-3, and CC-BY-SA-4.0" per `/usr/share/doc/adwaita-icon-theme/copyright`), the Paper Symbolic Icon Theme and deepin (for example `vendor/icons/Win11/emblems/16/emblem-error.svg` and `vendor/icons/Win11/apps/scalable/transmission.svg`). Upstream does not credit these; they are named here as a courtesy. The Paper and deepin licences could not be checked offline (see the open points).

### Own assets, for clarity

These look like they could be third-party material but are Win11 for GNOME's own work under MIT: everything in `vendor/ui/`, `vendor/host/` and `vendor/config/` that this file does not list (the earlier Win11 Shell sources, which had no LICENSE file of their own); the procedural wallpapers `vendor/ui/assets/wallpapers/{aurora,dunes,flow-dark,flow-light,glow,sunrise}.svg`; the logo and avatar in `vendor/ui/assets/img/` (`logo.svg`, `logo-mono.svg`, `logo.png`, `avatar.svg`, `avatar.png`); the Start disc `extension/start.svg` and `logo-circle.png`; the caption glyphs `vendor/config/gtk/glyphs/w11-*-symbolic.svg`; the line icons in `vendor/ui/js/core/icons.js`, `vendor/ui/js/apps/settings/navicons.js`, `native/shell/src/icons.rs` and `native/calendar/src/glyphs.rs` ("Fluent-style" describes the look; they are original drawings, not Microsoft's Fluent UI icons or a Segoe icon font); the app icons of the development mock in `vendor/ui/js/mock/index.js` (own drawings; the former `chrome` and `store` icons were replaced by a neutral globe and a plain shopping bag). Screenshots such as `docs/screenshots/*.png`, `preview-*.png` and `tests/gnome/baseline/**/*.png` are own captures; the icons visible in them remain GPL-3.0.

No font files are included. The style sheets only name fonts (Segoe UI, Segoe UI Variable, Selawik, Cascadia, Inter, Noto Sans, Ubuntu Sans) that the user's system provides, with generic fallbacks.

## 2. Code and data taken from other projects

| Path | What | Origin | Licence | Obligation and how it is met |
|---|---|---|---|---|
| `host/src/apps/recent_datetime.rs` | Rust translation of CPython's ISO 8601 parsing in `datetime.fromisoformat` (separator search, calendar and week dates, times and UTC offsets) and of the local-time solver in `_mktime`. Compiled into the host programs. | CPython `Lib/_pydatetime.py` (Python 3.14), Python Software Foundation | PSF-2.0 | Keep the PSF copyright notice and licence and a summary of the changes with the source and the binaries: see the notice below and [LICENSES/PSF-2.0.txt](LICENSES/PSF-2.0.txt). |
| `host/src/files/search.rs`, functions `compile_glob` and `compile_class` | Rust translation of the bracket-class handling of CPython's `fnmatch._translate` (splitting ranges into chunks, dropping descending ranges). | CPython `Lib/fnmatch.py` (Python 3.14) | PSF-2.0 | As above. Both Rust files start with a one-line notice that points here and to [LICENSES/PSF-2.0.txt](LICENSES/PSF-2.0.txt). |
| `host/src/services/html-entities.json`, `host/src/services/tray/html-entities.json` | The 2,231 HTML named character references, identical to CPython's `html.entities.html5`. Embedded in the host programs to unescape notification and tray text. | WHATWG HTML Living Standard (named character references), by way of CPython | Factual data. CPython's copy is PSF-2.0; the WHATWG publishes the HTML Standard under CC-BY-4.0. | Probably not protected as such. Credited below to be safe. |

Notice for the CPython-derived parts:

```text
Parts of host/src/apps/recent_datetime.rs and host/src/files/search.rs are derived
from CPython (Lib/_pydatetime.py and Lib/fnmatch.py, Python 3.14).
Copyright (c) 2001 Python Software Foundation; All Rights Reserved
Used under the Python Software Foundation License Version 2, see LICENSES/PSF-2.0.txt.

Summary of the changes (PSF licence, section 3): translated from Python to Rust and
reduced to what the recent-files (XBEL) reader and the file search need; dates are
returned as milliseconds since the Unix epoch and glob patterns become match tokens
instead of Python objects and regular expressions.

The table in host/src/services/html-entities.json and
host/src/services/tray/html-entities.json holds the HTML named character references
of the WHATWG HTML Living Standard (https://html.spec.whatwg.org/), licensed under
CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/), as published in CPython's
html.entities module.
```

### Test fixtures copied from Ubuntu packages

These files are in the repository only (the release archive does not contain `tests/`). They are very short functional configuration or interface data, so no licence text is considered necessary; their origin is named here and in their own header comments or folder README.

| Path | Origin | Licence of the source package |
|---|---|---|
| `tests/update/fixtures/compat/system-apt/apt.conf.d/15update-stamp`, `99update-notifier` | Byte-identical to the files of `update-notifier-common` in `/etc/apt/apt.conf.d/` | LGPL-2.0-or-later (`/usr/share/doc/update-notifier-common/copyright`) |
| `tests/update/fixtures/options/schemas/com.ubuntu.update-notifier.gschema.xml`, `tests/update/fixtures/release/schemas/com.ubuntu.update-manager.gschema.xml`, `tests/update/fixtures/upgrade/schemas/org.gnome.shell.gschema.xml` and the `gschemas.compiled` built from them | One or two keys (name, type, default) of the schemas of update-notifier 3.207.2, update-manager 1:26.04 and GNOME Shell 50; the update-notifier excerpt also keeps that key's summary and description | update-notifier: LGPL-2.0-or-later; update-manager: GPL (version not stated); GNOME Shell: GPL-2.0-or-later |
| `tests/update/fixtures/release/distro-info/ubuntu.csv` | Edited excerpt of distro-info-data's `ubuntu.csv` | ISC (`/usr/share/doc/distro-info-data/copyright`) |
| `tests/update/fixtures/release/meta-release/*` | Format of Canonical's meta-release files, partly invented | Factual data, no licence stated |

`tests/update/fixtures/compat/system-apt/apt.conf.d/20packagekit` (and its line in `tests/update/fixtures/compat/recorded/apt-config-system.txt`) is written for the test in the shape of PackageKit's apt hook: the same two hook keys and the same D-Bus call `org.freedesktop.PackageKit.StateHasChanged`, but not a copy of PackageKit's file. It is own work under MIT.

### Behaviour and values used without copying code

No licence obligation applies to these; they are named for credit and so that nobody mistakes them for copied code.

- `native/calendar/src/gnome.rs` talks to GNOME Shell's `org.gnome.Shell.CalendarServer` with the same rules as GNOME Shell's `calendar.js` (GPL-2.0-or-later). It is an independent Rust implementation.
- `host/src/services/update/release.rs` and `host/data/release-upgrades-helper` read `/etc/update-manager/release-upgrades` and meta-release files the way update-manager and Python's `configparser` do, written independently.
- `days_from_civil` and `civil_from_days` in `native/shell/src/flyouts/calendar.rs`, `native/calendar/src/dates.rs` and `host/src/apps/recent_datetime.rs` follow Howard Hinnant's published civil-date algorithms.
- The Oklab conversion in `vendor/ui/js/core/theme.js`, `native/shell/src/theme.rs`, `native/shell/src/flyouts/quick.rs` and `native/calendar/src/theme.rs` uses Björn Ottosson's Oklab matrices.
- Colours, radii and type sizes in `vendor/ui/css/tokens.css`, `vendor/ui/css/components.css` and the native style sheets follow the WinUI 3 theme resources (microsoft/microsoft-ui-xaml, MIT). Only values are used.

## 3. Rust crates linked into the binaries

The Rust programs link their crates statically, so the licences of those crates apply to the binaries even though the release archive does not contain their source. The host programs (`Cargo.lock`, the 12 `win11-*` programs in `bin/` of the release archive) link 95 crates; the native programs `win11-shell` and `win11-calendar` (`native/Cargo.lock`) link 37. Together that is 116 distinct crate versions. Where a crate offers a choice, it is used under MIT.

| Licence used | Crates | Text | What it asks for binaries |
|---|---|---|---|
| MIT | 98 (46 MIT only, 50 MIT OR Apache-2.0, `raw-window-handle`, `memchr`), among them gtk-rs (`gtk`, `gtk4`, `glib`, `gio`, `cairo-rs`, `pango` ...), `webkit2gtk`, `javascriptcore-rs`, `soup3`, `wry`, `serde`, `serde_json`, `url`, `ureq`, `tiny_http`, `time` | [MIT permission notice](LICENSES/RUST-CRATES.md#mit-permission-notice) | Each crate's copyright notice plus the MIT permission notice |
| Unicode-3.0 | 15 ICU4X crates (`icu_*`, `litemap`, `tinystr`, `writeable`, `yoke`, `zerofrom`, `zerotrie`, `zerovec`, `potential_utf`) | [Unicode-3.0.txt](LICENSES/Unicode-3.0.txt) | The Unicode notice in the documentation that comes with the binaries |
| MPL-2.0 | `option-ext` 0.2.0, unmodified | [MPL-2.0.txt](LICENSES/MPL-2.0.txt) | The licence, and where its source is available (<https://crates.io/crates/option-ext/0.2.0>). File-level copyleft: it does not extend to Win11 for GNOME's code. |
| Apache-2.0 AND MIT | `dpi` 0.1.2 | [Apache-2.0.txt](LICENSES/Apache-2.0.txt) and the libm MIT notice | Both texts |
| Zlib | `foldhash` 0.2.0 | [Zlib.txt](LICENSES/Zlib.txt) | Nothing for binaries; credited anyway |
| MIT (and Unicode-3.0; Apache-2.0 WITH LLVM-exception for `compiler_builtins`) | The Rust standard library of rustc 1.97.1, linked into every program, with the crates built into it: among them `addr2line`, `adler2`, `miniz_oxide`, `object`, `memchr` 2.7.6 and `compiler_builtins`, whose licences are given per upstream | see [RUST-CRATES.md](LICENSES/RUST-CRATES.md#rust-standard-library) | The Rust project's notice and the Unicode notice; the LLVM exception needs no notice in binaries |

The full list, with version, declared licence, copyright notice and source of every crate, is in [LICENSES/RUST-CRATES.md](LICENSES/RUST-CRATES.md). Crates that are only used while building (proc macros, build scripts) or only on other platforms do not end up in the Linux binaries and need no notice.

This file, [LICENSES/](LICENSES/) and [LICENSES/RUST-CRATES.md](LICENSES/RUST-CRATES.md) have to travel with every archive that contains these binaries. `scripts/package.py` puts `LICENSE`, this file and `LICENSES/` in the release archive. The native programs are not in the release archive yet; their crates are listed so the notices are complete once they are.

## 4. System libraries and programs used at runtime (not included)

Win11 for GNOME does not include these. The programs link to them dynamically, or start or import them, from the user's own system. Because nothing of them is redistributed, no licence text has to ship. That changes if a future .deb, Flatpak, Snap or AppImage bundles any of them: then their licence texts (and for the LGPL libraries the source offer and relinking terms) must be added.

| Component | Used by | Licence (from `/usr/share/doc/<package>/copyright`) |
|---|---|---|
| GTK 3 (3.24.52), with Pango and ATK | host programs | LGPL-2.0-or-later and LGPL-2.1-or-later |
| WebKitGTK and JavaScriptCoreGTK 4.1 (2.52.6) | host programs | BSD-2-Clause, with LGPL-2.0-or-later and LGPL-2.1-or-later parts |
| libsoup 3 (3.6.6) | host programs | LGPL-2.0-or-later |
| GTK 4 (4.22.4), graphene (1.10.8) | `win11-shell`, `win11-calendar` | LGPL-2.0-or-later and LGPL-2.1-or-later; MIT |
| GLib, GIO, GObject (2.88.0), gdk-pixbuf (2.44.5), cairo (1.18.4) | host and native programs | LGPL-2.1-or-later; LGPL-2.0-or-later and LGPL-2.1-or-later; LGPL-2.1-only OR MPL-1.1 |
| zlib (1.3.1), glibc (2.43), libgcc_s | host and native programs | Zlib; LGPL-2.1-or-later; GPL-3.0-or-later WITH GCC-exception-3.1 |
| libarchive (loaded with `dlopen`), ffmpeg and ffprobe (started as programs, optional) | host programs (archives, video thumbnails) | not bundled; licences not reviewed here |
| GNOME Shell 50 and Mutter | `extension/` runs inside GNOME Shell | GPL-2.0-or-later. The extension's MIT licence is compatible with it. |
| python-apt (3.1.0) | `host/data/phasing.py`, run with the system `python3` | GPL-2.0-or-later |
| PyGObject (3.56.2), pycairo (1.27.0), gtk-layer-shell | `scripts/`, `vendor/host/`, `tests/` | LGPL-2.1-or-later; LGPL-2.1-only OR MPL-1.1; MIT |
| Pillow, PackageKit-glib, libnm, Node.js built-in modules | tests and development tools only | HPND; GPL-2.0-or-later and LGPL-2.1-or-later; not distributed |

The `-sys` crates of gtk-rs and webkit2gtk-rs only declare the C interfaces in Rust (MIT); the C libraries themselves are not included. As the gtk-rs `COPYRIGHT` file points out, distributing executables that link to these libraries can bring in LGPL terms; with dynamic linking to the user's own copies, as here, nothing of the libraries is redistributed.

## 5. Trademarks

Microsoft, Windows, Windows 11, Microsoft Edge, Microsoft Office, Outlook, Skype, Visual Studio Code and Segoe are trademarks of the Microsoft group of companies. Google Chrome, Firefox, Steam, 1Password and the other product names and logos that appear in the icon theme or in screenshots belong to their owners. Win11 for GNOME is an independent project and is not affiliated with, sponsored by or endorsed by Microsoft or any of these owners. Its Start button is its own design (a blue gradient disc), not the Windows logo. Before publication, the material that showed or imitated such marks was removed: the unused `windows-logo` glyph (four squares) in `native/shell/src/icons.rs`, the old screenshots `vendor/ui/assets/img/screenshots/*.jpg` that showed a Windows-style Start logo, and the `chrome` and `store` icons of the development mock, which are now a neutral globe and a plain shopping bag. `vendor/ui/assets/img/lockscreen.jpg`, which had no recorded origin and was not used, was removed as well.

## Open points

These do not block publication; they are kept here so they can be checked later.

1. The Paper and deepin icon themes named in some SVG metadata of the icon theme (section 1): their licences could not be verified offline. Upstream does not credit them.
2. The Rust standard library crates marked "per upstream" in [LICENSES/RUST-CRATES.md](LICENSES/RUST-CRATES.md#rust-standard-library): compare them with the `Cargo.toml` and licence files in the `rust-src` component of the toolchain used for a release, and add their exact copyright lines.
