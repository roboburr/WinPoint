# Rust crates in the binaries

This is the list of third-party Rust crates that are compiled into the Win11 for GNOME programs, with the licence each one is used under and its copyright notice. It belongs to [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md); the licence texts are in this folder.

How it was made (2026-11-13, commit 07cb8014, offline):

- The crate set comes from `cargo tree --frozen -e normal,no-proc-macro --target x86_64-unknown-linux-gnu`, run on `Cargo.lock` (the host workspace: the 12 `win11-*` programs in `bin/` of the release archive) and on `native/Cargo.lock` (`win11-shell` and `win11-calendar`).
- The licence of each crate comes from the `license` field of its `Cargo.toml`. The copyright notices are the `Copyright` lines of the licence files inside the crate, as published on crates.io (read from the local cargo registry). Where those files name nobody, the `authors` field of `Cargo.toml` is given instead.
- Not listed, because none of their code ends up in a Linux binary: crates used only while building (proc macros and build scripts: 38 in `Cargo.lock`, 26 in `native/Cargo.lock`) and crates only used on other platforms (108 and 5; among them the MPL-2.0 crates `cssparser`, `cssparser-macros`, `selectors` and `dtoa-short`, which `wry` needs only on Android).
- Where a crate offers a choice of licences, Win11 for GNOME uses it under MIT.

When these crates change, run the same `cargo tree` command again and update this file.

## Summary

