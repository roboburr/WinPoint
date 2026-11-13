# Round-2 gate: the Rust text conversion (GE1b, 2026-11-05)

What every round-2 agent (R2-A … R2-D, plan-critique H) needs: the frozen i18n API, the stems and their owners,
the conversion procedure with the tools, and the done-when checks. G = this repo. Binding next to `contracts.md`
(C1-C3, C9). Built and verified in GE1b (`notes/claude/round1/ge1b.md`); the `core` and `window` stems are the
reviewed pilot.

## 1. Status of the gate (plan-critique F8)

| F8 item | State |
|---|---|
| Frozen `i18n.rs` API | `host/src/i18n.rs` + `ApiError::tr/trn` in `host/src/files/mod.rs` (section 2). Nobody edits either in round 2. |
| Every stem file created, complete `include_str!` list | 24 stems in `host/i18n/` and in `i18n::STEMS` (section 3). A unit test fails when the table and the directory differ. |
| Converter | `tools/i18n/extract_rs.py` (`list`, `convert`, `add`, `map`, `en`) |
| Rust checker rules | `tools/i18n/check_rust.py` (key exists, nl/en placeholder parity, plural completeness, stem = file, params given, quoted UI labels, no Dutch in the English column) |
| Drift guard | `tools/i18n/drift_rust.py` + `tests/i18n/rust-frozen-all.tsv` (every non-test literal at 3b898e6) |
| Ratchet | `tools/i18n/drift_rust.py` (on by default; `--new` lists): no new Dutch literal outside the catalogues (GE1b fix round, review M1) |
| Pilot | `core`: runtime.rs envelopes, `unknown()`, the window/surface errors, the reference-proxy errors, services/mod.rs; `window`: C4 titles. Oracles green, `cargo test` green in the sandbox. |

## 2. The frozen API

```rust
use crate::i18n::{self, Lang};
// language (C1): Win11's own settings.language; "en" is English, anything else Dutch
ctx.lang() -> Lang                       // services: the ServiceContext's settings file (one stat, cached)
ctx.language() -> String                 // "nl" | "en", the same value as text (existing call sites keep working)
i18n::current_language() -> Lang         // code without a ServiceContext (picker, gnome, launch, transfers …)
i18n::language_in(&config_dir) -> Lang   // a specific settings directory (tests, files/mod.rs's own config dir)
// text
i18n::tr(lang, "stem.key") -> String
i18n::trf(lang, "stem.key", &json!({"name": name.to_string()})) -> String
i18n::trn(lang, "stem.key", n, &json!({...})) -> String     // .one when n == 1, .other, optional .zero; {n} = n
i18n::NO_PARAMS                          // &Value::Null, for "no parameters"
ApiError::tr(code, lang, "stem.key", params) -> ApiError        // message + key + params (C2)
ApiError::trn(code, lang, "stem.key", n, params) -> ApiError
// formatting
i18n::decimal(lang, "1.5") / i18n::number(lang, x, digits)     // "1,5" in nl; no grouping, each caller's rounding
i18n::join(lang, &items) / i18n::join_or(lang, &items)          // "A, B en C" / "A, B and C"
i18n::pick(lang, nl, en)                 // inline {nl, en} data pairs that stay inline (software t(), _FRIENDLY)
```

* `lang` is anything `AsLang`: a `Lang`, or the `"nl"`/`"en"` strings the services already pass around (`&lang`,
  `ctx.language()`, `files::Context::language`). Exactly `"en"` is English.
* Placeholders are `{name}` (ASCII letters, digits, `_`). A value is a string or a number (numbers print as
  `format!("{}")` does). A placeholder without a value stays as written. There are no brace escapes: a lone `{` is a
  brace.
* Fallback: language → nl → the key itself. Each miss is logged once per process, in the bilingual log style
  (`i18n: sleutel ontbreekt (missing key) …`).
* The catalogues are embedded (`include_str!`), so there is no missing-file path. A row edit rebuilds the crate.
* `ApiError::json()` gives `{"code","message","key","params","data"}`. `key`/`params` are present only for catalogue
  messages, and `params` is then always an object. `ApiError::new` is unchanged, and so is its JSON.
