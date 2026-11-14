#!/usr/bin/env python3
"""Acceptance check of the WebKit removal (tag last-webkit, 2026-11-13; decision 10 of that workflow).

    python3 -B tools/check-no-webkit.py [--build-dir DIR] [--no-binaries] [--allowed]

WinPoint has no WebKitGTK, wry, JavaScriptCore or libsoup: not as a build or runtime dependency, not in code, not as
a feature; and the host links no GTK 3 (GLib, GIO and GDK Pixbuf only). Checked over every file git tracks or would
add (git ls-files --cached --others --exclude-standard), the whole repository: the code (host, native, scripts, tools,
extension, tests, the Cargo files), the user-facing docs (README.md, README.nl.md, THIRD_PARTY_NOTICES.md,
RELEASE_PLAN.md, LICENSES, docs), vendor (vendor/host, the Python reference that stays but is not installed,
decision 4) and the records (notes, validation, VALIDATION*.md), which the allow list covers as history:

1. code      the crate-level patterns (CODE below: wry::, webkit2gtk::, WebViewBuilder, WIN11_WEBKIT_APPS,
             window_state …) appear nowhere.
2. cargo     neither lockfile has a wry, webkit2gtk, javascriptcore or soup package or a GTK 3 crate (gtk, gdk, atk and
             their -sys crates), and no Cargo.toml asks for one.
3. vendor    vendor/ui holds no .js, .css or .html file, and nothing names vendor/ui/js, vendor/ui/css or
             vendor/ui/index.html.
4. words     every line that says webkit, javascriptcore, libsoup, soup3 or wry (any case) is covered by a row of
             tools/check-no-webkit.allow: history, Ubuntu's update categories, fixtures, frozen i18n data, the cleanup
             of earlier installs and other programs' names. A row that covers nothing is an error as well, so the
             list shrinks with the tree.
5. binaries  readelf -d of the 19 binaries (BINS in scripts/install.py) in DIR (default tests/build/native-release):
             no NEEDED webkit, javascriptcore, soup, libgtk-3 or libgdk-3; the GTK 4 programs (NATIVE_BINS) need
             libgtk-4.so.1 and the host binaries no libgtk or libgdk at all. ldd must not resolve one of them either.

A hit of checks 1 and 3 can be covered only by a history, frozen-data or fixture row. A reference through the tag
(`git show last-webkit:host/src/window.rs`, "last-webkit:vendor/ui/js/…") or another commit (`git show 418ea594:…`)
is history by construction and never counts. This file and the allow list name the patterns and are not searched.
Standard library plus readelf and ldd; reads only. Exit 0 when everything holds, 1 when anything is found (each hit
is printed), 2 on a usage error or a missing binary or tool (check 5 could not run: the other checks' hits are still
printed, the reason goes to stderr).
"""
from __future__ import annotations

import argparse
import ast
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOW = "tools/check-no-webkit.allow"
SELF = {"tools/check-no-webkit.py", ALLOW}
SCOPE = (".",)   # every file: a docs regression ("apt install … libwebkit2gtk-4.1-dev") counts as much as code
LOCKS = ("Cargo.lock", "native/Cargo.lock")
CLASSES = {"history", "update-category", "fixture", "frozen-data", "legacy-cleanup", "third-party"}
STRICT_CLASSES = {"history", "frozen-data", "fixture"}

CODE = re.compile(r"wry::|webkit2gtk::|javascriptcore::|WebViewBuilder|WebContext|evaluate_script|run_javascript|"
                  r"webkit2gtk-4\.1 >=|WIN11_WEBKIT_APPS|WIN11_DEV_SCRIPT|NATIVE_BRIDGE|window_state|WEBKIT_EXE_DIR")
VENDOR_REF = re.compile(r"vendor/ui/(?:js|css|index\.html)")
WORDS = re.compile(r"webkit|javascriptcore|libsoup|soup3|\bwry\b", re.IGNORECASE)
# A citation of the tag (or of any commit) through git: history by construction (decision 11).
CITATION = re.compile(r"git show [0-9A-Za-z_.-]+:\S+|last-webkit(?::\S+)?")

BANNED_CRATE = re.compile(r"wry|webkit|javascriptcore|soup")
GTK3_CRATES = {"gtk", "gtk-sys", "gdk", "gdk-sys", "atk", "atk-sys", "gtk3-macros"}
BANNED_LIB = re.compile(r"webkit|javascriptcore|soup|^libgtk-3\.|^libgdk-3\.")
HOST_GTK = re.compile(r"^libgtk|^libgdk-")
UI_SUFFIXES = {".js", ".mjs", ".css", ".html", ".htm"}


class Unchecked(Exception):
    """A missing tool or binary: nothing could be checked (exit 2, not the 1 of a hit)."""


@dataclass
class Rule:
    where: str          # allow-list line, "file:line"
    path: re.Pattern
    pattern: re.Pattern | None   # None: every line of the file
    klass: str
    reason: str
    used: int = 0


