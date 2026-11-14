#!/usr/bin/env python3
"""Static audit of host/src Rust tests (GE0b).

Lists every #[test] function with its module path, whether it is #[ignore],
and which risky patterns appear in its body: process spawns, D-Bus/GNOME
calls, HOME/XDG access, signals, pkexec/systemctl, absolute live paths.
Read-only; prints TSV to stdout.  Usage: audit_tests.py [G]
"""
from __future__ import annotations
import re
import sys
from pathlib import Path

G = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parents[2])
SRC = G / "host/src"

RISKY = {
    "spawn": r"Command::new|\.spawn\(|process::Command|launch::spawn|spawn_",
    "gnome_call": r"gnome::call|gnome::\w+\(|org\.gnome\.Shell",
    "dbus": r"bus_get|BusType::|DBusConnection|DBusProxy|dbus_call|call_sync|system_bus|session_bus|SessionBus|SystemBus",
    "home": r"\bHOME\b|home_dir|XDG_|dirs::|config_dir|data_dir",
    "env_set": r"set_var|remove_var",
    "signal": r"libc::kill|kill\(|SIGTERM|SIGKILL|signal",
    "priv": r"pkexec|systemctl|systemd-run|loginctl|gsettings|dconf",
    "live_path": r"/run/user|/home/robin|\.local/share|\.config/win11|/sys/fs/cgroup",
    "gtk": r"gtk::init|gtk::",
    "net": r"ureq::|TcpStream|tiny_http|http://",
}


def strip_comments_strings(text: str) -> str:
    return text  # patterns are intentionally matched in strings too


def find_tests(path: Path):
    text = path.read_text()
    lines = text.splitlines()
    out = []
    i = 0
    while i < len(lines):
        if re.match(r"\s*#\[test\]", lines[i]):
            start = i
            ignored = False
            j = i + 1
            while j < len(lines) and not re.search(r"\bfn\s+\w+", lines[j]):
                if "#[ignore" in lines[j]:
                    ignored = True
                j += 1
            if j >= len(lines):
                break
            m = re.search(r"\bfn\s+(\w+)", lines[j])
            name = m.group(1)
            # body by brace matching from line j
            depth = 0
            k = j
            seen = False
            body = []
            while k < len(lines):
                line = lines[k]
                body.append(line)
                # crude: ignore braces inside string literals
                clean = re.sub(r'"(?:\\.|[^"\\])*"', '""', line)
                clean = re.sub(r"'(?:\\.|[^'\\])'", "''", clean)
                clean = re.sub(r"//.*", "", clean)
                for ch in clean:
                    if ch == "{":
                        depth += 1
                        seen = True
                    elif ch == "}":
                        depth -= 1
                if seen and depth <= 0:
                    break
                k += 1
            if any("#[ignore" in lines[x] for x in range(max(0, start - 3), j)):
                ignored = True
            hits = sorted(key for key, rx in RISKY.items() if re.search(rx, "\n".join(body)))
            out.append((path.relative_to(G).as_posix(), start + 1, name, ignored, hits))
            i = k + 1
        else:
            i += 1
    return out


def main() -> None:
    rows = []
    for path in sorted(SRC.rglob("*.rs")):
        rows.extend(find_tests(path))
    print("file\tline\ttest\tignored\trisky")
    for file, line, name, ignored, hits in rows:
        print(f"{file}\t{line}\t{name}\t{'ignored' if ignored else ''}\t{','.join(hits)}")
    print(f"# total={len(rows)} ignored={sum(1 for r in rows if r[3])}", file=sys.stderr)


if __name__ == "__main__":
    main()