| Used under | Crates | Licence text | What it asks when binaries are distributed |
|---|---|---|---|
| MIT | 98: 46 MIT only, 50 "MIT OR Apache-2.0" (in either order), `raw-window-handle` ("MIT OR Apache-2.0 OR Zlib") and `memchr` ("Unlicense OR MIT") | [MIT permission notice](#mit-permission-notice) below | Keep each crate's copyright notice (table below) together with the MIT permission notice. |
| Unicode-3.0 | 15 ICU4X crates, pulled in by `url` > `idna` > `idna_adapter` | [Unicode-3.0.txt](Unicode-3.0.txt) | The Unicode copyright and permission notice must be in the documentation that comes with the binaries. |
| MPL-2.0 | 1: `option-ext` 0.2.0, pulled in by `wry` > `dirs` > `dirs-sys` | [MPL-2.0.txt](MPL-2.0.txt) | Tell recipients where the source of `option-ext` is available (see [below](#option-ext-020)). Only `option-ext`'s own files are under MPL-2.0; it does not extend to Win11 for GNOME's code. |
| Apache-2.0 AND MIT | 1: `dpi` 0.1.2, pulled in by `wry` | [Apache-2.0.txt](Apache-2.0.txt) and the [libm notice](#dpi-012) below | Both apply: the Apache-2.0 text, and the MIT notice of `src/libm.rs`. The crate has no Apache NOTICE file. |
| Zlib | 1: `foldhash` 0.2.0, pulled in by `serde_json` | [Zlib.txt](Zlib.txt) | No notice is required in binaries; the notice may not be removed from source copies. Listed here as an acknowledgement. |
| (toolchain) | The Rust standard library, linked into every program | see [below](#rust-standard-library) | MIT notice of the Rust project, and the Unicode notice for the Unicode tables in `core`. `compiler_builtins` is also under Apache-2.0 WITH LLVM-exception (per upstream), which needs no notice in binaries. |

In the column "In", `host` means the host programs (`Cargo.lock`) and `native` means `win11-shell` and `win11-calendar` (`native/Cargo.lock`). 116 distinct crate versions are linked: 79 only in host, 21 only in native, 16 in both.

## MIT permission notice

This is the permission notice that the MIT licence requires next to each copyright notice in the table. It is copied from the `LICENSE` file of the `gtk` 0.18.2 crate; the MIT files of all the other crates in the table have the same wording (checked word for word, ignoring line breaks). The copy in [MIT.txt](MIT.txt) is the same text with the copyright line of Win11 for GNOME.

```text
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Crates

| Crate | Version | In | Declared license | Used under | Copyright notice (from the crate's own license files; authors from Cargo.toml where the files name none) | Source |
|---|---|---|---|---|---|---|
| ascii | 1.1.0 | host | Apache-2.0 OR MIT | MIT | Copyright (c) 2017 Thomas Bahn and contributors; Copyright (c) 2014 The Rust Project Developers | <https://github.com/tomprogrammer/rust-ascii> |
| atk | 0.18.2 | host | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk3-rs> |
| atk-sys | 0.18.2 | host | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk3-rs> |
| base64 | 0.22.1 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2015 Alice Maz | <https://github.com/marshallpierce/rust-base64> |
| bitflags | 1.3.2 | host | MIT/Apache-2.0 | MIT | Copyright (c) 2014 The Rust Project Developers | <https://github.com/bitflags/bitflags> |
| bitflags | 2.13.2 | host, native | MIT OR Apache-2.0 | MIT | Copyright (c) 2014 The Rust Project Developers | <https://github.com/bitflags/bitflags> |
| bytes | 1.12.1 | host | MIT | MIT | Copyright (c) 2018 Carl Lerche | <https://github.com/tokio-rs/bytes> |
| cairo-rs | 0.18.5 | host | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| cairo-rs | 0.22.9 | native | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| cairo-sys-rs | 0.18.2 | host | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| cairo-sys-rs | 0.22.9 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| chunked_transfer | 1.5.0 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2015 The tiny-http Contributors; Copyright (c) 2015 The rust-chunked-transfer Contributors | <https://github.com/frewsxcv/rust-chunked-transfer> |
| cookie | 0.18.2 | host | MIT OR Apache-2.0 | MIT | Copyright 2017 Sergio Benitez; Copyright 2014 Alex Chricton; Copyright (c) 2014 Alex Crichton | <https://github.com/SergioBenitez/cookie-rs> |
| deranged | 0.5.9 | host | MIT OR Apache-2.0 | MIT | Copyright 2024 Jacob Pratt et al. | <https://github.com/jhpratt/deranged> |
| dirs | 6.0.0 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2018-2019 dirs-rs contributors | <https://github.com/soc/dirs-rs> |
| dirs-sys | 0.5.0 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2018-2019 dirs-rs contributors | <https://github.com/dirs-dev/dirs-sys-rs> |
| dpi | 0.1.2 | host | Apache-2.0 AND MIT | Apache-2.0 AND MIT | Apache-2.0 part: no copyright line in the crate (the rust-windowing/winit contributors); MIT part (src/libm.rs, from rust-lang/libm): Copyright (c) 2018 Jorge Aparicio and the others named in the [libm notice](#dpi-012) | <https://github.com/rust-windowing/winit> |
| equivalent | 1.0.3 | host | Apache-2.0 OR MIT | MIT | Copyright (c) 2016--2023 | <https://github.com/indexmap-rs/equivalent> |
| field-offset | 0.3.6 | host, native | MIT OR Apache-2.0 | MIT | Copyright (c) 2016-2021 Diggory Blake, and other contributors. | <https://github.com/Diggsey/rust-field-offset> |
| foldhash | 0.2.0 | host | Zlib | Zlib | Copyright (c) 2024 Orson Peters | <https://github.com/orlp/foldhash> |
| form_urlencoded | 1.2.2 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2013-2016 The rust-url developers | <https://github.com/servo/rust-url> |
| futures-channel | 0.3.34 | host, native | MIT OR Apache-2.0 | MIT | Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors | <https://github.com/rust-lang/futures-rs> |
| futures-core | 0.3.34 | host, native | MIT OR Apache-2.0 | MIT | Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors | <https://github.com/rust-lang/futures-rs> |
| futures-executor | 0.3.34 | host, native | MIT OR Apache-2.0 | MIT | Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors | <https://github.com/rust-lang/futures-rs> |
| futures-io | 0.3.34 | host, native | MIT OR Apache-2.0 | MIT | Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors | <https://github.com/rust-lang/futures-rs> |
| futures-task | 0.3.34 | host, native | MIT OR Apache-2.0 | MIT | Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors | <https://github.com/rust-lang/futures-rs> |
| futures-util | 0.3.34 | host, native | MIT OR Apache-2.0 | MIT | Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors | <https://github.com/rust-lang/futures-rs> |
| gdk | 0.18.2 | host | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk3-rs> |
| gdk-pixbuf | 0.18.5 | host | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gdk-pixbuf | 0.22.0 | native | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gdk-pixbuf-sys | 0.18.0 | host | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gdk-pixbuf-sys | 0.22.9 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gdk-sys | 0.18.2 | host | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk3-rs> |
| gdk4 | 0.11.5 | native | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk4-rs> |
| gdk4-sys | 0.11.5 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk4-rs> |
| gio | 0.18.4 | host | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gio | 0.22.10 | native | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gio-sys | 0.18.1 | host | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gio-sys | 0.22.9 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| glib | 0.18.5 | host | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| glib | 0.22.10 | native | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| glib-sys | 0.18.1 | host | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| glib-sys | 0.22.9 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gobject-sys | 0.18.0 | host | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gobject-sys | 0.22.9 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| graphene-rs | 0.22.8 | native | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| graphene-sys | 0.22.9 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| gsk4 | 0.11.5 | native | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk4-rs> |
| gsk4-sys | 0.11.5 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk4-rs> |
| gtk | 0.18.2 | host | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk3-rs> |
| gtk-sys | 0.18.2 | host | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk3-rs> |
| gtk4 | 0.11.5 | native | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk4-rs> |
| gtk4-sys | 0.11.5 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk4-rs> |
| hashbrown | 0.17.1 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2016 Amanieu d'Antras | <https://github.com/rust-lang/hashbrown> |
| http | 1.5.0 | host | MIT OR Apache-2.0 | MIT | Copyright 2017 http-rs authors | <https://github.com/hyperium/http> |
| httpdate | 1.0.3 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2016 Pyfisch | <https://github.com/pyfisch/httpdate> |
| icu_collections | 2.4.0 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| icu_locale_core | 2.4.0 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| icu_normalizer | 2.4.0 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| icu_normalizer_data | 2.4.0 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| icu_properties | 2.4.0 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| icu_properties_data | 2.4.0 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| icu_provider | 2.4.0 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| idna | 1.1.0 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2013-2025 The rust-url developers | <https://github.com/servo/rust-url/> |
| idna_adapter | 1.2.2 | host | Apache-2.0 OR MIT | MIT | Copyright (c) The rust-url developers | <https://github.com/hsivonen/idna_adapter> |
| indexmap | 2.14.2 | host | Apache-2.0 OR MIT | MIT | Copyright (c) 2016--2017 | <https://github.com/indexmap-rs/indexmap> |
| itoa | 1.0.18 | host | MIT OR Apache-2.0 | MIT | authors: David Tolnay | <https://github.com/dtolnay/itoa> |
| itoa | 1.0.19 | native | MIT OR Apache-2.0 | MIT | authors: David Tolnay | <https://github.com/dtolnay/itoa> |
| javascriptcore-rs | 1.1.2 | host | MIT | MIT | Copyright (c) 2013-2021, The Gtk-rs Project Developers.; Copyright (c) 2021, Tauri Programme within The Commons Conservancy. | <https://github.com/tauri-apps/javascriptcore-rs> |
| javascriptcore-rs-sys | 1.1.1 | host | MIT | MIT | Copyright (c) 2013-2017, The Gtk-rs Project Developers. | <https://github.com/tauri-apps/javascriptcore-rs> |
| libc | 0.2.191 | host, native | MIT OR Apache-2.0 | MIT | Copyright (c) The Rust Project Developers | <https://github.com/rust-lang/libc> |
| litemap | 0.8.3 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| log | 0.4.35 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2014 The Rust Project Developers | <https://github.com/rust-lang/log> |
| memchr | 2.8.3 | host, native | Unlicense OR MIT | MIT | Copyright (c) 2015 Andrew Gallant | <https://github.com/BurntSushi/memchr> |
| memoffset | 0.9.1 | host, native | MIT | MIT | Copyright (c) 2017 Gilad Naaman | <https://github.com/Gilnaa/memoffset> |
| num-conv | 0.2.2 | host | MIT OR Apache-2.0 | MIT | Copyright (c) Jacob Pratt | <https://github.com/jhpratt/num-conv> |
| once_cell | 1.21.4 | host | MIT OR Apache-2.0 | MIT | authors: Aleksey Kladov | <https://github.com/matklad/once_cell> |
| option-ext | 0.2.0 | host | MPL-2.0 | MPL-2.0 | authors: Simon Ochsenreither | <https://github.com/soc/option-ext.git> |
| pango | 0.18.3 | host | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| pango | 0.22.9 | native | MIT | MIT | Copyrights in the gtk-rs Project project are retained by their contributors.; authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| pango-sys | 0.18.0 | host | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| pango-sys | 0.22.9 | native | MIT | MIT | authors: The gtk-rs Project Developers | <https://github.com/gtk-rs/gtk-rs-core> |
| percent-encoding | 2.3.2 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2013-2025 The rust-url developers | <https://github.com/servo/rust-url/> |
| pin-project-lite | 0.2.17 | host, native | Apache-2.0 OR MIT | MIT | no copyright line or authors in the crate; the pin-project-lite contributors, see the source | <https://github.com/taiki-e/pin-project-lite> |
| potential_utf | 0.1.6 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| powerfmt | 0.2.1 | host | MIT OR Apache-2.0 | MIT | Copyright 2023 Jacob Pratt et al. | <https://github.com/jhpratt/powerfmt> |
| raw-window-handle | 0.6.2 | host | MIT OR Apache-2.0 OR Zlib | MIT | Copyright (c) 2019 Osspial; Copyright (c) 2020 Osspial | <https://github.com/rust-windowing/raw-window-handle> |
| roxmltree | 0.19.0 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2018 Yevhenii Reizner | <https://github.com/RazrFalcon/roxmltree> |
| serde | 1.0.229 | host | MIT OR Apache-2.0 | MIT | authors: Erick Tryzelaar, David Tolnay | <https://github.com/serde-rs/serde> |
| serde_core | 1.0.229 | host, native | MIT OR Apache-2.0 | MIT | authors: Erick Tryzelaar, David Tolnay | <https://github.com/serde-rs/serde> |
| serde_json | 1.0.154 | host, native | MIT OR Apache-2.0 | MIT | no copyright line or authors in the crate; the serde_json contributors, see the source | <https://github.com/serde-rs/json> |
| slab | 0.4.12 | host, native | MIT | MIT | Copyright (c) 2019 Carl Lerche | <https://github.com/tokio-rs/slab> |
| smallvec | 1.16.4 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2018 The Servo Project Developers | <https://github.com/servo/rust-smallvec> |
| smallvec | 1.16.5 | native | MIT OR Apache-2.0 | MIT | Copyright (c) 2018 The Servo Project Developers | <https://github.com/servo/rust-smallvec> |
| soup3 | 0.5.0 | host | MIT | MIT | Copyright (c) 2013-2017, The Gtk-rs Project Developers. | <https://gitlab.gnome.org/World/Rust/soup3-rs> |
| soup3-sys | 0.5.0 | host | MIT | MIT | Copyright (c) 2013-2017, The Gtk-rs Project Developers. | <https://gitlab.gnome.org/World/Rust/soup3-rs> |
| stable_deref_trait | 1.2.1 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2017 Robert Grosse | <https://github.com/storyyeller/stable_deref_trait> |
| thiserror | 1.0.69 | host | MIT OR Apache-2.0 | MIT | authors: David Tolnay | <https://github.com/dtolnay/thiserror> |
| thiserror | 2.0.21 | host | MIT OR Apache-2.0 | MIT | authors: David Tolnay | <https://github.com/dtolnay/thiserror> |
| time | 0.3.55 | host | MIT OR Apache-2.0 | MIT | Copyright (c) Jacob Pratt et al. | <https://github.com/time-rs/time> |
| time-core | 0.1.9 | host | MIT OR Apache-2.0 | MIT | Copyright (c) Jacob Pratt et al. | <https://github.com/time-rs/time> |
| tiny_http | 0.12.0 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2014-2019 The tiny-http contributors | <https://github.com/tiny-http/tiny-http> |
| tinystr | 0.8.5 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| ureq | 2.12.1 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2019 Martin Algesten | <https://github.com/algesten/ureq> |
| url | 2.5.8 | host | MIT OR Apache-2.0 | MIT | Copyright (c) 2013-2025 The rust-url developers | <https://github.com/servo/rust-url> |
| utf8_iter | 1.1.0 | host | Apache-2.0 OR MIT | MIT | Copyright Mozilla Foundation | <https://github.com/hsivonen/utf8_iter> |
| utf_types | 1.0.0 | host | Apache-2.0 OR MIT | MIT | Copyright Mozilla Foundation | <https://github.com/hsivonen/utf_types> |
| webkit2gtk | 2.0.2 | host | MIT | MIT | Copyright (c) 2016 Boucher, Antoni &lt;bouanto@zoho.com&gt;; Copyright (c) 2017-2021, The Gtk-rs Project Developers.; Copyright (c) 2021, Tauri Programme within The Commons Conservancy | <https://github.com/tauri-apps/webkit2gtk-rs> |
| webkit2gtk-sys | 2.0.2 | host | MIT | MIT | Copyright (c) 2016 Boucher, Antoni &lt;bouanto@zoho.com&gt; | <https://github.com/tauri-apps/webkit2gtk-rs> |
| writeable | 0.6.4 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| wry | 0.55.1 | host | Apache-2.0 OR MIT | MIT | Copyright (c) 2020-2023 Ngo Iok Ui & Tauri Programme within The Commons Conservancy | <https://github.com/tauri-apps/wry> |
| yoke | 0.8.4 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| zerofrom | 0.1.8 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| zerotrie | 0.2.5 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| zerovec | 0.11.9 | host | Unicode-3.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | <https://github.com/unicode-org/icu4x> |
| zmij | 1.0.23 | host, native | MIT | MIT | authors: David Tolnay | <https://github.com/dtolnay/zmij> |

## Notices beyond the plain licence text

### dpi 0.1.2

`dpi` is licensed "Apache-2.0 AND MIT". Its `src/libm.rs` is code from rust-lang/libm and is licensed solely under the crate's `LICENSE-LIBM-MIT`, which reads in full:

```text
rust-lang/libm as a whole is available for use under the MIT license:

------------------------------------------------------------------------------
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
------------------------------------------------------------------------------

This Rust library contains the following copyrights:

    Copyright (c) 2018 Jorge Aparicio

Portions of this software are derived from third-party works licensed under
terms compatible with the above MIT license:

* musl libc https://www.musl-libc.org/. This library contains the following
  copyright:

      Copyright © 2005-2020 Rich Felker, et al.

* The CORE-MATH project https://core-math.gitlabpages.inria.fr/. CORE-MATH
  routines are available under the MIT license on a per-file basis.

The musl libc COPYRIGHT file also includes the following notice relevant to
math portions of the library:

------------------------------------------------------------------------------
Much of the math library code (src/math/* and src/complex/*) is
Copyright © 1993,2004 Sun Microsystems or
Copyright © 2003-2011 David Schultz or
Copyright © 2003-2009 Steven G. Kargl or
Copyright © 2003-2009 Bruce D. Evans or
Copyright © 2008 Stephen L. Moshier or
Copyright © 2017-2018 Arm Limited
and labelled as such in comments in the individual source files. All
have been licensed under extremely permissive terms.
------------------------------------------------------------------------------
```

### option-ext 0.2.0

`option-ext` 0.2.0 by Simon Ochsenreither is used unmodified under the Mozilla Public License 2.0 ([MPL-2.0.txt](MPL-2.0.txt)). Its source code is available from crates.io at <https://crates.io/crates/option-ext/0.2.0> and from <https://github.com/soc/option-ext>. The crate's files carry no copyright line of their own.

### Rust standard library

The programs are built with rustc 1.97.1 (8bab26f4f 2026-07-14), which links the standard library (`core`, `alloc`, `std` and their dependencies) statically. According to the toolchain's own `share/doc/rust/COPYRIGHT-library.html`:

- The Rust Standard Library is licensed "Apache-2.0 OR MIT"; Win11 for GNOME uses it under MIT. Copyright: The Rust Project Developers (see <https://thanks.rust-lang.org>).
- `library/backtrace`: Copyright 2014 Alex Crichton and The Rust Project Developers (Apache-2.0 OR MIT).
- `library/std/src/sync/mpmc`: Copyright 2019 The Crossbeam Project Developers and The Rust Project Developers (Apache-2.0 OR MIT).
- `library/core/src/unicode/unicode_data.rs`: Unicode-3.0, Copyright © 1991-2024 Unicode, Inc. The permission notice is the one in [Unicode-3.0.txt](Unicode-3.0.txt) (that copy carries the ICU4X year range 2020-2024; the wording is otherwise the same).
- Crates from crates.io built into the standard library, as listed in that file: `cfg-if` 1.0.4 (Copyright (c) 2014 Alex Crichton), `gimli` 0.32.3 (Copyright (c) 2015 The Rust Project Developers), `hashbrown` 0.17.1 (Copyright (c) 2016 Amanieu d'Antras), `libc` 0.2.185 (Copyright (c) The Rust Project Developers) and `rustc-demangle` 0.1.27 (Copyright (c) 2014 Alex Crichton), all "MIT OR Apache-2.0" and used under MIT; `foldhash` 0.2.0 (Zlib, Copyright (c) 2024 Orson Peters, [Zlib.txt](Zlib.txt)).
- Further crates built into the standard library, per upstream. `COPYRIGHT-library.html` does not list them and their sources (`rust-src`) are not installed, so the licences below are the declarations the upstream projects publish, not checked against the sources on this machine. Where a choice is offered, Win11 for GNOME uses MIT. The copyright holders are the contributors of the projects in the source column; their exact copyright lines were not checked offline.

| Crate | Version | Declared licence (per upstream) | Used under | Source |
|---|---|---|---|---|
| addr2line | 0.25.1 | Apache-2.0 OR MIT | MIT | <https://github.com/gimli-rs/addr2line> |
| adler2 | as shipped with rustc 1.97.1 | 0BSD OR MIT OR Apache-2.0 | MIT | <https://github.com/oyvindln/adler2> |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | MIT | <https://github.com/Frommi/miniz_oxide> |
| object | 0.37.3 | Apache-2.0 OR MIT | MIT | <https://github.com/gimli-rs/object> |
| memchr | 2.7.6 | Unlicense OR MIT | MIT | <https://github.com/BurntSushi/memchr> |
| compiler_builtins | as shipped with rustc 1.97.1 | MIT AND Apache-2.0 WITH LLVM-exception | MIT and Apache-2.0 WITH LLVM-exception | <https://github.com/rust-lang/compiler-builtins> |

For `compiler_builtins` both licences apply: the MIT permission notice above, and the Apache License 2.0 ([Apache-2.0.txt](Apache-2.0.txt)) with the LLVM exception (<https://spdx.org/licenses/LLVM-exception.html>). That exception lifts the notice conditions of Apache-2.0 (sections 4(a), 4(b) and 4(d)) for code that ends up embedded in compiled object form, as it does here.
