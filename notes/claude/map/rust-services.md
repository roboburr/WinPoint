# Map: Rust host services: user-visible strings, language and i18n (read-only survey, 2026-11-05)

Scope: `host/src/services/**` (disks, display, index, media, notifications, shell, software, storage, sysmon, system,
tray and their submodules), `host/src/settings.rs`, `settings_effects.rs`, `settings_runtime.rs`, `runtime.rs`,
`backend.rs`, `events.rs`. Branch `claude/gnome-english-update` (= baseline `3b898e6`). Nothing was built or run.
I read the files and used one throwaway literal extractor (a Rust string lexer that skips comments, `#[cfg(test)]`
items and the test files). The counts are approximate (±5 %). The method is under "Not checked".

## Summary

* **The Rust host already knows the UI language.** `ServiceContext::language()` (`services/mod.rs:62-69`) reads
  `settings.json` → `"en"` or `"nl"`, and 31 call sites use it. It does not look at the request, `LANG` or
  `Accept-Language`. `files::Files::context()` (`files/mod.rs:153`) and `apps` read the same setting on their own.
* **About 40 % of the in-scope user text is already bilingual.** It uses four ad-hoc carriers:
  * embedded JSON `_STR{nl,en}` tables: disks 104, storage 50, sysmon 63 and system 33 keys, plus 12 Wi-Fi `[nl,en]`
    pairs and 63 sysmon `_FRIENDLY` process-name pairs (100 % key parity);
  * software's inline `t(lang, nl, en)` / `err(lang, code, nl, en)` pairs, about 63 of them;
  * a handful of `if lang == "en"` ternaries.

  Codex ported these from the Python host as they were.
