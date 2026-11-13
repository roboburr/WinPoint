# GW6: doc strings for `host/src/api-methods.json` (for WU-FIX / the coordinator)

`api-methods.json` belongs to WU-FIX in this round, so GW6 writes the doc-string changes it needs here. Both entries
already exist (GW2); `dangerous` stays `true` for `update.release.compat`. Only the `doc` text changes.

## `update.release.compat` (replace the doc)

```
{refresh?:bool} → {started:true} | {started:false, skipped:'notOffered'|'hold'|'recent'|'offline'|'metered'|'running'}: the compatibility check of the offered Ubuntu release (compat.rs; cheap verdict first: a releases.json status other than tested, or a GNOME major the installed extension's shell-version lacks, holds without running anything; else the private apt tree, at most once per release per 7 days, a failed check after 1 h or with refresh); the verdict arrives with update.changed in release.state
```

(Dry run: `{state:'unknown', dryRun:true}`, unchanged, from `update.rs`.)

## `update.status` (append to the doc, after the GW5 fields WU-FIX documents)

```
release (GW6, additive): state none|checking|ok|warn|hold|ubuntuHold|blocked|unknown (GW7 adds updatesFirst|rebootFirst|running|waiting|startFailed|ended), verdict (GW6's state, never overridden by GW7), reasons [{code, key, params, message}] (update.compat.*, rendered per call), gnomeMajor (the archive's major, null when only releases.json gave one), allowAnyway (releases.json, per codename), compat {running, needed, checkedAt} (needed: the page asks update.release.compat), upgraderNl (ubuntu-release-upgrader has a Dutch catalogue)
```

## Nothing else

No new method and no new argument: the card calls `update.release.start {}` (no `anyway` key, because
`update.rs`'s allowlist has none; see notes/claude/wu/gw6.md "Contract with GW7"), `update.release.attach {}`,
`update.release.ubuntuHold {on}`, `update.release.compat {refresh?}`, `update.install {}`, `system.battery.get`,
`files.open {path}` and `apps.openUri {uri}`, all existing entries.
