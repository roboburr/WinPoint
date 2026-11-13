# Reviewed native software operations

`Software` is loaded on the first `software.*` request. It accepts opaque desktop IDs or absolute installation-file paths, never a command or caller-selected package name. Installed desktop files are resolved by GIO; trusted exports and package ownership determine the provider. User launchers take precedence over system-package ownership, including a symlink to a system launcher.

Public methods:

- `software.inspect {path}` reads AppImage/DEB/Flatpak-ref/Flatpak-bundle/Snap metadata without launching the downloaded application.
- `software.uninstallInfo {id}` returns the provider, exact action, explanation and capability.
- `software.prepare {action:"install",path}` or `{action:"uninstall",id}` returns the definitive review plan and `info` metadata. An install captures and hashes the chosen bytes. Unsupported plans have `canExecute:false` and null `planId`.
- `software.execute {planId}` claims a ten-minute, single-use plan. The UI must show and explicitly confirm that returned review before this call. Removal re-resolves ownership and refuses a changed target.
- `software.status {jobId}`, `software.jobs {}` and `software.cancel {jobId}` share persisted jobs across hosts. Jobs is an array, newest first, max100. `software.changed` carries one raw job snapshot; observers in other hosts also poll status.

Job states are queued/running/authenticating/completed/failed/cancelled/interrupted. Progress may be null. A cancellation request remains active until the provider proves the terminal state. Provider success after a cancellation request remains completed. Unknown outcomes are interrupted. `dryRun:true` means completed preview without app changes. `createdAt` uses Unix seconds.

Private plan/job files use owner-only permissions, no-follow opens, atomic rename/fsync and flock. A global lock permits one software mutation at a time. Staging is bounded to 20 pending captures/8GiB with a 4GiB per-file maximum and a free-space reserve. Expired/orphan captures are pruned; active operations retain their stages. Terminal history retains at most200 entries/30days. Copies and hashes stream through128KiB buffers. Metadata helpers are limited to two processes, each256MiB address space/10CPU seconds, with bubblewrap network isolation when available and a separate resource-limited helper fallback.

Providers:

- AppImage installs only an owned executable copy and generated desktop launcher, tracked in a private manifest. It never runs an AppImage to inspect or install it. Removal deletes only the registered image/launcher and leaves unexpected files and the original download. An unmanaged user desktop file supports removeShortcut only.
- PackageKit uses the installed Ubuntu D-Bus ABI, typed InstallFiles/RemovePackages, interactive OS authorization and terminal Finished signals. It simulates first, rejects any unexpected/protected package removal, and never enables dependency removal or autoremove. Essential/required/base-system packages are protected.
- Snapd uses its Unix REST socket, X-Allow-Interaction for OS authorization, bounded response parsing, streamed multipart sideload, asynchronous change polling and abort. Unsigned-local/classic/devmode implications are included in the review notes. Removal uses purge:false.
- Flatpak uses fixed argv, an exact app ref and its authoritative user/system scope. No shell, password collection, delete-data, force-remove, unused/all removal or caller-defined command is used. Output is bounded and cancellation sends SIGINT, then waits for the actual exit.

The standalone native fixture tests live under `work/native-software-test` in the development workspace. They use generated AppImages/packages, private HOME/XDG/D-Bus, fake PackageKit/Snapd/Flatpak providers and explicit private file copies. Real system installation/removal and distribution-specific polkit dialogs are intentionally not exercised. The HTTP proof uses the production Rust binary with a generated non-runnable AppImage entirely under a temporary home. Python in this test directory is tooling, not the runtime.

Protocol references:

- Installed `/usr/share/dbus-1/interfaces/org.freedesktop.PackageKit{,.Transaction}.xml`; [PackageKit transaction documentation](https://www.freedesktop.org/software/PackageKit/doc/dbus-org.freedesktop.packagekit1.Transaction.html).
- [Canonical snapd REST reference](https://staging-api.snapcraft.io/docs/snapd-api), [Snapcraft REST authentication guide](https://snapcraft.io/docs/how-to-guides/snap-development/use-the-rest-api/).
- [Flatpak command reference](https://docs.flatpak.org/en/latest/flatpak-command-reference.html), including exact refs, scope flags and data-preserving uninstall options.

These APIs enable the Installer and Settings/Start removal flows. Frontend review, drop/browse path provenance, job presentation and final installer integration are validated separately.
