# Process isolation and shell memory — 0.7.0

> **History (D-G72, 14 November 2026):** measured with the HTML interface in WebKitGTK of the time. It is gone: File Explorer, Settings, Task Manager and the other apps are native GTK 4 programs now, and WinPoint has no WebKit and no GTK 3 host any more. The Rust services this report checks still exist. The last commit with WebKit is tagged `last-webkit`; see [native/APPS.md](native/APPS.md).

> **History (D-G70, 13 November 2026):** the WebKit Panel, Start and Desktop processes measured here are gone. The native GTK 4 shell `win11-shell` replaces them; its backend runs in the shell slice, `win11-shell` itself is still outside it. See [notes/claude/gtk-native.md](notes/claude/gtk-native.md).

New Panel, Start and Desktop processes run in the dedicated
`app-win11shell.slice`, with a shared 1000 MiB hard memory limit and a 768 MiB high
threshold. The hard limit is below 1 GiB and covers their backends and WebKit
children together. A preloaded Start remains a child of its Panel, preserving
the existing parent-death cleanup.

Other native windows run in independent services under `app.slice`. External
apps start through the small `win11-launch` GIO helper in their own service,
outside the shell slice. `ExitType=cgroup` keeps the app alive after its launcher
helper exits. GIO retains desktop-file field expansion, URI handling, executable
and D-Bus activation; the compositor's fresh activation token is preserved.
No desktop Exec string is interpreted by a shell.

The user manager creates each service before starting the app. Complete cleaned
environments and command overrides are preserved, including fresh activation
tokens. Manager environment entries absent from that cleaned environment are
explicitly removed. `ExecStartEx` disables systemd environment expansion: an
actual-manager regression probe found that plain `ExecStart` expanded `${HOME}`
inside otherwise literal JSON. The corrected path preserves `${HOME}`, `$HOME`,
percent signs, quotes and backticks exactly.

The helper reports GIO success or failure through a private local socket in a
0700 directory. The wait is bounded, and the API does not report a successful
launch when GIO has rejected the entry. The helper links GIO/GLib/GObject and
standard runtime libraries; it does not load GTK/WebKit or start a backend.

Validation on the actual systemd 259 user manager used only short-lived own test
services:

- A shell probe's parent cgroup exposed `memory.max=1048576000` and
  `memory.high=805306368`; a separate app probe ran outside that slice.
- Literal argv, working directory, fresh token overrides and removal of a stale
  token were verified across the real manager boundary.
- An independent helper probe verified literal desktop-file paths, URI arguments,
  environment and working directory, survival of the app after its helper's ACK
  and exit, and propagation of a GIO error through the private ACK channel.
- Three regular Rust unit groups passed. The actual-manager test is ignored by
  the normal unit suite and was explicitly run successfully.

The old live GNOME Panel scope was never capped, moved or stopped: it can contain
unrelated already-running applications. These limits apply to newly launched
0.7.0 processes. Existing windows need the normal session restart to use the new
launch path. No memory exhaustion test was run.

A session bus without a systemd user manager uses a direct launch fallback and
prints an explicit warning; the cgroup memory limit is unavailable there. An
available manager returning a launch error does not silently bypass isolation.

Evidence: `validation/resource-scope-0.7.0.json`; private helper evidence is also
in `work/helper-scope-test`, and the kernel-cgroup probe is retained in
`work/resource-scope-test` in the working directory.
