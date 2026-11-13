# Everything ownership and lifetime — 0.7.0

The GNOME backend receives `WIN11_GNOME_APP` from its Rust host. Only `alles` can start an index worker. Panel, the resident Start menu, Desktop, Explorer, Settings, Task Manager and Disks cannot start one through `index.status`, `index.search`, direct startup, or an inherited `WIN11SHELL_INDEX_AUTOSTART=1`. The original non-GNOME host, without this role, keeps its behavior.

Start retains its original app search and existing fallback to recent files; its file-search request no longer creates a filesystem index. Everything runs as a unique application, forwards repeated launch parameters to its existing window and owns one backend/indexer. Closing that window stops its worker; the on-disk index remains available for the next opening.

The first-run note in the original Everything UI reflects this policy for the Rust host, in Dutch and English. The reference Python UI retains its original background-indexing note. CSS is unchanged.

## Validation

**13 service lifecycle groups passed** (`work/index-lifecycle-test/run.py`): eight non-Everything roles blocked every startup route, Everything searched 200 private fixture files through one reused worker, closing stopped it while retaining its persisted index, reopening loaded the cache and stopped again, and the original host policy remained available.

**6 GNOME integration groups passed** (`work/alles-app-lifecycle-test/run.py`) with the actual 0.7.0 Rust binaries and GNOME 50.1 Wayland at 1440×900:

- Warm Panel, Start and Desktop created no indexer even with forced autostart; status/search calls did not create one.
- Opening Everything started exactly one worker and found the 200 fixture files.
- A second command-line launch delivered `needle-020` to the original search field, with one window, backend and indexer PID.
- Closing Everything stopped the worker and retained the cache. Subsequent shell status calls did not restart it.

All roots/cache and HOME/XDG/session D-Bus paths were private. No installed user app or production cache was stopped or deleted by these tests. `validation/index-lifecycle-0.7.0.json` records the checks, tested binary hashes and Python service hash. Memory limits and app cgroup separation require separate validation; these tests do not claim a hard memory bound.
