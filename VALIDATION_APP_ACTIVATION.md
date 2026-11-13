# App activation — 0.7.0

A user launch from the original Start/taskbar/app UI now obtains a fresh GNOME compositor startup token. The separate Rust host receives both `DESKTOP_STARTUP_ID` and `XDG_ACTIVATION_TOKEN`. Tokens are issued with `global.display.get_current_time_roundtrip()` because D-Bus handlers have no Clutter event and `global.get_current_time()` returned zero. The parent’s already-consumed startup token is removed.

Installed desktop and default-file/URI launches retain the existing sanitised environment. A native GIO launch-context subclass also returns the token through the `get_startup_notify_id` virtual method: environment variables alone do not carry activation platform data to an already-running D-Bus app. gio 0.18 lacks AppLaunchContext subclass bindings, so a small private GObject subclass overrides that method, stores an immutable owned token, and leaves every other parent method intact. Its unit test verifies token identity, separate instances and the fallback without a valid token.

No notification setting or global focus policy was disabled. Unknown app names and arbitrary desktop-file paths cannot request this token through the new bridge method.

## Real GNOME/GTK/WebKit validation

`work/app-activation-test/run.py`, run under `dbus-run-session`, passed **10 groups** with the actual 0.7.0 ELF files in a private HOME/XDG tree and GNOME 50.1 Wayland at 1440×900:

- Native `apps.launch`: Explorer, Settings, Task Manager, Everything and Disks each mapped and took compositor focus from an already painted/focused Explorer window.
- The separate `shell.openApp` launch route also opened Everything in the foreground.
- An ordinary installed GTK3 desktop fixture launched through native GIO `apps.launch` received focus.
- An already-running D-Bus GTK application service received both `activation-token` and `desktop-startup-id` in its actual `before_emit` platform data and opened its window in the foreground.
- All eight app launch cases left no ready notification and produced **zero visible banner updates**, observed through `Main.messageTray._bannerBin` whenever its opacity became greater than zero.
- Unknown apps, uninstalled IDs and arbitrary desktop paths were rejected; repeated valid launches received different compositor tokens.

Some GTK3 clients briefly emit `window-demands-attention` when mapping. The trace shows focus following within a few milliseconds and the corresponding source disappearing before a banner is visible. This is not reported as zero internal attention signals.

Separate pre-fix probes reproduced the exact Settings/Everything “is ready” notification while Explorer retained focus. A token issued with timestamp zero did not fix it; a token issued with the compositor timestamp did. Another private D-Bus probe showed empty activation platform data with environment variables alone, and both token keys after the native context override.

The compact report `validation/app-activation-0.7.0.json` records all five binary hashes, the installed bridge hash, and actual focus/attention timing. Source fixtures and private logs remain in the workspace; no user apps, indexer cache or live session were modified by these tests. Concurrent app launches and deliberate user focus changes during a slow launch were not exercised.
