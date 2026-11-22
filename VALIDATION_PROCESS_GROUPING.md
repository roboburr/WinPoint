# Application process grouping, 0.9.5

The user showed an app already present under Apps, with its helpers in a second background group named
"Win11 GNOME external". A read-only query of the running 0.9.4 backend confirms separate, disjoint PID lists:
the browser's window process is an app; its helpers are in the launcher service group. No PID was counted twice.
The large groups on that machine contain ChatGPT and Zen processes. The taskbar process also hosts Start and the
desktop, so its displayed memory includes their wallpaper, UI state and rendering buffers.

The cause is the service boundary in `resolve`: it rejected every ordinary service before following the parent
process, even when the same service already contained an identified application window.

The fix applies to identified apps generally, without a brand or launcher whitelist. A service containing one app
owns its ordinary same-user helpers, including reparented helpers. If several apps share a service, descendants
follow their own parent tree within that service; ambiguous orphans stay unassigned. Full cgroup paths prevent
unrelated service instances with the same unit name from merging. Other users and protected unassigned processes
remain separate. A native app's own backend is associated with its window too, including the Task Manager backend
that provides this process list. Process protection flags and action checks are unchanged.

The first three new behavioral regressions fail against the old service barrier, while ten existing regressions
pass. The fixed implementation passes **14 process tests**, including the additional self-backend case. They cover
distinct apps, shared services, reparented helpers, cgroup boundaries, user boundaries and protected processes.
API-level cases use real helper processes with controlled service/window identity; they verify app membership,
exactly one group per PID, and a group memory total equal to the sum of its process rows.

This changes attribution, not how much RAM the applications allocate. The process memory estimate remains
`statm` resident pages minus shared pages. Consolidating rows does not reduce system memory use. The installed
session is not changed by these tests; actual grouping after the user installs the update remains to be checked.

## Follow-up: separate launch scopes, 22 November 2026

A read-only inventory found the main ChatGPT process in a generic Chromium scope, with its helpers in a different
external launcher service. The original service-owner rule could not join those two groups. The grouping code
now resolves a registered window identity or a unique executable before falling back to the scope name. A
window-confirmed executable also identifies helpers in a different unit, including reparented helpers inside an
unambiguous app-specific installation directory. This is general application recognition, without a per-brand fix.

Shared executables and directories are deliberately ambiguous. Registered PWA and wrapper window identities are
preserved; generic interpreters and shared binary/AppImage directories cannot claim unrelated applications.
Unknown external processes fall back to their executable or individual PID, rather than the old WinPoint launcher
description. New launcher services use the translated description “External apps”. The memory estimate and
protected-process action checks are unchanged.

The actual host library passed 45 targeted tests, with one real-user-manager test deliberately ignored. These
include eleven new grouping regressions, a three-process split-scope fixture, single membership and the summed
memory invariant, the new launcher description, five bundled-icon selection cases and existing isolation/restore
checks. The private runner used its own HOME, XDG directories, keyfile settings and session bus; no guarded system
tool was called. An invalid negative test fixture was corrected to include a canonical user-service cgroup path
before the successful run; production grouping logic did not change in that correction.

The icon regression was also traced to a naming mismatch: the installer registers Win11Shell and Win11Shell-dark,
while the former theme selector looked only for Win11 and Win11-dark. The selector now chooses the installed
matching alias first and retains legacy themes as a fallback. Seven additional private checks of the actual icon
installer verified alias registration, collisions, upgrades, uninstall restoration and preservation of user themes.

These are source and isolated-test results. No new host binary has been installed or loaded into the running
session, and the mini-pc has not been accessed.
