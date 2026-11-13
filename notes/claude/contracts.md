# Contracts between the layers (fixed before round 1)

Coordinator document. A step may not change a contract on its own; if one turns out wrong, write it in your step
note and the coordinator decides. G = this repo.

## C1 Language value

* `settings.language` is `"nl"` or `"en"`. Anything else (missing file, missing key, unknown string, the object form
  that `settings.rs:886-890` accepts) means `"nl"`.
* Win11's own setting is the only source. Never the GNOME/system locale, `LANG`, or `navigator.language` (D-G3).
  Single exception: the installer's first-install guess (H D4/D13), which reads the locale once and then stores the choice.
* Rust: `ctx.language()` inside services; contexts without a `ServiceContext` use `i18n::current_language()`, which
  caches the parsed value keyed by the settings file's (mtime, size, inode) so no call does a full parse.
* Extension: reads only `GLib.get_user_config_dir()/win11-gnome/settings.json` (not H's `~/.config/win11shell` file),
  watches the directory (`WATCH_MOVES`, the store writes by rename), creates no files.
* The host ignores `X-Shell-Lang` in v1 (D-G7). The UI may send it; nothing depends on it. Rust never adds `?lang=`.

## C2 Error envelope

```json
{"ok": false, "error": {"code": "not_found", "message": "<rendered text>", "key": "files.not_found", "params": {"name": "a.txt"}, "data": {}}}
```

* `code` and `message` stay as they are today. `message` is rendered in the language of the process at call time.
* `key` (string, catalogue key `stem.name`) and `params` (object, the named placeholders, strings or numbers) are
  optional and added by converted call sites. `data` keeps its current meaning.
* The UI matches errors on `code` (and on `key` when it needs more detail), never on `message` text.

## C3 Catalogues

* Rust: `host/i18n/<stem>.tsv`, H's format (`key<TAB>nl<TAB>en`, `\n`/`\t`/`\\` escapes, `{name}` placeholders,
  plurals as `key.one`/`key.other` (and optional `key.zero`), first key segment = stem), embedded with `include_str!`.
  Fallback: current language → nl → key; each miss logged once.
* nl values come from G's own literals, byte for byte, never from H. en values may come from H only when H's nl is
  byte-identical to G's literal; everything else is new en (en-GB, Windows 11 wording, `tools/i18n/glossary.tsv`).
* UI keeps `makeT` dictionaries. Extension and installer use inline `{nl, en}` tables (they must work on failure paths).
* Dutch changes are listed per layer, one exact key per line with a reason: `tests/i18n/BASELINE-CHANGES.ui`,
  `.rust`, `.ext`.

## C4 Native window titles

* The host owns GTK window titles (`window.rs`): one title per kind in nl and en, re-applied when the language changes.
  Titles are the app's name as the UI shows it in that language ("Alles" in both, D-G5). No page or folder names in v1.
* The UI does not send `setTitle` for native windows, and the host ignores `setTitle` for host-titled windows
  (round 1). A file-portal caller's own picker title still wins.

## C5 Notifications

* The host stores its own notifications with `appName: ""` (not "Melding"). An item that enters the host's own store
  (`add()`) with `"Melding"` is stored as `""`. The extension's notification list and its events pass through
  unmapped (revised after round 1, GE1b decision 1).
* The UI renders an empty `appName` with its own localized label (nl "Melding", en "Notification").

## C6 Desktop entries

* Names and comments per language live in `host/src/desktop-entries.json` (schema: `version`, `comment {nl,en}`,
  `entries {<Name>: {nl, en}}` for the 10 `io.github.win11shell.<Name>.desktop` files).
* `scripts/install.py` writes `Name=`/`Comment=` in the language of the existing settings.json (missing → `--lang` →
  nl). The rest of each entry is unchanged.
* At runtime only `win11-panel` rewrites them: at start and on a language change, only the `Name=` and `Comment=`
  lines, only files in `$XDG_DATA_HOME/applications` whose `StartupWMClass=io.github.win11shell.<Name>` matches,
  atomically (temp file + rename), and only when the text differs.

## C7 Extension files

* Prefer keeping new extension code inside the existing files. If a new file is added, `scripts/install.py` must copy
  it (derive the list from the directory, minus `schemas/`) and the harness install check must prove it.

## C8 Heavy commands

* Every cargo, Chrome or gnome-shell run goes through `tools/heavy.sh [--min-mb N] --tag <step> -- <cmd>` (lock,
  memory and disk check inside the lock, own process group, watchdog at 1500 MB). gnome-shell runs and release builds
  use `--min-mb 2560`. Cargo always `--offline --locked -j2`.
* heavy.sh exports `WIN11_HEAVY_LOCK=WIN11_HEAVY_HELD=<lock>` to the command. A nested heavy.sh,
  `tests/i18n/run.sh`, `tests/rust-sandbox/run-tests.sh` and the GNOME harness then use that lock and skip their own
  flock, also inside a git worktree (coordinator fix after round 1). So `tools/heavy.sh -- tests/rust-sandbox/run-tests.sh`
  is the plain recipe everywhere; the wrapper-lock recipe is retired. Never edit heavy.sh while a heavy run is going:
  bash reads a running script from disk.
* `/tmp` is allowed only for one run's sockets and temporary files that are deleted at the end of that run (Unix
  socket paths inside G exceed 108 bytes). Anything that must survive goes inside G.
* Headless Chrome runs only inside `tests/update-ui/nonet.sh`'s bubblewrap layer (allowlisted read-only root, empty
  `/run`, loopback only, host sockets masked). `tests/i18n/run.sh` re-executes itself through heavy.sh and nonet.sh
  when called bare; `tests/update-ui/shoot.sh` does the same (round 2 coordinator fix, GW3 review B1).

## C9 Never translate

Machine markers written into user files (`settings_effects.rs` CSS marker lines), input words the index parser
accepts (`index/query.rs`), matchers for daemon output (`disks.rs`, `bluetooth.rs`, `tray/helpers.rs`), logs,
systemd unit ids, XDG folder names on disk, `api-methods.json` docs. They go into `tests/i18n/allow-host.txt` with a
reason.

## C10 Test system bus (Windows Update, D-G39)

The update service accepts a test bus from `WIN11_UPDATE_PK_BUS` only when all of these hold, and otherwise fails
closed (`test_seam`, no bus opened):
* the value is exactly one `unix:path=` address (an optional `,guid=` is allowed; no abstract, tcp or second address);
* the socket sits directly in `$XDG_RUNTIME_DIR`, and that directory is not under `/run`;
* that directory is owned by the user, has mode 0700 and holds the `.win11-update-fixture` marker;
* `/run/dbus/system_bus_socket` is not visible (the caller runs inside a layer with a private `/run`).
GW1's `tests/update/run.sh` meets it. GW8 writes the marker into the GNOME harness runtime directory before it starts
the fake there. Production never sets the variable and always uses the real system bus.