* **Call the API only through the `i18n::` path** (`crate::i18n::trf(…)`, or `use crate::i18n;` + `i18n::trf(…)`)
  and `ApiError::tr`. check_rust.py finds keys only there. Several services have their own bare `tr()` for their
  JSON tables (disks, storage, sysmon, system), and a bare `tr(` would be mistaken for one.
* **A bin crate** (bin/launch.rs, which includes launch_context.rs by `#[path]`) has no `crate::i18n`. Add
  `#[allow(dead_code)] #[path = "../i18n.rs"] mod i18n;` to that bin. i18n.rs only uses std, serde_json and glib, and
  its tests use no other crate module, so this works.

**Language rules** (C1, and H §2.4.4 where it applies to G):
* There is no per-request language in v1 (D-G7: the host ignores `X-Shell-Lang`). Every text is rendered in the
  settings language at the moment it is built. `ctx.lang()` is one `stat` plus a cached value (2.3 µs per call
  against 30.5 µs for the 3b898e6 `language()`, debug build), so reading it per call or per item is fine.
* **Never freeze a text.** Do not render in a `static`, `const`, `OnceLock`, `lazy` initialiser or in a long-lived
  struct field built once. Cache keys must include the language. Today these do not: disks' 1 s snapshot and
  storage.volumes' 2 s cache (R2-B).
* Text that is stored and shown later must be stored as key + params and rendered when it is read:
  `index.status` (R2-B, re-send on a language change); software job messages (frozen in round 2, Windows Update
  later). An event broadcast to every window may be rendered at emit time: all windows share one language.
* `files::Context::language` (files/mod.rs:153, R2-C) does a full `fs::read` of settings.json per `files.*` call.
  Replace it with `i18n::language_in(&self.config)` while converting.

## 3. Stems and owners

The first key segment is the stem. A stem is one file `host/i18n/<stem>.tsv`, and only its owner writes it.

