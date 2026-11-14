The Rust FileChooser implementation
=================================

`win11-file-portal` owns `org.freedesktop.impl.portal.desktop.win11shell` and
implements OpenFile, SaveFile and SaveFiles at `/org/freedesktop/portal/desktop`.
The existing xdg-desktop-portal frontend keeps its own name and remains responsible
for identifying callers and granting sandboxed applications document access.
Selection uses the native GTK 4 Explorer in picker mode (`win11-gtk-explorer
--picker-stdio`, one child process per request, on the portal's own backend) and the
host's picker validation (picker.rs). The backend creates or truncates no selected file.

The backend is activated through D-Bus. It accepts at most four requests and four
filesystem workers. Request.Close, caller disappearance, user cancellation, invalid
options and idle termination release the request and end its picker process. It exits
after 15 idle seconds; it does not launch an indexing worker as part of startup. The
process itself runs no GTK: a GIO service application, its runtime and HTTP server.
Without win11-gtk-explorer (a broken install) a request fails and stderr says so.

Typed options include glob/MIME filters, current filter, multiple/folder selection,
NUL-terminated filesystem hints, current filename, custom accept label, modal state,
combo/boolean choices and read-only selection. SaveFiles chooses a directory,
returns collision-free candidates in the original order, and preserves non-UTF-8
suggested basenames as file URIs. The picker child parents itself to a Wayland exported
parent (GTK 4, xdg-foreign). X11 window handles, which GTK 4 cannot parent to, and
invalid/unavailable external parents fall back to an unparented chooser, as in the
GTK reference backend.

Registration precedes acquisition of the implementation name, without replacement
flags. A short registration main loop services the frontend's initialization before
the application registers and starts, preventing cold-activation recursion. Only this
backend process disables GIO's portal lookup (GTK_USE_PORTAL=0) and its picker child
GTK's (GDK_DEBUG=no-portals); app launch boundaries restore the original
GTK_USE_PORTAL setting. No desktop-wide theme override is performed.

Backend selection is a FileChooser-specific portals.conf preference. Applications
must use a desktop portal (through their toolkit or native API); custom application
choosers cannot be replaced by selecting a portal backend. Installer/browser
configuration is handled separately and must preserve other portal interfaces.

The native launch boundaries opt newly spawned GTK callers in through their child
environment only when the user's owned native registration is complete and the
FileChooser preference selects Win11Shell. Explicit GTK_USE_PORTAL values remain
unchanged. Applications opened from the chooser receive the original parent
setting rather than its internal GTK override. Existing processes and D-Bus
services can retain their own environment; toolkit/browser portal preferences may
still be needed. No session environment or browser profile is rewritten.

Per-user activation includes a Type=dbus service in app.slice. A new login, or an
explicit user-manager daemon-reload and safe desktop-portal restart, loads newly
installed unit/selection files. The installer does not interrupt active portal
sessions. Uninstall restores the original selection exactly when untouched, and
preserves later unrelated edits or an explicitly changed backend preference.

Protocol references:
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.FileChooser.html
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.Request.html
- https://flatpak.github.io/xdg-desktop-portal/docs/window-identifiers.html
- https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html

Tests: portal::tests covers typed model/result validation without GUI or writes.
`work/native-portal-test` holds the isolated D-Bus/headless GNOME protocol tests,
real frontend activation tests, Wayland/X11 parenting tests and a real Zen Browser
Save Page As test using a new private profile and a local test page. All runtime,
settings, service definitions and selected test files are confined to /tmp.