@dataclass
class Report:
    problems: list[str] = field(default_factory=list)
    allowed: dict[str, list[str]] = field(default_factory=dict)

    def problem(self, check: str, text: str) -> None:
        self.problems.append(f"{check}: {text}")


def glob_re(glob: str) -> re.Pattern:
    """A path glob: * and ? stay within one folder, ** spans folders (a/**/b, a/**)."""
    out, i = [], 0
    while i < len(glob):
        if glob.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif glob.startswith("**", i):
            out.append(".*")
            i += 2
        elif glob[i] == "*":
            out.append("[^/]*")
            i += 1
        elif glob[i] == "?":
            out.append("[^/]")
            i += 1
        else:
            out.append(re.escape(glob[i]))
            i += 1
    return re.compile("".join(out) + r"\Z")


def load_rules(report: Report) -> list[Rule]:
    rules = []
    path = ROOT / ALLOW
    for n, line in enumerate(path.read_text(encoding="utf-8").split("\n"), 1):
        if not line.strip() or line.startswith("#"):
            continue
        where = f"{ALLOW}:{n}"
        cols = line.split("\t")
        if len(cols) != 4 or not all(c.strip() for c in cols):
            report.problem("allow", f"{where}: needs four tab-separated columns (path, pattern, class, reason)")
            continue
        glob, pattern, klass, reason = cols
        if klass not in CLASSES:
            report.problem("allow", f"{where}: unknown class {klass!r} (one of {', '.join(sorted(CLASSES))})")
            continue
        try:
            compiled = None if pattern == "*" else re.compile(pattern)
        except re.error as e:
            report.problem("allow", f"{where}: bad pattern {pattern!r}: {e}")
            continue
        rules.append(Rule(where, glob_re(glob), compiled, klass, reason))
    return rules


def scope_files() -> list[str]:
    out = subprocess.run(["git", "-C", str(ROOT), "ls-files", "-z", "--cached", "--others", "--exclude-standard",
                          "--", *SCOPE], check=True, capture_output=True).stdout
    files = sorted({f for f in out.decode("utf-8", "surrogateescape").split("\0") if f})
    return [f for f in files if f not in SELF and (ROOT / f).is_file()]


def text_lines(rel: str) -> list[str] | None:
    data = (ROOT / rel).read_bytes()
    if b"\0" in data[:8000]:
        return None   # binary (screenshots, fixtures): git grep -I skips these as well
    return data.decode("utf-8", "replace").split("\n")


def covered(rules: list[Rule], rel: str, line: str, classes: set[str] | None) -> Rule | None:
    for rule in rules:
        if (classes is None or rule.klass in classes) and rule.path.match(rel) \
                and (rule.pattern is None or rule.pattern.search(line)):
            rule.used += 1
            return rule
    return None


def check_text(report: Report, rules: list[Rule]) -> int:
    files = scope_files()
    for rel in files:
        lines = text_lines(rel)
        if lines is None:
            continue
        for n, line in enumerate(lines, 1):
            bare = CITATION.sub("", line)
            hits = [(c, rx) for c, rx in (("code", CODE), ("vendor", VENDOR_REF), ("words", WORDS)) if rx.search(bare)]
            if not hits:
                continue
            strict = any(c != "words" for c, _ in hits)
            rule = covered(rules, rel, line, STRICT_CLASSES if strict else None)
            shown = line.strip()
            shown = shown if len(shown) <= 160 else shown[:157] + "…"
            if rule is None:
                report.problem("+".join(c for c, _ in hits), f"{rel}:{n}: {shown}")
            else:
                report.allowed.setdefault(rule.klass, []).append(f"{rel}:{n}: {shown}")
    for rule in rules:
        if not rule.used:
            report.problem("allow", f"{rule.where}: covers no line any more; remove the row")
    return len(files)


def check_cargo(report: Report) -> None:
    for lock in LOCKS:
        names = re.findall(r'^name = "([^"]+)"$', (ROOT / lock).read_text(encoding="utf-8"), re.MULTILINE)
        for name in sorted(set(names)):
            if BANNED_CRATE.search(name) or name in GTK3_CRATES:
                report.problem("cargo", f"{lock}: package {name}")
    manifests = subprocess.run(["git", "-C", str(ROOT), "ls-files", "-z", "--cached", "--others", "--exclude-standard",
                                "--", "Cargo.toml", "*/Cargo.toml"], check=True, capture_output=True).stdout
    for rel in sorted({m for m in manifests.decode().split("\0") if m and (ROOT / m).is_file()}):
        section = ""
        for n, line in enumerate((ROOT / rel).read_text(encoding="utf-8").split("\n"), 1):
            head = re.match(r"\s*\[([^\]]+)\]", line)
            if head:
                section = head.group(1)
                dotted = re.search(r"dependencies\.([A-Za-z0-9_-]+)$", section)
                names = [dotted.group(1)] if dotted else []
            elif "dependencies" in section:
                # a renamed dependency (gtk = { package = "gtk4", … }) is its package
                renamed = re.search(r'package\s*=\s*"([^"]+)"', line)
                dep = re.match(r"\s*([A-Za-z0-9_-]+)\s*=", line)
                names = [renamed.group(1)] if renamed else [dep.group(1)] if dep else []
            else:
                names = []
            for name in names:
                if BANNED_CRATE.search(name) or name in GTK3_CRATES:
                    report.problem("cargo", f"{rel}:{n}: dependency {name}")


