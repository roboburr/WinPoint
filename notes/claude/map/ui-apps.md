# UI apps map: Taakbeheer, Verkenner, Alles, Schijven, preview, mock (English option)

Read-only survey, 2026-11-05. Scope: the Win11 Shell UI apps listed in the title, plus the Rust host texts they show.

Paths used below:
- **G** = `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome`, branch `claude/gnome-english-update`; the UI files are under `G/vendor/ui/js/`.
- **H** = `/srv/codex-claude/claude-scratch/c2c10c4c-ae6d-4f6a-a907-342de497d54f/931db6b9-2976-4e14-8e5a-041952887025/scratch-2026-10-25-a089b6`.
- Line numbers are G's unless marked H.

How the evidence was gathered:
- **Hashes:** sha256 of each area file was compared across four versions: G's `vendor/ui-original-sha256.json`, G as it is now, H's `dist/i18n-snapshots/*/ui/js/**`, and H as it is now.
- **H's own scanners, run read-only on G:** `tools/i18n/jsdicts.mjs` and `tools/i18n/scan_ui.mjs`, under node. They were pointed at G through a scratch root that symlinks `ui` to `G/vendor/ui`.
- **Rust literals:** counted with a regex heuristic in the scratchpad.
- **Diffs:** of the H E1 snapshot (which equals G's baseline) against H now, and against G now.

## Summary

- **G's copy of these apps is H's UI from just before step E1.**
  - 62 area files are in Codex's baseline manifest. 57 of them hash-equal H's `dist/i18n-snapshots/E1/` copy: 52 that Codex left alone and 5 that Codex later changed.
  - `mock/settings.js` is even older: it equals H's E0a snapshot. So does `core/i18n.js`, outside this area.
  - Four Taakbeheer files (`hostcall.js`, `store.js`, `hostbanner.js`, `pages/details.js`) come from before H's services split. They lack the split's "direct routes" code.
- **The area is already bilingual by design.**
  - It has 23 `makeT` dictionaries with 1,290 keys, and nl/en key parity is 100 % (0 missing either way). 31 of those keys are plural objects.
  - Every context menu, dialog, column header, status bar and empty state goes through `t()`.
  - Number, size and date formatting goes through `core/format.js` with `locale()` (nl-NL or en-GB).
- **What is missing in the UI is H's E1 list for these apps, plus Codex's own additions.**
  - From E1: 14 plural objects, 4 sites that use Dutch text directly, `collator()`, keeping state over the language-switch reload, refetching host text after a switch, and `X-Shell-Lang`.
  - From Codex: 5 hand-made nl/en strings and 15 English-only error texts that can reach a Dutch infobar.
- **Codex's changes in this area mostly do not follow the i18n pattern.**
  - `explorer/picker.js` does, with its own `makeT` of 15 keys.
  - `explorer/tabtransfer.js`, `explorer/main.js` (exportTab) and `alles/main.js` (the indexer note) do not.
  - The Alles note calls the app "Everything" in English. That contradicts H's decision D5 that "Alles" stays a name. The note also ignores `?lang=`.
- **Most of H's i18n work for these apps can be ported file by file.**
  - 18 files can be copied from H as they are (their H diff is i18n only).
  - 3 Taakbeheer files need only the i18n hunks, because H's versions also carry Hyprland-split code.
  - 2 files need a manual merge: `explorer/main.js` and `alles/main.js`.
  - 2 Codex-only files need new work: `picker.js` and `tabtransfer.js`.
  - All the other area files are byte-identical in G and H.
- **The Rust host already localises names and labels.**
  - Disks `_STR` has 104/104 keys, storage 50/50 and sysmon 63/63, plus the friendly process and service names.
  - File places, MIME type names, new-item names and the "- kopie" / "- Copy" suffix follow `settings.language`.
- **Host errors and refusals are still Dutch only.**
  - There are about 212 unique Dutch sentence literals in `files/`, `sysmon/`, `storage/`, `index/`, `picker.rs` and `transfers.rs`.
  - 141 of them equal an nl row in H's `i18n/host/*.tsv` exactly, so their English already exists.
  - The host has no `X-Shell-Lang` and no `key`/`params` in its errors. It reads the language from settings.json on every call.
- **In the native runtime, Verkenner loses or gets stale tabs on a language-switch reload.**
  - `explorerSession` is written into the init script once, when the window is created (`G/host/src/window.rs:482`).
  - H's sessionStorage keep-over-reload fixes this, but it has to be ordered against Codex's ticket and session restore.
- **Every `vendor/ui` edit must also regenerate `vendor/ui-sha256.json` and update `vendor/ui-patches.json`.** Otherwise `scripts/package.py` fails, because it runs `verify-ui.py`.
- **Effort for this area:**
  - UI: about 12 agent-hours, including the nl/en verification.
  - Rust host texts these apps show: about 16 agent-hours.

## Findings

### F1. Where G's files come from (hash check)

| Group | Files | G vs Codex baseline | Baseline = H snapshot | Meaning |
|---|---|---|---|---|
| Unchanged by Codex, from before H's E1 | 52 files: all area files except the rows below | equal | `E1/` | H's E1 diff applies cleanly |
| Before the split (Taakbeheer) | `taskmgr/hostcall.js`, `store.js`, `hostbanner.js`, `pages/details.js` | equal | not `E1/` (H's E1 snapshot already has the split's direct routes) | port the E1 hunks only |
| Older core | `mock/settings.js` (and `core/i18n.js`, outside the area) | equal | `E0a/` | needs E0a and E1 |
| Changed by Codex | `alles/main.js`, `explorer/main.js`, `explorer/tabs.js`, `explorer/detailspane.js`, `mock/system.js` | changed | `E1/` | see F5 |
| Added by Codex | `explorer/picker.js`, `explorer/tabtransfer.js` | new | none | G only |

`core/bridge.js` is outside this area, but the area depends on it. Codex's change to it is a single line (the settings icon, `bridge.js:273`). So H's E1 `bridge.js` can be ported almost exactly.

### F2. (a) How strings are written, per file

Notes on the columns:
- **"own"** means the file has its own `makeT`; **"shared"** means it imports `t` from the app's `i18n.js`.
- **`t()` sites** is an approximate grep count.
- **Hard-coded** counts user-visible literals outside the dictionaries. It comes from H's `scan_ui.mjs` (157 hits in G overall, 26 in this area) plus a manual pass.

**Taakbeheer** (`apps/taskmgr`, 20 files, every one with its own dictionary except `store.js` and `util.js`)

| File | Dict keys nl/en | Plurals | `t()` sites | Hard-coded / notes |
|---|---|---|---|---|
| actions.js | 80/80 | 0 | 101 | `endBodyMany` :22/69, `endTitleMulti` :24/71 and `pCount` :42/89 are not plurals. `normStatus` :130-131 matches the Dutch words `reageert` and `onderbroken`; harmless, because Rust sends the codes `running`/`suspended`/`efficiency` (`sysmon/process.rs:955`) |
| main.js | 13/13 | 0 | 11 | `?page=`/`?tab=` aliases :61-71 (`opruimen`, `categorieën`, `grote-bestanden`) stay by design (H allow-ui `keep`). `onLangChange` :235 rebuilds the pages but does not refetch host text |
| table.js | 6/6 | 0 | 2 | `new Intl.Collator(['nl','en'], …)` :26 (H: `collator(COLL_OPTS)`) |
| util.js | none | | 0 | `plural(n, one, many)` :54 has no callers (H removed it). `isEn()` :55 |
| hostcall.js / hostbanner.js | 5/5, 9/9 | 0 | 9, 10 | no `X-Shell-Lang` (:97); `makeError` :73 does not copy `key`/`params` |
| store.js | none | | 0 | no `langChanged()`; in-flight calls are reused across a switch (:93) |
| pages/details.js | 28/28 | 0 | 35 | `many: '{n} processen'` :19/28 is not a plural |
| pages/performance, processes, services, settings, startup, storage, summary | 90, 37, 33, 39, 29, 16, 44 | 0,0,0,0,1,0,1 | 99, 43, 28, 27, 25, 11, 43 | none; `processes.js:54` picks a `TYPE_LABELS[nl/en]` table |
| storage/categories, largefiles | 12, 33 | 0, 4 | 12, 37 | none |
| storage/cleanup.js | 45/45 | 3 | 55 | `` `${t('freed')} – ${t('partly').toLowerCase()}` `` :321 builds a sentence from parts. `t('items', {n: number(d.count), count})` :132: G's `makeT` prefers `n`, so Dutch shows "1.000 item" when the count is 1000 (the plural bug fixed in H's E0a) |
| storage/explorer.js | 26/26 | 0 | 30 | `other: 'Overige ({n} kleinere items)'` :18/28 is not a plural |
| storage/sunburst.js | 6/6 | 0 | 6 | `file1`/`filesN` plus a hand-written `n === 1` branch :20-24 |