| Stem | Owner | Source files |
|---|---|---|
| `core` | GE1b, frozen | runtime.rs, services/mod.rs |
| `window` | GE1b, frozen | window.rs (C4 titles) |
| `update` | rounds 4-6 | the update.* service |
| `system` | R2-A | services/system.rs, services/system/** (incl. the Wi-Fi texts, D-G2 `power.rs:154`) |
| `display` | R2-A | services/display.rs (D-G2 `:86`; the monitors.conf header stays, C9) |
| `media` | R2-A | services/media.rs |
| `notifications` | R2-A | services/notifications.rs (the error texts; C5 is done) |
| `shell` | R2-A | services/shell.rs, services/shell/{backlog,params}.rs |
| `clipboard` | R2-A | services/shell/clipboard.rs |
| `filemanager1` | R2-A | services/shell/filemanager.rs |
| `tray` | R2-A | services/tray.rs, services/tray/** |
| `settings` | R2-A | settings.rs, settings_runtime.rs, settings_effects.rs (CSS markers stay, C9) |
| `sysmon` | R2-B | services/sysmon.rs, services/sysmon/** |
| `storage` | R2-B | services/storage.rs, services/storage/** |
| `disks` | R2-B | services/disks.rs, services/disks/** |
| `index` | R2-B | services/index.rs, services/index/**, bin/index_worker.rs |
| `files` | R2-C | files/** except the ApiError block |
| `files_launch` | R2-C | the launch/open paths of files/actions.rs |
| `apps` | R2-D | apps/** |
| `picker` | R2-D | picker.rs, portal.rs and portal/** (if converted) |
| `transfers` | R2-D | transfers.rs |
| `launch` | R2-D | launch.rs, bin/launch.rs, launch_context.rs, process_scope.rs |
| `gnome` | R2-D | gnome.rs (incl. `:25`) |
| `wallpaper` | R2-D | wallpaper.rs |

**Unowned in round 2, so they stay as they are:** lib.rs (only start-up errors that go to stderr, allow-listed),
events.rs (one start-up io error), backend.rs (the reference backend, internal), images*, window_state.rs (English
internal errors), app_params.rs, bin/app.rs (CLI usage), services/software* (already bilingual pairs; frozen).

## 4. Converting one file (the procedure)

Work in your own worktree (`.wt/r2-<x>`). Run each tool from the worktree root, because they locate everything from
their own path. Never type a Dutch text, and never edit a TSV row by hand.

1. **List the sites:** `python3 -B tools/i18n/extract_rs.py list host/src/<file>.rs`.
   * Each line gives the ordinal (the site id), the line, the fn, GE0b's class at 3b898e6 and the pattern the
     converter will use.
   * Where H's nl is byte-identical, the line also gives H's en. `[H has only plural rows …]` marks a D-G1a case.
   * `REFUSED: …` marks a site you convert by hand (section 5).
   * Allow-listed literals (`tests/i18n/allow-host.txt`, C9), technical literals and logs are hidden; `--all`
     shows them.
2. **Decide per site:** the key (`<stem>.<camelCase meaning>`, e.g. `files.notFound`; name it after the meaning,
   not the error code), the language expression (`@self.ctx.lang()`, `@ctx.lang()`, `@&lang`, `@lang`; the default
   is `crate::i18n::current_language()`), and placeholder names for positional `{}` arguments (`:name,path`).
   Inline `{e}` keeps its name unless you rename it.
3. **Dry run:**
   ```
   python3 -B tools/i18n/extract_rs.py convert host/src/<file>.rs --stem <stem> \
     --site 12=files.notFound:name@ctx.lang() --site L340=files.gone@&lang \
     --en "12=“{name}” no longer exists." --note "12=Verkenner: an item vanished during the action" \
     --expect "12=‘{}’ bestaat niet meer."
   ```
   * It prints the call-site patch and the catalogue diff, and writes nothing.
   * `--site L<line>=…` picks the single user-visible literal on that line. `--expect ORD=TEXT` refuses a site
     whose text differs.
   * **Ordinals shift after every edit of the file:** run `list` again right before each `convert`. The converter
     refuses literals GE0b classed technical or log unless you pass `--force`.
   * A plan file (`--plan PLAN.tsv`, columns: ordinal, key, names, lang, en, note) does many sites at once.
     Keep plan files in your notes directory.
4. **Review the patch.**
   * The patch touches only the converted expressions.
   * `ApiError::new(c, "…")` / `ApiError::new(c, &format!(…))` become `ApiError::tr(c, LANG, "key", params)`.
   * `format!(…)` becomes `crate::i18n::trf(…)`.
   * `"…".to_string()` / `String::from("…")` become `crate::i18n::tr(…)`; `"…".into()` becomes `tr(…).into()`.
   * `.ok_or("…")` becomes `.ok_or_else(|| tr(…))`.
   * A `json!` value becomes `tr(…)`.
   * Any other bare literal becomes `&crate::i18n::tr(…)`, marked REVIEW: check the type and lifetime.
   * Arguments become `"name": expr.to_string()`, which is the same Display text `format!` printed.
5. **Apply** with `--apply`, then `tools/heavy.sh --tag r2<x>-check -- cargo check --offline --locked -j2
   --all-targets`. Fix type errors by hand without touching the texts.
6. **English.** Every row needs en; check_rust.py fails on an empty one.
   * Write en-GB in Windows 11 wording (`tools/i18n/glossary.tsv`).
   * Take en from H only when the converter offered it (exact nl match).
   * Change an en after review with `python3 -B tools/i18n/extract_rs.py en <key> "<text>" --apply`.
   * A quoted UI name in en ("Starten toestaan" → “Allow launching”) must be the UI's en text. Otherwise
     check_rust.py fails ([label]). Allow-list a name that is no UI label (a GNOME app's own name) with
     `label:<text>` in your allow file.
   * No Dutch in en ([en-dutch], an error): no word of `tests/i18n/dutch-words.txt` (built to contain no English
     word), and no en that is a copy of a Dutch nl. A Dutch name that must stay (an app's own name) is
     allow-listed with `en:<text>` (path `host/i18n/<stem>.tsv`). A warning names en words that only GE0b's
     wider heuristics call Dutch ("map", Dutch word shapes): check each.
   * The converter warns when a site has no `@LANG` while its fn has `self.ctx`, `ctx`, `lang` or `language`
     in scope (`list` shows the suggestion as a last column, e.g. `@self.ctx.lang()`). Take it unless the
     process language is really meant: the default `crate::i18n::current_language()` reads the process's own
     settings file, so a service test with its own config dir would get the wrong language.
7. **Run the checks** (section 6). Then hand-convert what was refused, and list every intended Dutch change.

**What the tools write.** Each row gets a comment with the fn and context, and a machine line directly above it:
```
# serve(): ApiError "forbidden": HTTP 403: a request from another origin
#@ host/src/runtime.rs 70 7f1d3bc763e111b4 plain
core.forbidden	Geen toegang.	Access denied.
```
* `#@ <path> <3b898e6 ordinal> <sha256[:16]> <fmt|plain|en|enfmt> [<spec>=<name> …]` says which 3b898e6 literal
  the row replaced and how its placeholders were renamed (`{}=name`, `{e}=error`).
  * `fmt`/`plain`: the literal is the row's nl.
  * `en`/`enfmt`: the literal was the English half of an inline pair and is the row's en.
* Several sites can share a row: one `#@` line each.
* `#@ new` marks a row with Dutch text that did not exist (it needs a `key:` line, section 5).
* Keep `#@` lines directly above their row, with no blank line in between.

## 5. Cases the converter does not do

* **`{:?}`, `{x:>5}`, `{:.2}` (any format spec).** Turn the value into text first, e.g.
  `let size = format!("{:.1}", v);`, then convert `format!("… {size} …")`. The drift check then sees the old literal
  gone: add a BASELINE-CHANGES line, or, when the Dutch output is byte-identical for every value, state that in the
  line's reason. Decimal points inside Dutch messages becoming commas is a Dutch change (H §2.8.2): list it.
* **A literal `{word}` in the text** (from `{{word}}`): convert by hand. The catalogue has no brace escape.
* **Sentences built from pieces** (`files/properties.rs:158` "Alle van het type "): one row per piece keeps Dutch
  byte-identical. Merging the pieces into one sentence is a Dutch change and needs a line.
* **Plurals (D-G1a).** Adopting `.one`/`.other` changes nl for n == 1.
  * Map the old literal to the `.other` row.
  * Add `.one` with `extract_rs.py add --stem <s> --key <k>.one --nl … --en …`.
  * Add `key:<k>.one` to your BASELINE-CHANGES file.
* **Rows whose nl is a 3b898e6 literal the code no longer holds as such** (moved into a table): use
  `extract_rs.py add … --from host/src/<file>.rs#<ordinal>`, which writes the `#@` line for you.
* **Inline pairs** (`if nl { "…" } else { "…" }`, `{nl, en}` tuples; 105 pairs outside software). These are
  converted by hand.
  * Convert the nl half with `convert` (or `add --from`) and give the pair's English as `--en`.
  * Rewrite the `if` by hand.
  * Record the English half with
    `extract_rs.py map <key> --from host/src/<file>.rs#<ordinal> --en [--spec '{}=name' …] --apply`.
  * The drift check then accepts both removed literals, and the render test checks the English too.
  * `map` without `--en` records an nl literal for a site you converted entirely by hand.
  * The pilot uses `map --en` for seven window titles (window.tsv).
* **Removed technical literals** (a refactor drops `"ok"`, `"code"` json keys): the drift check reports them.
  `python3 -B tools/i18n/drift_rust.py --propose` prints a line per uncovered literal; fill in a real reason.
* **Tests that pin Dutch in an English context** (e.g. files/actions.rs
  `malformed_uri_authority_is_rejected_before_any_handler_launch` expects "Deze link is ongeldig." for an en
  context). Update them to the language-correct text in the same change. Never change an oracle file
  (`services/{system,shell,disks}/reference-tests.json`): they are the Dutch guard.
* **Input words** (`index/query.rs`): never translated. An English example inside a translated message must stay
  an input word the parser accepts (it accepts only `dm:vandaag`, not `dm:today`). Say so in the row's note.

**BASELINE-CHANGES.** Write your own `tests/i18n/BASELINE-CHANGES.rust-r2<x>`; drift_rust.py reads it with the
main file. Its format is the header of `tests/i18n/BASELINE-CHANGES.rust`, five tab-separated fields:
```
host/src/<file>	<ordinal>	<sha16>	<replacement or ->	<reason>     (a frozen literal changed or removed)
key:<catalogue key>	-	-	<old text or ->	<reason>                    (a row with new Dutch text)
```
Every line is a Dutch change Robin will see in the report. D-G1a plural fixes are accepted. D-G2 GNOME-wrong texts
belong to R2-A. Hold back anything cosmetic.

## 6. Done-when (per agent; the coordinator runs the same on the merge)

| Check | Command (from the worktree root) | Must give |
|---|---|---|
| Drift and ratchet | `python3 -B tools/i18n/drift_rust.py --paths <your host/src files> --warnings`, then plain `python3 -B tools/i18n/drift_rust.py --warnings` | `0 errors`; explain each `new:` warning in your note |
| Catalogue and call sites | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | `0 errors`; explain each warning in your note (glossary warnings: either fix them or say why the term differs). Without node the label rule is an error; `--no-labels` skips it, which you must state |
| Build | `tools/heavy.sh --tag r2<x>-check -- cargo check --offline --locked -j2 --all-targets` | no new warning in your files (baseline: lib 28, win11-launch 2) |
| Tests incl. the 3 oracles | `tools/heavy.sh --min-mb 2048 --tag r2<x>-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` (in a worktree; see the note below for the main tree) | all green; lib count = 269 + yours, 8 ignored; the 16 oracle tests (`oracle_tests::*` 8, the 7 `disks::tests` python cases, `reference_contracts`) ok; `logs/guard-calls.log` empty |
| nl render per converted site | part of the test run: `i18n::tests::every_converted_site_renders_its_3b898e6_dutch` reads every `#@` line and checks that the runtime renders the 3b898e6 bytes for sample values | ok |
| en render per stem | add one test per stem that renders a few converted messages in en through the real call site, like `runtime::i18n_tests` does for core | ok |
| No frozen-language text | grep your files for `OnceLock`/`static` around `tr(`, and key the caches you touched on the language | none left |
| Process-list cost (R2-B only, F28) | `tests/gnome/scenarios/ge1b-perf.py` with your build (its docstring has the command; `--out tests/gnome/scenarios/ge1b-perf.out/r2b`) | `sysmon.processes` median within noise of GE1b's `r1` run, or explained |

**The ratchet** (part of every plain `drift_rust.py` run; `--no-ratchet` turns it off, `--new` only lists):
* It takes every literal of the working tree beyond the 3b898e6 multiset of its file. A new file is new as a
  whole; moving a literal to another file makes it new there.
* GE0b's classifier (`tools/i18n/scan_rust.py`, run on the working tree, about 5 s) labels each one.
* An error is a `user-visible-dutch` literal, or the Dutch half of a new inline pair, unless it is a catalogue key
  or matches a rule of `tests/i18n/allow-host*.txt` (C9).
* A warning is a new English-only user text (Dutch users would see English), or Dutch-looking text that the
  classifier calls technical.
* Convert what it reports, or allow-list it in your `allow-host-r2<x>.txt` with a reason (a log helper, a
  matcher …). A refactor that keeps a text in the same file keeps it frozen, not new.

* **Never** pass `--include-ignored`.
* **Heavy commands** (main tree and worktrees alike, after the coordinator's lock fix, contracts C8):
  `tools/heavy.sh --min-mb 2048 --tag r2<x>-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast`. heavy.sh takes
  the machine-wide lock and exports it as `WIN11_HEAVY_LOCK`/`WIN11_HEAVY_HELD`, so run-tests.sh (and run.sh, the
  harness) use it without a second flock. The wrapper-lock recipe is retired. run-tests.sh's `env -i` drops
  `CARGO_TARGET_DIR`, so tests in a worktree build in the worktree's own `target/` (about 1 GB).

## 7. Never translate (C9)

`tests/i18n/allow-host.txt` lists them by path and selector:
* log and CLI macros, `.expect` texts;
* lib.rs/runtime.rs start-up errors (stderr only);
* HTTP reason phrases;
* the gtk.css markers (`settings_effects.rs` CAP_*/THEME_*) and the monitors.conf header;
* the index query parser's aliases, date words and other technical literals;
* the UDisks/BlueZ/tray error matchers;
* process_scope's unit and slice names;
* the XDG fallback folder names;
* api-methods.json.

The converter refuses all of these. A round-2 agent that finds another one adds it to
`tests/i18n/allow-host-r2<x>.txt`, which the coordinator merges. Until then, say so in your note.