def check_vendor(report: Report) -> None:
    ui = ROOT / "vendor/ui"
    for folder, dirs, files in os.walk(ui):
        dirs.sort()
        for name in sorted(files):
            if Path(name).suffix.lower() in UI_SUFFIXES:
                report.problem("vendor", f"{Path(folder, name).relative_to(ROOT)}: no HTML, JS or CSS under vendor/ui")


def install_lists() -> tuple[tuple[str, ...], tuple[str, ...]]:
    tree = ast.parse((ROOT / "scripts/install.py").read_text(encoding="utf-8"))
    found = {}
    for node in tree.body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name) \
                and node.targets[0].id in ("BINS", "NATIVE_BINS"):
            found[node.targets[0].id] = tuple(ast.literal_eval(node.value))
    return found["BINS"], found["NATIVE_BINS"]


def check_binaries(report: Report, build: Path) -> str:
    for tool in ("readelf", "ldd"):
        if not shutil.which(tool):
            raise Unchecked(f"{tool} not found")
    bins, native = install_lists()
    missing = [b for b in bins if not (build / b).is_file()]
    if missing:
        raise Unchecked(f"{build} lacks {', '.join(missing)} (BINS in scripts/install.py)")
    env = {**os.environ, "LC_ALL": "C"}
    gtk4 = 0
    for b in bins:
        path = build / b
        dyn = subprocess.run(["readelf", "-d", "--wide", str(path)], capture_output=True, text=True, env=env)
        if dyn.returncode:
            report.problem("binaries", f"{b}: readelf -d failed: {dyn.stderr.strip()}")
            continue
        needed = re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", dyn.stdout)
        for lib in needed:
            if BANNED_LIB.search(lib):
                report.problem("binaries", f"{b}: NEEDED {lib}")
            elif b not in native and HOST_GTK.search(lib):
                report.problem("binaries", f"{b}: NEEDED {lib} (a host binary links GLib, GIO and GDK Pixbuf only)")
        if b in native:
            if "libgtk-4.so.1" in needed:
                gtk4 += 1
            else:
                report.problem("binaries", f"{b}: no NEEDED libgtk-4.so.1 (a native GTK 4 program)")
        resolved = subprocess.run(["ldd", str(path)], capture_output=True, text=True, env=env)
        for lib in re.findall(r"^\s*(\S+)\s+=>", resolved.stdout, re.MULTILINE):
            if BANNED_LIB.search(lib) or (b not in native and HOST_GTK.search(lib)):
                report.problem("binaries", f"{b}: ldd resolves {lib}")
    return f"{len(bins)} binaries in {build} ({len(bins) - len(native)} host, {gtk4} GTK 4)"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--build-dir", default="tests/build/native-release",
                    help="the 19 binaries to check (default tests/build/native-release)")
    ap.add_argument("--no-binaries", action="store_true", help="skip check 5")
    ap.add_argument("--allowed", action="store_true", help="also print every covered line, by class")
    a = ap.parse_args()
    report = Report()
    rules = load_rules(report)
    nfiles = check_text(report, rules)
    check_cargo(report)
    check_vendor(report)
    binaries, unchecked = "binaries skipped", None
    if not a.no_binaries:
        build = Path(a.build_dir)
        try:
            binaries = check_binaries(report, build if build.is_absolute() else ROOT / build)
        except Unchecked as e:
            binaries, unchecked = "binaries NOT checked", e
    if a.allowed:
        for klass in sorted(report.allowed):
            print(f"== allowed: {klass} ({len(report.allowed[klass])})")
            for hit in report.allowed[klass]:
                print(f"  {hit}")
    for p in report.problems:
        print(p)
    counts = ", ".join(f"{k} {len(v)}" for k, v in sorted(report.allowed.items()))
    state = f"{len(report.problems)} problem(s)" if report.problems else "OK"
    print(f"check-no-webkit: {state}; {nfiles} files, {len(rules)} allow rows (covered lines: {counts}); {binaries}")
    if unchecked:
        print(f"check-no-webkit: {unchecked}; check 5 did not run (--no-binaries skips it)", file=sys.stderr)
        return 2
    return 1 if report.problems else 0


if __name__ == "__main__":
    sys.exit(main())