**Verkenner** (`apps/explorer`, 13 files, one shared dictionary `i18n.js` with 256/256 keys and 13 plurals, plus `picker.js`'s own)

| File | `t()` sites | Hard-coded / notes |
|---|---|---|
| main.js | 171 | 1 hand-made pair through `document.documentElement.lang` :368-370 (Codex); 3 English-only `Error`s :367, :439, :1987 (Codex); the address-bar and `?place=` aliases :1529-1530 and :1760-1765 stay by design; the language switch reloads the window :1716 without keeping the tabs |
| util.js | 21 | `lang()==='en' ? 'Shortcut' : 'Snelkoppeling'` :102 and `` `${EXT}-bestand` `` :106 are ternaries. `TYPES_NL` has 69 keys and `TYPES_EN` only 28 (:73/:89); this is only a fallback, because Rust sends `typeName` (`files/read.rs:607`, 47 MIME pairs). The collator :173-176 already uses `locale()` |
| dialogs.js, toolbar.js, pages.js, detailspane.js, navpane.js, fileview.js, tabs.js | 85, 42, 21, 18, 8, 6, 5 | none. Column labels are passed in from main.js through `t()`; menus are `{label: t(…)}` (main.js:1388-1460) |
| dnd.js | 0 | no strings |
| picker.js (Codex) | 24, own `makeT` with 15/15 keys at :23 | `t('save')==='Opslaan' ? 'Opslaan als' : 'Save as'` :49 compares translated text; `onLangChange` :88 does not relabel the read-only checkbox or the "Alle bestanden" filter; the error from `win11:picker-error` :87 is Rust's Dutch text |
| tabtransfer.js (Codex) | 0, no `makeT` | its own `text(en, nl)` keyed on `<html lang>` :7, used for 2 user texts (:54, :143-145); 12 English-only validation and transfer `Error`s (:10-40, :168), which reach the infobar through `fail()` :54 |

**Alles** (`apps/alles`, 7 files, one shared dictionary `i18n.js` with 229/229 keys and 8 plurals)

| File | `t()` sites | Hard-coded / notes |
|---|---|---|
| main.js | 77 | the quick chip sets the query `'dm:vandaag'` :384, so an English user sees a Dutch query; `t('hintHidden'), ' ', link` :434 splits one sentence around the link; `selected: number(sel)` :465 has no `count`; the Codex pair :420-423 (F5); the switch reloads :644 and loses the query, sort and focus |
| i18n.js | 7 | `'Afbeelding'` fallback inside `ty_image` :258; `trashFailedBody` "item(s)" :60/189; `trashTooMany` and `copyCapped` :61, :65 are not plurals; the en tips quote Dutch examples ("foto", "vakantie", "werk", "map") |
| panes.js | 15 | Dutch-only tip examples :85-91 (`foto`, `vakantie 2024`, `in:~/Documenten factuur`, …), shown in both languages |
| actions.js, options.js, table.js, highlight.js | 35, 47, 2, 0 | none (`highlight.js` has no text) |

**Schijven** (`apps/disks`, 7 files, one shared dictionary `i18n.js` with 233/233 keys and 1 plural)

| File | `t()` sites | Hard-coded / notes |
|---|---|---|
| i18n.js | n/a | `hours` :44/254, `bytes` :120/290 and `labelHint` :160/320 are not plurals; `s_busy: '{op} {p}'` :66/268 builds a phrase from parts (used at util.js:134-137); the en text "Disk management doesn’t work" :227 should be capitalised as "Disk Management" |
| properties.js, actions.js, overview.js, util.js, main.js | 110, 97, 27, 18, 17 | none hard-coded. `main.js:216` `onLangChange` re-renders but does not reload, so host labels ("Schijf 0", partition types, SMART messages) stay in the old language until the next `load()`; `load()` :93 reuses an in-flight call across a switch |
| numparse.js | 0 | already locale-aware (`Intl.NumberFormat(loc).formatToParts`) |

**preview/wm.js**: own `makeT` with 6/6 keys, nothing hard-coded. It is only used by the in-browser preview surface (`js/main.js`), not by the GNOME windows.

### F3. (a) The same findings, by kind of text

- **Context menus, dialogs, column headers, status bars, empty states:** these all go through `t()` in all four apps. There are 26 `EmptyState`/`setEmptyText` sites, and every one has a key. Shortcut labels ("Ctrl+C", "Alt+Enter") are the same in nl and en-GB Windows.
- **Text from the host shown as it is:** `e.message` in Explorer's `showError` (main.js:910-918), in the picker, in Schijven's `actions.js:29` / `main.js:135,138` and in Taakbeheer. That text is Dutch today (F4).
- **Size and date formatting:**
  - `format.js` `number`, `bytes`, `rate`, `bitrate`, `percent`, `date`, `time` and `fileTime` all use `Intl` with `locale()`.
  - `bytes()` uses unit words ("byte(s)", "KB"…) that are the same in nl and en.
  - `duration()` and `relTime()` hold their own nl/en word ternaries; H keeps those by design.
  - `DateTimeFormat` and `RelativeTimeFormat` are built on every call (format.js:105, :132, :142). H's E1 caches them: 2,000 date+time rows took 208 ms uncached and 4 ms cached.
- **Plurals:**
  - There are 31 plural objects in the area.
  - The selection rule (G `core/i18n.js:52`) is `Number(params?.n ?? params?.count ?? 0)`. With a formatted `n`, Dutch "1.000" becomes 1 and selects `one`. Example: Taakbeheer › Opslag › Opruimen shows "1.000 item".
  - H's E0a added `countOf`, and E1 converted 14 more strings to plural objects in this area (F6).
- **Collation:**
  - `taskmgr/table.js:26` sorts with a fixed `['nl','en']` locale.
  - `explorer/util.js:175` already uses `locale()`.
  - `disks/actions.js:61,64` uses `toLocaleLowerCase` for the typed confirmation word, which is fine.

### F4. (b) Text that comes from the Rust host

Language source: `ServiceContext::language()` (`G/host/src/services/mod.rs:62`) and `files::Context` (`files/mod.rs:142-163`). Both read settings.json on every call. No request header can override it. Errors are `{code, message}` only.

**Already bilingual on the host (nothing to do except check it):**
- **Schijven:**
  - `services/disks/data.json` `_STR` has 104 nl and 104 en keys, read through `tr()` (`disks.rs:75`).
  - Through it come disk names ("Schijf {n}"), volume names (`model.rs:525`), `partTypeName` (`t_*`), file-system names, protection reasons (`r_*`), error mapping (`map_error`, `disks.rs:418`) and the "no S.M.A.R.T." message.
  - SMART *status* is a code (`ok`/`warning`/`critical`/`unknown`, `model.rs:301-323`), and the UI renders it (`properties.js:164-165` `t('h_'+s)`).
  - Raw udisks values are shown as they are: attribute names (`properties.js:139`) and a self-test status other than `success` (`:131`).
- **Taakbeheer:**
  - `services/sysmon/text.json`: `_STR` has 63 keys per language (health checks `h_*`, the "Win11 Shell" host row, disk, Wi-Fi and Ethernet labels).
  - `_FRIENDLY` has 63 `[nl, en]` process names. `unit_friendly()` (`common.rs:223`) gives Dutch service names and returns `None` in English, which leaves systemd's English description.
  - App names come from `.desktop` files in the settings language (`process.rs:416-417`, test `startup.rs:238`).
  - Process states (`common.rs:619`) and statuses are codes.
- **Opslag:** `services/storage/data.json` `_STR` has 50 nl and 50 en keys (categories `c_*`, clean-up names and descriptions `k_*`/`d_*`, browsers `b_*`).
- **Verkenner:**
  - Places (`files/places.rs:12-60`) and MIME type names (`files/read.rs:607`, `types.json` 47 pairs) follow the language.
  - So do new item names (`mutate.rs:58-73`), the copy suffix "- kopie" / "- Copy" (`mutate.rs:395`), `.desktop` names (`read.rs:448-494`) and the "(2)" conflict numbering (`mutate.rs:245`).
  - Name conflicts are resolved automatically by numbering. There is no Windows "Replace or skip" dialog in this UI, so the only conflict texts are the host's error messages.

**Dutch only on the host.**

Columns:
- **Unique literals:** unique sentence-like Dutch literals outside tests, found by heuristic, so the real total may be higher.
- **Exact nl match in H:** how many of them equal an nl row in H's `i18n/host/*.tsv` exactly, which means the English is already written.

| Rust area | Unique literals | Exact nl match in H | Examples (file:line) |
|---|---|---|---|
| `files/` | 95 | 60 | `dry.rs:57` "Er bestaat al een item met de naam ‘{name}’ in deze map."; `read.rs:129` "Deze map bestaat niet (meer)."; `archives.rs:39-58`; `trash.rs:509`; `clipboard.rs:66-90`; `properties.rs:158` "Alle van het type " (a sentence built from parts); `actions.rs:53` |
| `services/sysmon*` | 34 | 33 | `actions.rs:95,133-145` (End task refusals), `:225,229` (efficiency refusals inside `results[].error`), `:522-694` (restart refusals), `:748` (terminal restart question), `:842-892`; `startup.rs:190-223` |
| `services/storage*` | 22 | 21 | `cleanup.rs:192-254` ("Sluit eerst {}."); `scan.rs`; `common.rs:332-344`; `categories.rs:105` |
| `services/index*` (Alles) | 36 | 27 | `index.rs:145,236` (`index_restarting`), `:255` (an `index.status` event `message`, sent once in the language of that moment), `:284-468`; `query.rs:39,75,881,897` (regex and date errors that mention "dm:vandaag"); `engine.rs:161-358`; `cache.rs` |
| `picker.rs` (Codex) | 13 | 0 | `:64` "Er zijn al vier bestandskiezers geopend.", `:245-309`, shown in the picker footer |
| `transfers.rs` (Codex) | 12 | 0 | `:30-199` "Tab-overdracht is verlopen; de brontab blijft behouden", shown by `tabtransfer.js` `fail()` |

The Alles query parser already accepts both languages: `query.rs:823-824` takes `today|vandaag` and `yesterday|gisteren`. So H's `q_todayQuery: 'dm:today'` works against G's host.

### F5. (c) Codex's changes in this area (`G/vendor/ui-patches.json`)

| Patch (ui-patches key) | File and lines | New user text? | Follows the i18n pattern? |
|---|---|---|---|
| `alles_progress` | alles/main.js :360-517 (the overlay and index ring are updated in place) | none: the same `t('welcomeBody'/'firstFound'/'st_*')` keys | yes |
| `alles_index_lifecycle` | alles/main.js :420-423 | 1 nl/en pair | **no**: an inline ternary on `getSetting('language')` (ignores `?lang=`, no key); the English says "Everything" (H D5: the name stays "Alles"); it replaces `firstNote` only when `runtime === 'rust'` |
| `window_session` | explorer/main.js :378-398 (save on `win11:close`/`pagehide`), :1946-1975 (restore) | none | n/a; but see F6 on the language-switch reload |
| tab transfer (`cross_origin_tab_transfer`, `native_tab_detach`) | explorer/main.js :364-447, :1943-1994; tabs.js :7, :19-21, :34-36; tabtransfer.js (new) | 1 pair in main.js; 2 pairs and 12 English-only errors in tabtransfer.js; 3 English-only errors in main.js | **no**: `document.documentElement.lang` and a `text(en, nl)` helper instead of `makeT` |
| `clipboard_ordering` | explorer/main.js :43-62, :285-297, :1266, :1733 | none | n/a |
| `native_picker` | explorer/main.js :65-69, :668, :837, :872, :993, :1616; picker.js (new) | 15 keys | **yes** (`makeT`), apart from the `'Opslaan'` comparison (:49) and an incomplete `onLangChange` (:88) |
| `native_explorer_paint` / video posters | explorer/detailspane.js :8-9, :62-76 | none (`alt` = file name) | n/a |
| `interimNative` (mock) | mock/system.js :93-101, :620-651 | 9 Dutch-only mock errors | like the other mocks (Dutch only) |

### F6. (d) H's i18n work compared with G, per file

H's E1 changes in this area were taken from `diff H/dist/i18n-snapshots/E1/ui/js/<f> H/ui/js/<f>`, with E-review included. They depend on H's `core/i18n.js`: `collator()`, `countOf()` (prefers a numeric `count` over a string `n`), `list()` and `i18nReport`. That file has 102 diff lines against G, and G has not changed it, so it can be copied. They also depend on `core/format.js` (24 lines, Intl caching, copy) and on `bridge.js` E1 (`X-Shell-Lang`, `e.key`/`e.params`).

| File | What H changed | Port to G |
|---|---|---|
| taskmgr/actions.js | 3 plurals; `normStatus` by code, no Dutch regex | **copy** |
| taskmgr/main.js | `store.langChanged()` on a switch; `?open=` debug helper | **copy** (needs store.js) |
| taskmgr/table.js, util.js | `collator(COLL_OPTS)`; drop the unused `plural()` | **copy** |
| taskmgr/storage/cleanup.js, explorer.js, sunburst.js | `freedPartly` whole sentence; 2 plurals plus `count` | **copy** |
| taskmgr/hostcall.js | `X-Shell-Lang: lang()`; `makeError(…, from)` copies key/params | **hunk only**: H's file also has the split's `DIRECT` routes |
| taskmgr/store.js | `inflightLang`/`gen` per channel; `langChanged()` | **hunk only**: H's file also uses `direct.live` |
| taskmgr/pages/details.js | `many` becomes a plural | **hunk only**: H's file also has the split's `hungPids` |
| taskmgr/hostbanner.js | no E1 change; H's `downLive` belongs to the split | keep G |
| explorer/i18n.js | 2 plurals; `shortcut` and `extFile` keys | **copy**, then add keys for the Codex texts |
| explorer/util.js | `TYPES_EN` completed (+41); `t('shortcut')`/`t('extFile')`; `collator()` | **copy** |
| explorer/main.js | `saveForReload`/`restoreAfterReload` (sessionStorage, 15 s) on a switch; `?open=` debug helper | **manual merge**, see below |
| explorer/tabs.js, detailspane.js | none in H | keep G |
| explorer/picker.js, tabtransfer.js | not in H | new work |
| alles/actions.js, panes.js, table.js | `count` added; tip examples as `{nl,en}` with `tr()`; `topIndex()` | **copy** |
| alles/i18n.js | `q_todayQuery`, `{link}` in `hintHidden`, 3 plurals, `ty_imageAny`, English tips with English examples | **copy**, plus a new key for Codex's Rust indexer note |
| alles/main.js | `withLink()`; keeps query, filter, sort, focus and top row over the reload; `t('q_todayQuery')`; `count` | **manual merge**: H's hunks are at G0 lines 23-43, 73-108, 372, 404, 434 and 591-642; Codex's at G0 356-424 and 443-517. They interleave at the welcome panel (chip :384, hint :434), which is mechanical but not automatic |
| disks/i18n.js, properties.js, util.js | 3 plurals plus `count`; `b_format…b_other` whole phrases; "Disk Management" | **copy** |
| disks/main.js | `load()` after a switch; `loadingLang`/`loadGen`; debug menus | **copy** |
| disks/actions.js, numparse.js, overview.js; taskmgr pages other than details; explorer dialogs, dnd, fileview, navpane, pages, toolbar; alles highlight and options; preview/wm.js | none | already identical |
| mock/settings.js | E0a: `ENUMS` `language ∈ {nl,en}` | **copy** |
| mock/system.js | none in H | keep G |
| other mock/* | none (identical) | none |

**The Verkenner merge needs one ordering decision.** G's `start()` (main.js:1943-1994) restores from a tab ticket first, then from Rust's `explorerSession`, then from the URL. Rust's session is fixed when the window is created (`window.rs:482`). So in the native runtime, `location.reload()` after a switch brings back the *window-creation* session, or the URL place when the window opened with `?path=`/`?place=`. Any tabs the user opened since then are lost.

H's `restoreAfterReload()` should run first, before ticket and session. A ticket in the reloaded URL has already been committed or released, so the merge must make sure it is not claimed a second time. The `pagehide` handler (main.js:398) also fires on the reload and saves the current tabs to Rust. That is harmless.

**Also outside this area:** H's E1 English for these apps was reviewed later (E-review: "sign-in screen", quotes as “ ”). Copying H's *current* files gets that wording. In H, Dutch changed only for a count of 1, recorded in H's `tests/i18n/BASELINE-CHANGES`. Examples: "1 proces" instead of "1 processen", and "1 item kon niet …" instead of "1 item(s) konden niet …".

### F7. Mocks (brief)

- **Mock errors are Dutch only.** `mock/*.js` has 115 `err(code, '…')` sites, all Dutch. H did not translate them either.
- **Some mocks already follow the language.** `mock/files.js` follows `settings.language` (`LANG`, :93-96 and :846), and so does `mock/sysmon.js:772`. `mock/index_search.js` understands both query languages.
- **Mocks only matter in mock mode:** the screenshot and text-dump checks, and `?mock=1`.

## Work needed (agent-hours)

**UI, this area** (this assumes U0 lands first or alongside):

| # | Item | Size |
|---|---|---|
| U0 | *Dependency, core owner:* copy H's `core/i18n.js` and `core/format.js`; port `core/bridge.js` E1 (`X-Shell-Lang`, `e.key`/`e.params`, the bridge's own texts), keeping Codex's one-line icon patch; copy `mock/settings.js` | 1.5 h |
| U1 | Taakbeheer: copy 7 files (actions, main, table, util, storage/cleanup, storage/explorer, storage/sunburst); port the E1 hunks into hostcall.js, store.js and pages/details.js | 1.5 h |
| U2 | Schijven: copy 4 files (i18n, main, properties, util) | 0.5 h |
| U3 | Alles: copy 4 files (actions, i18n, panes, table); merge main.js; replace :420-423 with a key such as `firstNoteNative` (nl: Codex's text; en: "…when you open Alles again.") | 2 h |
| U4 | Verkenner: copy i18n.js and util.js; merge main.js (reload state before ticket before session; debug helper) | 2.5 h |
| U5 | Codex strings: tabtransfer.js `text()` becomes keys in explorer/i18n.js (`tabMoveFailed`, `tabKeptSource`); the 12 validation errors and main.js :367/:439/:1987 become one generic localised message (details to the console); main.js :368 becomes a key; picker.js: a `saveAs` key and a complete `onLangChange` | 1.5 h |
| U6 | Regenerate `vendor/ui-sha256.json`; add the changed files and an `english_option` reason to `vendor/ui-patches.json`; run `scripts/verify-ui.py` | 0.3 h |
| U7 | Checks: port H's `tools/i18n/{tok,jsdicts,scan_ui}.mjs` and the UI part of `check.py`/`ratchet.py` (they already run on G through a root that points `ui` at `vendor/ui`); a G allowlist; nl/en text dumps of the Taakbeheer, Verkenner, Alles and Schijven surfaces in mock mode, compared before and after (H's `tests/i18n/dump.html`, `freeze.js`, `serve.py`) | 3 h |

**Rust host, the texts these apps show** (overlaps with the Rust-services map):

| # | Item | Size |
|---|---|---|
| R1 | Per-request language: read `X-Shell-Lang` in `runtime.rs` (around :256), carry it to `ServiceContext::language()` and `files::Context` (thread-local or an argument); add optional `key`/`params` to `ApiError` | 2.5 h |
| R2 | One Rust catalog helper (embed TSV with `include_str!`, `L(lang, key, params)`, a parity unit test), or extend the existing `_STR` JSON tables | 1.5 h |
| R3 | sysmon: 34 unique texts (33 have English in H `sysmon.tsv`), including `results[].error` and the restart question | 2 h |
| R4 | files: 95 unique texts (60 in H `files.tsv`/`files_launch.tsv`); fix the composed `properties.rs:158` | 5 h |
| R5 | storage: 22 (21 in H) | 1 h |
| R6 | index/Alles: 36 (27 in H); render `index.status.message` and `roots[].error` when sent, and send again after a language change (as H's E3b does) | 2.5 h |
| R7 | picker.rs (13) and transfers.rs (12): no H equivalent, so the English has to be written | 1.5 h |
| R8 | disks: check that `map_error` passes raw udisks messages through; decide on raw self-test and attribute names | 0.5 h |

Totals:
- **UI:** about 12.8 h, including U0.
- **Host:** about 16.5 h.

U1–U6 can be done before any host work. An English UI then still shows Dutch host errors until R1–R7 land.

## Risks

- **Dutch must not change.**
  - Copying H's files changes Dutch text for a count of exactly 1 (14 plural objects). The Dutch in `alles/i18n.js` `trashFailedBody` and `hintHidden` changes too.
  - H recorded these as intended. Robin's rule is "Dutch stays unchanged", so he should accept these changes, or they should be left out.
  - Without the U7 dumps, an nl regression is invisible.
- **Alles progress animation.** The Codex patch `alles_progress` exists to keep the progress panel's animation and height stable. A careless merge with H's E1 changes to alles/main.js can bring back the re-render on every status event.
- **Three restore sources in Verkenner.** A wrong order gives duplicated tabs, a ticket that is claimed twice, or `pendingSelect` lost after a reload.
- **The Windows 11 look.** No English rendering of these apps has been checked in G. Codex changed `css/explorer.css` and `css/components.css`, and English labels have different widths (status bar, picker footer, Taakbeheer columns). H's E1 checks were done against H's CSS.
- **Mixed language until the host work lands:**
  - In English: Dutch infobars from files, sysmon, storage, index, picker and tab transfer.
  - In Dutch, today: Codex's English-only tab-transfer errors.
- **Packaging.** Missing the manifest update breaks `scripts/package.py:17`, which runs `verify-ui.py`.
- **Other workflows.** Other agents (the `notes/claude/map/merge-trial` directory exists) may edit the same `vendor/ui` files. G is now under git, so merges are visible, but `explorer/main.js` and `alles/main.js` are hot spots.
- **Memory.** The U7 text dumps need a browser engine, and only about 3 GB is free. Run one surface at a time.

## Not checked

- **Nothing was rendered or run.** No browser, no Rust build, no live language switch. I did not verify:
  - that `settings.changed` reaches every standalone Rust app window;
  - how fast Taakbeheer and Schijven refetch;
  - the window-session behaviour on reload. That point is inferred from `window.rs:482`, where the session is written into the init script as a one-time `format!`.
- **The Rust literal counts are heuristic.** They take sentence-like strings with common Dutch words and skip `#[cfg(test)]` blocks. They miss short labels and texts built with `format!` from neutral parts. I did not read every file under `files/`, `services/` or `picker.rs` in full.
- **CSS and text widths** for English in these apps.
- **The quality of H's English** was not reviewed again against its glossary.
- **Other areas:** Settings, defaultapps, installer, shell surfaces, `extension/`, and `core/*` beyond the diffs quoted here.
- **The Python backend in `vendor/host`.** The default is native (`nativeDefaultNoPython: true`), and the backend proxy (`runtime.rs:390-405`) would not forward `X-Shell-Lang`. I did not check whether that path still matters.
- **Mock files** were only counted, not compared text by text.