* **Still Dutch-only:** about 232 UI-visible literal sites (about 186 unique texts). Most are `ApiError` messages.
  On top of that come about 42 start-up/internal errors (backend.rs, index cache), 25 systemd unit names that are
  Dutch-only by design (English falls back to the unit's own description), 7 Dutch log lines and 2 CSS file markers.
  Biggest files: sysmon/actions.rs 35, index.rs + index/query.rs 41, tray.rs 19, runtime.rs 17, storage/* 31,
  system/energy.rs 13, settings.rs 14.
* **Error shape:** `{"ok":false,"error":{"code","message","data"?}}` (`files/mod.rs:43-49`, `runtime.rs:527-542`).
  `code` is a category, not a message id (`bad_args` alone has 60+ different messages). **There is no stable message
  key.** disks passes a catalog key internally, but it is not put on the wire.
* **IPC:** each window process runs its own `tiny_http` server on 127.0.0.1. A call is
  `POST /api/call/<method>` with body `{"args":{…}}` and the `X-Shell-Token` header (`vendor/ui/js/core/bridge.js:139-160`,
  `vendor/ui/js/apps/taskmgr/hostcall.js:87-100`). Each request gets its own thread (`runtime.rs:210-212`). Events
  arrive over SSE `/api/events`. The 172 methods in `api-methods.json` are `{name, dangerous, doc}` only.
  **A language could ride along as an `X-Shell-Lang` header with about 15 lines of change.** No such header exists
  today.
* **Formatting in Rust is limited and already mostly bilingual:**
  * three size formatters (storage `fmt_size`, sysmon `fmt_bytes`, index `size_label`). The index one is
    Dutch-comma only;
  * one duration formatter (sysmon `fmt_duration`, bilingual);
  * no user-facing dates. The UI formats almost all numbers and dates itself.
* **H's catalogs fit G almost as they are.**
  * The nl column of H's reviewed E2 TSVs equals G's `_STR` nl **100 %**: 246 of 250 keys are present, and the 4
    others became `.one/.other` rows in H.
  * **196 of G's Dutch-only literals have an identical-nl row in H's `i18n/host/*.tsv`.** About 102 of those rows
    were reviewed (E2). About 94 come from the unreviewed round 2.
* **Proposal:** add one small `host/src/i18n.rs`.
  * Catalogs: TSV `key<TAB>nl<TAB>en` (H's format), embedded with `include_str!`, so there is no runtime file and no
    broken-install path.
  * Lookup: `tr/trf/trn`. Fallback: current language → nl → key. Plurals use `.one/.other(/.zero)`.
  * Language: a thread-local request language from `X-Shell-Lang`, falling back to `settings.language`.
  * Errors: an additive `key`/`params` in the error envelope.
  * Do not merge the UI's `makeT` dictionaries into it.
  * Estimate for this area: **about 32 agent-hours**.

## Findings

### F1. How errors and texts reach the UI

* `ApiError { code, message, data }` (`files/mod.rs:29-50`) is serialised as `{"code","message","data"?}`. The
  envelope in `runtime.rs:527-542` is `{"ok":true,"result"}` or `{"ok":false,"error":…}`. The UI shows
  `err.message` as it is (`bridge.js:159-160`, fallback `'Er ging iets mis.'`).
* Raw HTTP envelopes in `runtime.rs` are Dutch JSON at :276 ("Geen toegang."), :309 ("Gebruik POST."),
  :322/:332/:367 ("Ongeldige aanvraag."), :339/:353 ("Te veel gegevens.") and :427 ("Referentiedienst niet
  bereikbaar: {e}"). Plain-text bodies are English: "Bad URL", "Unauthorized", "Not Found", "Method Not Allowed"
  (:225-:388). Those are never shown, because bridge.js turns 401/403 into its own text.
* Service start-up failures become `ApiError::new("offline", &message)` (`services/mod.rs:116`). So a `String` error
  from any `Service::new` reaches the UI, in Dutch.
* Error constructions in scope:
  * about 150 direct `ApiError::new(` sites;
  * plus module helpers: disks `self.err(code, key, args)` → catalog (`disks.rs:192`, about 41 sites);
    software `err(lang, code, nl, en)` (about 36); storage `err(code, msg)` (`storage/common.rs:78`, about 18);
    sysmon `err(code, msg)` (`sysmon/common.rs:91`, about 14);
  * plus 8 raw JSON envelopes in runtime.rs.
* Errors inside successful results:
  * Wi-Fi `{"ok":false,"code","error","reason"}` (`system/network.rs:735-755`, bilingual);
  * `results[].error` of storage cleanup (`storage/cleanup.rs:212`/:352 "Dat lukte niet (ApiError).", :254
    "Sluit eerst {}.", :291-:375);
  * sysmon endTask/efficiency (`sysmon/actions.rs:133-308`);
  * software job `message` and `error.message` (`software.rs:471-615`).

### F2. Inventory by file (non-test code)

Columns: **NL** = Dutch-only UI-visible literal sites; **Cat** = catalog lookups / keys used; **Pairs** = inline nl/en
pairs; **EN-UI** = English shown to the user; **Tech** = English or neutral technical strings that must stay as
they are (D-Bus errors to other apps, matched daemon texts, protocol).

| File | NL | Cat | Pairs | EN-UI | Notes |
|---|---|---|---|---|---|
| runtime.rs | 15 | – | – | – | 6 ApiError (:92 "De dienst is afgesloten.", :130/:132 "Venster is gesloten/reageert niet", :139, :145, :518 "Deze functie bestaat niet."), 8 JSON envelopes, :445 SSE text; :42/:204 start-up |
| backend.rs | (18 internal) | – | – | – | start-up/instance-file errors (:22-:213); visible only if start-up fails or with `WIN11_REFERENCE_INSTANCE` |
| events.rs | (1 internal) | – | – | – | :35 "De runtime-map moet privé zijn" |
| services/mod.rs | 1 | – | – | – | :87 "De dienst wordt gestart." |
| settings.rs | 12 | – | – | – | :124-:168, :444-:556 ("Ongeldige invoer.", "Deze instelling kan niet worden gewijzigd." …); :57, :410 internal |
| settings_runtime.rs | 1 | – | – | – | :97 "De instellingendienst wordt afgesloten."; 3 Dutch eprintln (:122, :139, :140) |
| settings_effects.rs | 0 | – | – | – | :17/:20 Dutch **CSS markers** written into GTK css (machine markers: keep byte-identical); 3 Dutch eprintln (:48-:71); :669 internal |
| services/display.rs | 8 | – | – | – | :82-:149; :86 "…alleen in de Win11-sessie." is Hyprland-era (`hypr_available()`); :387 Dutch header comment in monitors.conf (file) |
| services/media.rs | 4 | – | – | – | :218-:232 |
| services/notifications.rs | 5 | – | 3 | – | ApiError :394-:505; **:361 `appName` fallback "Melding"** is stored in history; test notification :513-:514 bilingual; :420 passes `language` to the extension (`extension/notificationsBridge.js:214`, its own inline pairs); :125/:159 English D-Bus errors (tech) |
| services/shell.rs, shell/clipboard.rs | 4 | – | – | – | shell.rs :234/:250, :330 user-name fallback "gebruiker"; clipboard :124, :133 |
| services/shell/filemanager.rs | 0 | – | – | – | English diagnostics :413-:492 (tech/log) |
| services/index.rs | 19 | – | – | – | ApiError :284-:453; event `index.status` message :255; child-error relay :316-:320 ("Er ging iets mis bij het zoeken."); drive labels :521 "Systeemschijf", :528 "{}-schijf"; :171-:273 internal pipe errors |
| services/index/engine.rs (child process) | 4 | – | – | – | root errors :161 "Deze locatie is niet gevonden.", :165 "Dit is geen map." (go into status), :358/:940, :1007; :897 usage (stderr) |
| services/index/query.rs (child process) | 18 | – | – | – | query errors :39-:906 (`ext:`, `size:`, `dm:` help). The 30 Dutch keywords at :273-:847 are **input vocabulary**, bilingual on purpose: keep |
| services/index/cache.rs | (10 internal) | – | – | – | cache-corruption io::Errors (:305-:939); the cache is rebuilt |
| services/software.rs + software/{common,inspect,resolve}.rs | ~1 | – | ~63 | – | already bilingual per call (`lang` once, :146); the English internal errors in appimage/flatpak/snapd/packagekit (~67) end up in the job's `error.message` as "details" (EN, like PackageKit's own text) |
| services/storage/{cleanup,scan,common,categories}.rs | 31 | 50 keys (`_STR`) | – | – | scan :141-:643, cleanup :192-:758 (including pkexec results :689-:758), common :332-:347; volumes/categories via `tr()` |
| services/storage/volumes.rs | 0 | yes | – | – | `fmt_size(lang, …)` :84-:105 bilingual |
| services/sysmon/actions.rs | 35 | some | – | – | endTask/restart/service texts :73-:892, including `format!("Beëindigen lukte niet ({}).")` :184 |
| services/sysmon/common.rs | 2 | 63 keys + 63 `_FRIENDLY` | 4 ternaries | – | `unit_friendly` :222-:295: 25 Dutch-only unit names (returns `None` for en, by design) + `_GSD_NL`/`_GVFS_NL`; `fmt_bytes`, `fmt_duration` bilingual |
| services/sysmon/{startup,process,health,performance}.rs, sysmon.rs | 5 | yes | own_names :998-:1006 (5 tuples) | – | startup :190-:223, sysmon.rs :135 |
| services/system/bluetooth.rs, agent.rs | 0 | 33 keys (`strings.json`) | – | – | via `text(ctx,key)` (`system.rs:485`); English matched BlueZ texts :1213-:1224 (tech) |
| services/system/network.rs | 7 | 12 Wi-Fi pairs | – | – | ApiError :278-:548; :564/:568 input words |
| services/system/energy.rs | 13 | – | – | – | power-profiles texts :100-:361, new GNOME code; **no row in H** |
| services/system/{audio,hardware,power}.rs, system.rs | 20 | – | – | – | audio :79-:157, hardware :142-:168, power :53-:195 (:154 says "Daarvoor zijn Hyprland en hyprsunset nodig": a Hyprland-era text), system.rs :497-:524 |
| services/tray.rs | 19 | – | – | – | ApiError :1284-:1563; menu **labels come from the apps** over DBusMenu (`tray/helpers.rs:580`), the host adds none; :523-:574 English D-Bus errors to SNI clients (tech) |
| services/disks/* | 0 | 104 keys (`_STR`) | – | – | fully bilingual; one composite of two catalog sentences `disks/checks.rs:132-:136`; English UDisks texts matched at `disks.rs:566-:592` (tech) |

Totals:
* Dutch-only UI-visible ≈ 232 sites (≈ 186 unique);
* internal/start-up ≈ 42, Dutch-only by design 25, Dutch logs 7, file markers 2 (+1 config comment);
* bilingual ≈ 262 catalog keys + 63 friendly-name pairs + ≈ 63 software pairs + ≈ 10 ternaries/tuples.

No GTK dialogs in scope. GTK dialogs live in portal.rs and picker.rs, outside this area. No desktop notification is
raised by the Rust host itself. The only host-made notification is `notifications.test`.

### F3. Where the language comes from today

* `ServiceContext::language()` (`services/mod.rs:62-69`) calls `self.settings()`, which is `SettingsStore::snapshot()`
  (`settings.rs:77-81`). That takes the settings lock, reads `settings.json` in full and compares bytes
  (`settings.rs:300-340`). It is fresh per call, so after a switch the next call is already in the new language.
  There is no worker-mirror race of the kind H's D15 fixed.
* Cost: one lock + read + clone per `language()` call. disks calls it for every `err()` (`disks.rs:189-193`). Fine
  in practice, but a per-request value would be cheaper.
* Not read anywhere: `LANG`/`LC_*` (except `locale t_fmt` for the 24-hour default, `system/info.rs:345`), the HTTP
  `Accept-Language` header (WebKit derives that from the system locale: en_US on Robin's machine, which wants Dutch,
  so it **must not** be used), and the UI's `?lang=` override (`vendor/ui/js/core/theme.js:49`). With `?lang=en` a
  Dutch host therefore still answers in Dutch.
* The settings store does not restrict `language`: a test sets `language.custom` (`settings.rs:886-890`). Every
  reader maps "not `en`" to `nl`, so this is harmless.
* **Frozen-language spots:**
  * `index.status` message emitted from a background thread (`index.rs:255`) and root errors in the child's status
    (`index/engine.rs:161-165`);
  * software job `message` written into the job file in the language at creation (`software.rs:471`, :517, :547,
    :606-:615) and shown later;
  * notifications' "Melding" stored per item (`notifications.rs:361`);
  * caches not keyed on language: the disks snapshot for 1 s (`disks.rs:293-312`) and `storage.volumes` for 2 s
    (`storage.rs:105-115`). Both are short.
  * `storage.volumesChanged` is rendered in the settings language (`storage.rs:84`), which is correct.

### F4. Number, size, date and duration formatting in Rust

| Site | What | Language handling |
|---|---|---|
| `storage/volumes.rs:84-105` `fmt_size` | drive sizes, 0 decimals from 10 | bilingual (`.replace('.', ',')` for nl) |
| `sysmon/common.rs:298-322` `fmt_bytes` | 1024 threshold, "{n} bytes" | bilingual (same replace) |
| `sysmon/common.rs:323-345` `fmt_duration` | "{s} seconden"/"{s} s", "1 minuut"/"1 minute", "{h} u {m} min"/"{h} h {m} min" | bilingual (hand-written plurals) |
| `index.rs:614-624` `size_label` | "1,8 TB", "{n} GB" for the drive name "{size}-schijf" (:528) | **Dutch comma always**; the label is Dutch |
| `sysmon/health.rs:303-307` | `h_svc_warn_t` vs `h_svc_warn_t_pl` by `n == 1` | ad-hoc plural |
| `disks/checks.rs:120-136`, `storage/cleanup.rs:102` | list joins `", "` and `" / "` | language-neutral separators |
| `display.rs:209` `fmt_rate`, `system/audio.rs:103` | "1920x1080@60", volume for the CLI | machine values: keep |
| `settings.rs:738-757` `stamp()` (strftime) | backup file name stamps | machine: keep |
| `index/query.rs:743`, :823-:847 | parses "uur/dagen/vandaag/size:" in **both** languages | input: keep |

Everything else returns raw numbers or epoch milliseconds, and the UI formats them with `Intl` (`core/format.js`).

### F5. What H offers (read: `notes/english-option-design.md` §0-2.4.5, §2.8.2; `i18n/host/*.tsv`)

* The format is `i18n/host/<area>.tsv` (`key\tnl\ten`, escapes `\n`/`\t` only, `{name}` placeholders,
  `key.one/.other/.zero` plurals, first key segment = file stem), with semantic keys and the fallback current → nl → key.
* The host API was `L(key, **p)`, `Ln(key, n, **p)`, `pick(nl, en)`, `decimal()`, `join()`. The error envelope got
  an additive `key`/`params` (D8). The caller's language came from the `X-Shell-Lang` header (D15), and text
  rendered once for everybody (events, shared caches) used the settings language.
* Status: H's E2 rows (`core`, `ctl`, `disks`, and E2's `storage`/`sysmon`/`system`) were reviewed. Rows written in
  round 2 (`index`, `tray`, `apps`, `settings`, `display`, `notifications`, `media`, `clipboard`, the E3 additions
  to `sysmon`/`storage`/`system`) were "neither reviewed nor tested" (§[2026-11-05] Frozen).
* Measured against G:
  * `_STR` nl in G equals H's TSV nl for every shared key: disks 104/104, storage 50/50, system 33/33, sysmon 59/59.
    The 4 remaining sysmon keys are now `.one/.other` rows in H.
  * The English differs in 2 rows: H fixed `disks.unavailable` ("Disk Management") and `sysmon.h_upd_sec_m`.
  * Dutch-only literals in G with an exact (placeholder-normalised) nl match in H: **196**. By H file: sysmon 41,
    index 32, system 25, storage 23, tray 19, apps 12, core 12, settings 9, display 8, notifications 5, files 4,
    media 4, disks 1, clipboard 1.
  * No match (new English needed) for about 40 UI texts: all of `system/energy.rs` (13), `runtime.rs` :92/:130/:132/:145/:427,
    `services/mod.rs:87`, `index.rs:284`, `notifications.rs:361`/:431, `clipboard.rs:133`, `network.rs:286`,
    `sysmon/actions.rs:608`/:638, `process.rs` own_names, `settings_runtime.rs:97`, plus about 25 `unit_friendly`
    names if English names are wanted.

## Proposal: smallest sound i18n mechanism for the Rust host

1. **Catalog:** H's TSV format as it is, under `G/i18n/host/<area>.tsv`, so one checker can read UI, host and
   scripts later. The Rust code embeds the files with `include_str!` in a static list `[("core", include_str!(…)), …]`.
   It parses them lazily once (`OnceLock<HashMap<&'static str, [&'static str; 2]>>`). The catalog lives in the
   binary, so there is no missing-file failure path, and H's D14 "inline pairs on failure paths" is not needed in
   Rust. cargo rebuilds when an included file changes.
2. **API** (`host/src/i18n.rs`, about 200 lines with tests):
   * `enum Lang { Nl, En }` with `Lang::parse(&str)` (only nl/en; case and surrounding space ignored) and
     `Lang::from_settings(&Value)` (anything other than `"en"` is `Nl`);
   * `tr(lang, key) -> &'static str`; `trf(lang, key, &[("name", &dyn Display)]) -> String` (same `{name}` fill as
     the existing `tr()` helpers); `trn(lang, key, n, params)` → `key.zero` (n == 0, only when the row exists) /
     `key.one` (n == 1) / `key.other`, filling `{n}`;
   * `decimal(lang, &str)` (swaps the separator of an already formatted number, so each formatter keeps its own
     rounding) and `join(lang, items, And|Or)`;
   * a missing key in en falls back to nl, a missing key in nl returns the key, each logged once to stderr.
3. **Errors:** `ApiError` gets `key: Option<&'static str>` and `params: Option<Value>`. They are additive in
   `json()`, and `ApiError::new` stays. The new constructor is `ApiError::tr(code, lang, key, params)`. The UI keeps
   showing `message`. The Rust tests can assert keys instead of Dutch text.
4. **Language per call:**
   * `bridge.js` `realCall` and `taskmgr/hostcall.js` `fetchCall` send `X-Shell-Lang: lang()`.
   * `runtime.rs serve()` reads the header (it already reads `X-Shell-Token`, :256-:262) into a `thread_local!`
     `Cell<Option<Lang>>`. That is safe, because each request runs on its own thread (:210-:212).
   * `ServiceContext::language()` returns `request_lang().unwrap_or(settings)`, so all 31 existing call sites
     follow without edits. `files::Files::context()` and `apps` get the same one-line change.
   * Background threads (index/storage/disk monitors, software job threads) have no request language and use the
     settings language, which is right for events. Software already passes `lang` into its job thread explicitly.
5. **Child processes:** the index worker (`bin/index_worker.rs`, the same crate, so the catalog is available there
   too) returns `{code, key, params}` and the parent renders it in the caller's language (H's E3b design). Root
   errors in the status keep a key, not text. The software and image workers produce no UI text.
6. **Existing carriers:**
   * Convert the 4 `_STR` JSON blocks and the Wi-Fi pairs to TSV mechanically, or take H's reviewed E2 TSVs
     directly, since their nl is verified identical. The three `tr()` helpers (`disks.rs:75`,
     `storage/common.rs:192`, `sysmon/common.rs:191`) and `system.rs:485` become one-line wrappers.
   * Keep `t(lang, nl, en)` in software and the `_FRIENDLY`/`own_names` data tables as inline pairs, and mark them
     for the checker (they are complete, and moving them gains nothing).
7. **Sharing with the JS UI: do not share one catalog.** The UI's per-module `makeT` dictionaries stay (H §2.2: 2,500
   keys, live next to their code). The host returns text already rendered in the caller's language. Shared are the
   convention (keys, `{name}`, `.one/.other`, fallback) and one checker. A generated mock module from the TSVs
   (H §2.4.5) is optional, and less useful in G, where the UI mostly runs against the real host.
8. **Plural rules:** nl and en both use only `one` (n == 1) and `other` for integers, so `n == 1` is the full rule.
   It is the same as `makeT`.

## Work needed (estimates in agent-hours, this area only)

| # | Item | Size |
|---|---|---|
| 1 | `host/src/i18n.rs` (Lang, TSV parser, tr/trf/trn, decimal, join, log-once, request thread-local) + unit tests | 3 |
| 2 | `ApiError` `key`/`params` + `ApiError::tr`; runtime.rs envelopes (:276-:427, :518) and mod.rs/runtime.rs offline texts via `core.*` | 2 |
| 3 | `X-Shell-Lang`: bridge.js, taskmgr/hostcall.js, `runtime.rs serve()`, `ServiceContext::language()`, `files::context()`; test: `?lang=en` against an nl setting | 1.5 |
| 4 | `_STR` JSON → TSV (disks, storage, sysmon, system, Wi-Fi), wrappers, `_pl` → `.one/.other` (sysmon/health.rs:303) | 2 |
| 5 | Move the Dutch-only texts to keys, by area: system/* incl. energy (33 sites) 3; sysmon (40) 2.5; storage (31) 2; index + child protocol + status re-render (41) 4; tray (19) 1; settings*/display/media/notifications/shell/clipboard (35) 2 | 14.5 |
| 6 | Frozen-language fixes: disks 1 s and storage.volumes 2 s caches keyed on language; software job messages rendered at read time (store key) or accepted as is; notifications "Melding" → `""` with the UI label (UI change in notifcenter.js) | 2 |
| 7 | `index.rs:614` `size_label` through `decimal()`; check the other two formatters keep their nl output | 0.5 |
| 8 | Ratchet test in Rust: scan `host/src/**/*.rs` literals with H's `tests/i18n/dutch-words.txt` + allowlist (input words, CSS markers, logs D10); TSV parity/placeholder test | 3 |
| 9 | Dutch-unchanged guard: keep `services/system/reference-tests.json` (475 cases, 48 Dutch messages), disks and shell oracles green; frozen nl column snapshot | 1.5 |
| 10 | English review (en-GB Windows 11 wording, H §4.2 glossary) of about 94 unreviewed round-2 rows + about 40 new rows (energy, runtime, …) | 2 |
| | **Total** | **≈ 32** |

Decisions Robin needs to make (they change Dutch text, so they are not part of "Dutch unchanged"):
* the Hyprland-era messages `system/power.rs:154` (night light needs "Hyprland en hyprsunset") and `display.rs:86`
  ("alleen in de Win11-sessie") are wrong on GNOME;
* whether the 25 `unit_friendly` names get English or keep the systemd description.

## Risks

* **Dutch drift.** Moving 232 literals by hand risks typos in nl. The oracle fixtures (reference-tests.json) cover
  only part of them, so a frozen-nl check of the TSV column against the old literals (H's `nl-frozen` idea) is
  needed. Extracting by script from the literal list is safer than retyping.
* **Unreviewed English.** About half of the reusable H rows are round-2 output. Do not take them on trust.
* **Request language leaking into shared state.** A `LazyService` created on a request thread
  (`services/mod.rs:104-117`) and caches filled on a request thread would render in the request language. This
  matters only when the header differs from the setting (`?lang=` testing), but caches should key on the language.
* **Child process.** The index worker keeps its own copy of the catalog (same binary build). If parent and child
  come from different builds during an update, keys could be missing. The fallback to key/nl hides that, and the
  log line shows it.
* **Third-party text stays English:** PackageKit/snapd/flatpak details in software jobs, D-Bus errors relayed
  from apps, and tray labels from apps. That is right (Windows shows vendor text too), but an English-only reviewer
  may flag it.
* Matching on daemon English (`disks.rs:566-592`, `bluetooth.rs:1213-1224`, `tray/helpers.rs:484-497`) depends on
  the daemons running in an English or C locale. This is unrelated to the UI language. Do not "translate" these.
* Every `language()` call does a full settings read under the lock. Calling `tr()` per string with `ctx.language()`
  inside loops (process lists) would add I/O. Read the language once per call.

## Not checked

* No build, no tests run, no live behaviour. Line numbers are from baseline `3b898e6`.
* The counts come from my own literal lexer plus a Dutch-word heuristic (H's 1,295-word list + stems). I
  hand-corrected about 20 false positives ("processes", "processor", D-Bus names). The nl/en pair detection
  (adjacent literals within 4 lines) undercounts pairs in long one-line `json!` expressions. Treat the numbers as
  ±5 %.
* Out of area and not inventoried: `host/src/files/**`, `apps/**`, `portal*.rs`, `picker.rs` (the GTK dialogs),
  `gnome.rs`, `lib.rs`, `launch*.rs`, `transfers.rs`, `window*.rs`, the extension, the vendored UI, scripts.
* I did not check which UI modules show which host text, or whether they refetch on a language change (H §2.4.4
  list), or whether G's vendored UI has H's E1 additions. Its `core/i18n.js` is the older 65-line version (Oct 25),
  against H's 153 lines.
* I did not verify that every Dutch `Err(String)` in internal paths (backend.rs, index/cache.rs) really never
  reaches the UI. I classified them by reading the call paths, not by tracing every one.
* I did not compare G's English pairs in software.rs and the JSON catalogs against the H §4.2 glossary.
