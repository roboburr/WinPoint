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
