# GW7: doc-string changes for `host/src/api-methods.json` (for WU-FIX / the coordinator)

`api-methods.json` belongs to WU-FIX in this round; GW7 did not edit it. These are the `doc` strings GW7's code now
answers to (one sorted ASCII line in the file; `’`/`→` as `’`/`→` like the existing entries).

| name | dangerous | doc (new) |
|---|---|---|
| `update.release.start` | true | `{anyway?:bool} → {started:true, already?:true}: Ubuntu's preconditions in order (not_offered; updates_pending with data {count, held}; reboot_first; busy; hold — anyway:true only after the page's warning and only for a hold verdict whose release has allowAnyway), then upgrade.json, temporary linger and pkexec do-release-upgrade in a terminal through the wrapper (Role::App, app.slice); already:true while our start is still on its way` |
| `update.release.attach` | true | `{} → {started:true}: pkexec release-upgrade-attach in the terminal wrapper (always a password), only while Ubuntu's screen session runs (not_running); no_helper with the manual command without the root helper` |
| `update.status` | false | append to the current text: `; release (GW7, additive): state is the card (running, waiting, startFailed, ended, updatesFirst, rebootFirst, none after an upgrade that waits for its reboot, else the verdict), verdict (GW6's state under it), startRc, reasons (startFailed: the start failure), verdictReasons, heldNames, attachable` |

Error codes `update.release.start` can answer: `not_offered` (also for an `ubuntuHold` verdict), `updates_pending`,
`reboot_first`, `busy`, `hold` (every verdict other than ok/warn, unless hold + allowAnyway + anyway), `no_terminal`,
`test_seam`, `failed`; and the guard's `update_busy` (`update.guard.upgrade`) while Ubuntu's upgrader runs.
`update.release.attach`: `not_running`, `no_helper`, `no_terminal`, `test_seam`, `failed`.
