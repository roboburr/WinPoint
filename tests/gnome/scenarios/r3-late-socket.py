"""R3-NATIVE fix round (review m2): a socket that a host process binds in the checkout DURING a run stops the run.

launch masks the host sockets that exist when it starts, and the guard checks once; the harness's socket watch
(SocketWatch, every 2 s) covers the rest of the run. This scenario only waits: it writes <out>/late-ready, and its
outside helper (tests/gnome/lib/late_socket.py, started next to the harness) then binds a listening socket at
tests/logs/r3-late.s. The scenario only stat()s that path, never connects. The expected outcome is that the harness
stops the session within one or two scans of the bind, and the run ends with exit 5 (EXIT_ABORTED):
results.json socketWatch.appeared names the socket, the scenario has no "survived" check, and late-helper.json
counts 0 connections. If the scenario is still running 60 s after it saw the socket, the check
"the socket watch stopped the session" fails.

Run (from W):
    tools/heavy.sh --min-mb 2560 --tag r3-late -- bash -c 'python3 -B tests/gnome/lib/late_socket.py & \\
        python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/native-release \\
        --scenario tests/gnome/scenarios/r3-late-socket.py \\
        --strace tests/gnome/scenarios/r3-late-socket.out/connect.strace; rc=$?; sleep 3; kill %1; wait; exit $rc'
"""
import os
import stat
import time
from pathlib import Path

G = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent / 'r3-late-socket.out'
LATE = G / 'tests' / 'logs' / 'r3-late.s'


def kind(path: Path) -> str:
    try:
        mode = os.lstat(path).st_mode
    except OSError:
        return 'absent'
    return 'socket' if stat.S_ISSOCK(mode) else 'char device (masked)' if stat.S_ISCHR(mode) else 'other'


def run(h):
    h.check('the late socket does not exist yet', kind(LATE) == 'absent', kind=kind(LATE))
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'late-ready').write_text('ready\n')
    h.log('late-ready written; waiting for the outside helper to bind tests/logs/r3-late.s')
    seen = None
    for _ in range(600):
        if kind(LATE) == 'socket':
            seen = time.monotonic()
            break
        time.sleep(0.1)
    h.record('lateSocketSeen', {'seen': seen is not None, 'kind': kind(LATE)})
    if seen is None:
        h.check('the outside helper bound the late socket within 60 s', False)
        return
    h.log('late socket visible inside (not connected); the socket watch must stop the session now')
    while time.monotonic() - seen < 60:
        time.sleep(0.5)
    h.check('the socket watch stopped the session', False, survivedSeconds=round(time.monotonic() - seen, 1))
