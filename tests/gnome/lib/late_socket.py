"""R3-NATIVE fix round (review m2), the OUTSIDE half of tests/gnome/scenarios/r3-late-socket.py.

Runs next to the harness, outside the sandbox. When the scenario has written <out>/late-ready (the session is up,
long after launch scanned /proc/net/unix), it binds a listening socket in the checkout (tests/logs/r3-late.s), counts
the connections it accepts, and removes it when the run has written <out>/results.json (or after 900 s). Its
report goes to <out>/late-helper.json. Nothing here connects anywhere.

    tools/heavy.sh --min-mb 2560 --tag r3-late -- bash -c 'python3 -B tests/gnome/lib/late_socket.py & \\
        python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/native-release \\
        --scenario tests/gnome/scenarios/r3-late-socket.py \\
        --strace tests/gnome/scenarios/r3-late-socket.out/connect.strace; rc=$?; sleep 3; kill %1; wait; exit $rc'
"""
import json
import socket
import sys
import threading
import time
from pathlib import Path

G = Path(__file__).resolve().parents[3]
OUT = G / 'tests' / 'gnome' / 'scenarios' / 'r3-late-socket.out'
LATE = G / 'tests' / 'logs' / 'r3-late.s'


def main() -> int:
    t0 = time.time()
    for stale in (OUT / 'late-ready', OUT / 'results.json', OUT / 'late-helper.json'):
        stale.unlink(missing_ok=True)
    while not (OUT / 'late-ready').exists():
        if time.time() - t0 > 900:
            print('late_socket: no late-ready marker within 900 s', file=sys.stderr)
            return 1
        time.sleep(0.2)
    LATE.parent.mkdir(parents=True, exist_ok=True)
    LATE.unlink(missing_ok=True)
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(str(LATE))
    server.listen(4)
    bound = time.time()
    hits, lock = [], threading.Lock()

    def accept():
        while True:
            try:
                conn, _ = server.accept()
            except OSError:
                return
            with lock:
                hits.append(round(time.time() - bound, 2))
            conn.close()

    threading.Thread(target=accept, daemon=True).start()
    while not (OUT / 'results.json').exists() and time.time() - t0 < 900:
        time.sleep(0.5)
    try:
        server.shutdown(socket.SHUT_RDWR)
    except OSError:
        pass
    server.close()
    LATE.unlink(missing_ok=True)
    with lock:
        report = {'socket': str(LATE), 'boundAtEpoch': bound, 'boundAfterSeconds': round(bound - t0, 1),
                  'connections': len(hits), 'connectionTimes': hits, 'removedAfterSeconds': round(time.time() - t0, 1)}
    (OUT / 'late-helper.json').write_text(json.dumps(report, indent=1) + '\n')
    print(f'late_socket: bound {LATE} after {report["boundAfterSeconds"]} s, {len(hits)} connections', flush=True)
    return 0


if __name__ == '__main__':
    sys.exit(main())
