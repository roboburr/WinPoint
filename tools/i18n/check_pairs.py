#!/usr/bin/env python3
"""Check the inline {nl, en} tables of the GNOME extension and the installer (notes/claude/contracts.md C3).

    python3 -B tools/i18n/check_pairs.py [--warnings] [--json]

What it checks (exit 1 on any error):
* extension/*.js: every `// i18n: pairs` table `const TEXT = {key: {nl: '…', en: '…'}, …}` has both languages,
  non-empty, with the same {placeholders}; every key used through _t/t/text/tr('key') exists; an object literal
  anywhere with only one of nl:/en: is an error; no literal text is left in the visible call sites (failure(),
  fail(), new Error(), Main.notify(), accessible_name:, _button(), ctrlAltTabManager.addGroup()).
* scripts/*.py: every dict literal whose values are {"nl": …, "en": …} dicts (read with ast, nothing is run); the
  same rules; t/_t/install.t("key") calls name existing keys; print/SystemExit/ValueError take no literal text.
* scripts/*.sh, tools/h-remnants.sh: every `pair 'NL' 'EN'` (or `section 'NL' 'EN'`) has two non-empty texts with the same printf
  directives; no other echo/printf of a literal sentence.
* host/src/desktop-entries.json (C6, owned by GE1b; read only): version 1, nl/en for the comment and the 10
  entries that scripts/install.py writes.
* the glossary rule (ge1c-review m2): when an nl text has a phrase of tools/i18n/glossary.tsv, its en text must
  have the Windows 11 en-GB phrase (check.Glossary, the same rule as check_native.py; an error since the round-3
  lock-in, R3-UI; exceptions on purpose in GLOSSARY_OK).
Warnings: a table key that nothing uses; an nl text equal to its en text (allowed, listed for review); US spellings
in an en text (D-G15: en-GB; "minimized", "color").
"""
from __future__ import annotations
import ast
import json
from pathlib import Path
import re
import string
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import check  # noqa: E402  (tools/i18n/check.py: Glossary, shared with check_native.py)

GLOSSARY = check.Glossary(Path(__file__).resolve().parent/"glossary.tsv")
# (file, table.key) → nl glossary phrases whose en rendering differs on purpose.
GLOSSARY_OK = {
    # Printed only in nl (the locale guess gave nl): the Dutch UI path is the one the user will see.
    ("scripts/install.py", "T.langZone"): {"Instellingen", "Tijd en taal", "Taal en regio"},
    ("scripts/install.py", "T.langFormats"): {"Instellingen", "Tijd en taal", "Taal en regio"},
    # "achtergrond" is the window's frosted backdrop here (Mica/Acrylic), not the desktop Background.
    ("extension/frostedBridge.js", "TEXT.identityMismatch"): {"Achtergrond"},
}
# D-G15: en-GB. US spellings that Windows 11 en-GB does not use (stems; matched case-insensitively at word starts).
US_SPELLING = re.compile(r"\b(?:(?:minimi|maximi|organi|customi|personali|recogni|initiali|synchroni|optimi|authori|"
                         r"priori|summari|categori|finali)z(?:e|ed|es|ing|ation)|colors?|colored|centers?|centered|"
                         r"canceled|canceling|behaviors?|favorites?|gray)\b", re.IGNORECASE)
PLACEHOLDER = re.compile(r"\{(\w+)\}")
PRINTF = re.compile(r"%[-+ #0]*\d*(?:\.\d+)?[sdifxXc%]")
# Internal, never-shown tokens (file_portal's reason inside its own "invalid record" message; extension internals
# that are caught and never leave the process).
ALLOW_LITERAL = {
    ("scripts/file_portal.py", "marker"), ("scripts/file_portal.py", "selector path"),
    ("scripts/file_portal.py", "files"), ("scripts/file_portal.py", "path/mode"),
    ("extension/panelBridge.js", "Client protocol geometry is not ready"),
}
# Texts that are the same in both languages on purpose.
SAME_OK = {"Start", "Alles", "Win11 Start", "Win11 Settings", "Win11 Shell"}


class Report:
    def __init__(self):
        self.errors: list[str] = []
        self.warnings: list[str] = []
        self.stats: dict[str, int] = {}

    def error(self, where: str, text: str):
        self.errors.append(f"{where}: {text}")

    def warn(self, where: str, text: str):
        self.warnings.append(f"{where}: {text}")

    def count(self, key: str, n: int = 1):
        self.stats[key] = self.stats.get(key, 0) + n


def check_pair(report: Report, where: str, key: str, pair: dict, placeholders=PLACEHOLDER):
    report.count("pairs")
    keys = set(pair)
    if keys != {"nl", "en"}:
        report.error(where, f"{key}: languages {sorted(keys)}, expected ['en', 'nl']")
        return
    nl, en = pair["nl"], pair["en"]
    if not isinstance(nl, str) or not isinstance(en, str) or not nl.strip() or not en.strip():
        report.error(where, f"{key}: an empty or non-text value")
        return
    a, b = sorted(placeholders.findall(nl)), sorted(placeholders.findall(en))
    if a != b:
        report.error(where, f"{key}: placeholders differ: nl {a} / en {b}")
    if nl == en and nl not in SAME_OK:
        report.warn(where, f"{key}: nl == en ({nl!r})")
    report.count("glossary-checked")
    for gnl, gen in GLOSSARY.check(nl, en, GLOSSARY_OK.get((where, key), set())):
        report.error(where, f"glossary: {key}: nl “{gnl}” → en should contain “{gen}”")
    for word in sorted({m.group(0) for m in US_SPELLING.finditer(en)}):
        report.warn(where, f"en-GB: {key}: US spelling “{word}”")


# ---------------------------------------------------------------------------------------------------------------
# JavaScript: a small literal reader, enough for the constrained tables (identifiers, strings, { } : , comments)
# ---------------------------------------------------------------------------------------------------------------

def js_string(src: str, i: int) -> tuple[str, int]:
    quote = src[i]
    out, i = [], i + 1
    escapes = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "b": "\b", "f": "\f", "v": "\v"}
    while i < len(src):
        c = src[i]
        if c == "\\":
            nxt = src[i + 1]
            if nxt == "u":
                if src[i + 2] == "{":
                    end = src.index("}", i + 3)
                    out.append(chr(int(src[i + 3:end], 16)))
                    i = end + 1
                else:
                    out.append(chr(int(src[i + 2:i + 6], 16)))
                    i += 6
                continue
            if nxt == "x":
                out.append(chr(int(src[i + 2:i + 4], 16)))
                i += 4
                continue
            out.append(escapes.get(nxt, nxt))
            i += 2
            continue
        if c == quote:
            return "".join(out), i + 1
        if quote == "`" and c == "$" and src[i + 1:i + 2] == "{":
            raise ValueError("template substitution inside a pairs table")
        out.append(c)
        i += 1
    raise ValueError("unterminated string")


def js_skip(src: str, i: int) -> int:
    while i < len(src):
        if src[i].isspace():
            i += 1
        elif src.startswith("//", i):
            i = src.find("\n", i)
            i = len(src) if i < 0 else i
        elif src.startswith("/*", i):
            i = src.index("*/", i) + 2
        else:
            break
    return i


def js_value(src: str, i: int):
    """An object literal of identifiers/strings mapping to strings or nested objects; returns (value, end)."""
    i = js_skip(src, i)
    if src[i] in "'\"`":
        return js_string(src, i)
    if src[i] != "{":
        raise ValueError(f"unexpected {src[i:i + 20]!r}")
    obj, i = {}, i + 1
    while True:
        i = js_skip(src, i)
        if src[i] == "}":
            return obj, i + 1
        if src[i] in "'\"":
            key, i = js_string(src, i)
        else:
            m = re.compile(r"[A-Za-z_$][\w$]*").match(src, i)
            if not m:
                raise ValueError(f"bad key at {src[i:i + 20]!r}")
            key, i = m.group(0), m.end()
        i = js_skip(src, i)
        if src[i] != ":":
            raise ValueError(f"expected ':' after {key}")
        value, i = js_value(src, i + 1)
        if key in obj:
            raise ValueError(f"duplicate key {key}")
        obj[key] = value
        i = js_skip(src, i)
        if src[i] == ",":
            i += 1


def js_strip(src: str) -> str:
    """The source with comments blanked and string contents kept (a rough lexer; regex literals are rare here)."""
    out, i = [], 0
    while i < len(src):
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = len(src) if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif src.startswith("/*", i):
            j = src.index("*/", i) + 2
            out.append(re.sub(r"[^\n]", " ", src[i:j]))
            i = j
        elif src[i] in "'\"`":
            try:
                _, j = js_string(src, i)
            except (ValueError, IndexError):
                j = i + 1
            out.append(src[i:j])
            i = j
        else:
            out.append(src[i])
            i += 1
    return "".join(out)


VISIBLE_JS = [
    (re.compile(r"\bfailure\(\s*'[a-z_]+'\s*,\s*['\"`]"), "failure() with literal text"),
    (re.compile(r"\bfail\(\s*'[a-z_]+'\s*,\s*['\"`]"), "fail() with literal text"),
    (re.compile(r"\bnew Error\(\s*(['\"`])((?:(?!\1).)*)\1"), "new Error() with literal text"),
    (re.compile(r"\bMain\.notify\(\s*['\"`]"), "Main.notify() with literal text"),
    (re.compile(r"\baccessible_name:\s*['\"`]"), "accessible_name with literal text"),
    (re.compile(r"\b_button\(\s*['\"`]"), "_button() with literal text"),
    (re.compile(r"\baddGroup\([^,()]+,\s*['\"`]"), "ctrlAltTabManager.addGroup() with literal text"),
]
USE_JS = re.compile(r"(?:\b_t|\bt|\btext|\btr)\(\s*'(\w+)'")


def check_js(report: Report, path: Path):
    rel = path.relative_to(ROOT).as_posix()
    src = path.read_text(encoding="utf-8")
    plain = js_strip(src)
    tables = {}
    for m in re.finditer(r"//\s*i18n: pairs[^\n]*\n(?:\s*//[^\n]*\n)*\s*const\s+(\w+)\s*=\s*", src):
        name = m.group(1)
        try:
            table, end = js_value(src, m.end())
        except (ValueError, IndexError) as error:
            report.error(rel, f"table {name}: cannot read it ({error})")
            continue
        report.count("tables")
        if not isinstance(table, dict):
            report.error(rel, f"table {name} is not an object")
            continue
        for key, pair in table.items():
            if not isinstance(pair, dict):
                report.error(rel, f"{name}.{key}: not an {{nl, en}} pair")
                continue
            check_pair(report, rel, f"{name}.{key}", pair)
        tables[name] = (table, m.start(), end)
    keys = {key for table, _, _ in tables.values() for key in table}
    # Object literals with only one language, anywhere in the file (a leftover inline pair).
    for m in re.finditer(r"[{,]\s*(nl|en)\s*:", plain):
        start = plain.rfind("{", 0, m.end())
        try:
            obj, _ = js_value(src, start)
        except (ValueError, IndexError):
            continue
        if isinstance(obj, dict) and ("nl" in obj) != ("en" in obj):
            line = src.count("\n", 0, m.start()) + 1
            report.error(f"{rel}:{line}", "an object literal with only one of nl:/en:")
    outside = plain
    for _table, start, end in sorted(tables.values(), key=lambda v: -v[1]):
        outside = outside[:start] + " " * (end - start) + outside[end:]
    for pattern, what in VISIBLE_JS:
        for m in pattern.finditer(outside):
            if what.startswith("new Error") and (rel, m.group(2)) in ALLOW_LITERAL:
                continue
            line = src.count("\n", 0, m.start()) + 1
            report.error(f"{rel}:{line}", what)
    used = set(USE_JS.findall(outside))
    for key in sorted(used - keys):
        report.error(rel, f"key {key!r} is used but not in a pairs table")
    literals = set(re.findall(r"'(\w+)'", outside))
    for key in sorted(keys - used - literals):
        report.warn(rel, f"key {key!r} is never used")
    report.count("js-files")


# ---------------------------------------------------------------------------------------------------------------
# Python: ast only
# ---------------------------------------------------------------------------------------------------------------

def py_placeholders(text: str) -> list[str]:
    return sorted(name for _, name, _, _ in string.Formatter().parse(text) if name is not None)


class PyPlaceholders:
    @staticmethod
    def findall(text: str) -> list[str]:
        return py_placeholders(text)


def literal_dict(node):
    try:
        return ast.literal_eval(node)
    except (ValueError, SyntaxError, TypeError):
        return None


def check_py(report: Report, path: Path, tables_by_module: dict):
    rel = path.relative_to(ROOT).as_posix()
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=rel)
    tables: dict[str, dict] = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.Dict):
            value = literal_dict(node)
            if not isinstance(value, dict) or not value:
                continue
            if all(isinstance(v, dict) and {"nl", "en"} & set(v) for v in value.values()):
                name = "?"
                for parent in ast.walk(tree):
                    if isinstance(parent, ast.Assign) and parent.value is node:
                        name = ",".join(t.id for t in parent.targets if isinstance(t, ast.Name))
                report.count("tables")
                for key, pair in value.items():
                    check_pair(report, rel, f"{name}.{key}", pair, PyPlaceholders)
                tables[name] = value
            elif ("nl" in value) != ("en" in value) and all(isinstance(v, str) for v in value.values()):
                report.error(f"{rel}:{node.lineno}", "a dict with only one of 'nl'/'en'")
    tables_by_module[path.stem] = tables
    report.count("py-files")
    return tree


def check_py_uses(report: Report, path: Path, tree, tables_by_module: dict):
    rel = path.relative_to(ROOT).as_posix()
    own = {k for table in tables_by_module.get(path.stem, {}).values() for k in table}
    install = {k for table in tables_by_module.get("install", {}).values() for k in table}
    used = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call) or not node.args:
            continue
        func = node.func
        first = node.args[0]
        name = func.id if isinstance(func, ast.Name) else func.attr if isinstance(func, ast.Attribute) else None
        if name in ("t", "_t") and isinstance(first, ast.Constant) and isinstance(first.value, str):
            module = func.value.id if isinstance(func, ast.Attribute) and isinstance(func.value, ast.Name) else None
            keys = install if module == "install" else own
            used.add((module or path.stem, first.value))
            if first.value not in keys:
                report.error(f"{rel}:{node.lineno}", f"unknown text key {first.value!r}")
            if name == "t" and isinstance(func, ast.Name) and len(node.args) > 1 and isinstance(node.args[1], ast.Constant):
                pass
        if name in ("print", "SystemExit", "ValueError") and isinstance(func, ast.Name):
            if isinstance(first, (ast.Constant, ast.JoinedStr)):
                text = first.value if isinstance(first, ast.Constant) else "".join(
                    v.value for v in first.values if isinstance(v, ast.Constant))
                if isinstance(text, str) and re.search(r"[A-Za-z]{2}", text) and (rel, text) not in ALLOW_LITERAL:
                    report.error(f"{rel}:{node.lineno}", f"{name}() with literal text {text[:40]!r}")
    return used


# ---------------------------------------------------------------------------------------------------------------
# Shell
# ---------------------------------------------------------------------------------------------------------------

SH_PAIR = re.compile(r"""\b(?:pair|section)\s+'((?:[^'\\]|'\\'')*)'\s*(?:\\\n\s*)?'((?:[^'\\]|'\\'')*)'""")
SH_LITERAL = re.compile(r"""^\s*(?:echo|printf)\s+(?:--\s+)?(['"])(?=[^'"]*[A-Za-z]{3,}\s+[A-Za-z]{3,})""", re.M)


def check_sh(report: Report, path: Path):
    rel = path.relative_to(ROOT).as_posix()
    src = path.read_text(encoding="utf-8")
    for m in SH_PAIR.finditer(src):
        nl, en = m.group(1), m.group(2)
        check_pair(report, rel, f"line {src.count(chr(10), 0, m.start()) + 1}", {"nl": nl, "en": en},
                   type("P", (), {"findall": staticmethod(lambda s: PRINTF.findall(s))}))
    for m in SH_LITERAL.finditer(src):
        line = src.count("\n", 0, m.start()) + 1
        report.error(f"{rel}:{line}", "echo/printf of a literal sentence (use pair 'NL' 'EN')")
    report.count("sh-files")


def check_entries(report: Report):
    path = ROOT/"host/src/desktop-entries.json"
    rel = path.relative_to(ROOT).as_posix()
    try:
        table = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        report.error(rel, f"cannot read it ({error})")
        return
    if table.get("version") != 1:
        report.error(rel, "version is not 1")
    src = (ROOT/"scripts/install.py").read_text(encoding="utf-8")
    names = re.findall(r'"(\w+)"', re.search(r"ENTRIES = \((.*?)\)", src).group(1))
    check_pair(report, rel, "comment", table.get("comment") or {})
    entries = table.get("entries") or {}
    if sorted(entries) != sorted(names):
        report.error(rel, f"entries {sorted(entries)} differ from install.py ENTRIES {sorted(names)}")
    for name in names:
        if name in entries:
            check_pair(report, rel, f"entries.{name}", entries[name])
    report.count("json-files")


def main(argv: list[str]) -> int:
    report = Report()
    for path in sorted((ROOT/"extension").glob("*.js")):
        check_js(report, path)
    tables_by_module: dict = {}
    trees = [(path, check_py(report, path, tables_by_module)) for path in sorted((ROOT/"scripts").glob("*.py"))]
    used = set()
    for path, tree in trees:
        used |= check_py_uses(report, path, tree, tables_by_module)
    for module, tables in tables_by_module.items():
        for table in tables.values():
            for key in table:
                if (module, key) not in used:
                    report.warn(f"scripts/{module}.py", f"key {key!r} is never used through t()")
    for path in sorted((ROOT/"scripts").glob("*.sh")) + [ROOT/"tools/h-remnants.sh"]:
        if path.is_file():
            check_sh(report, path)
    check_entries(report)
    if "--json" in argv:
        print(json.dumps({"errors": report.errors, "warnings": report.warnings, "stats": report.stats}, indent=1,
                         ensure_ascii=False))
    else:
        for line in report.errors:
            print("ERROR " + line)
        if "--warnings" in argv:
            for line in report.warnings:
                print("warn  " + line)
        stats = ", ".join(f"{k} {v}" for k, v in sorted(report.stats.items()))
        print(f"check_pairs: {len(report.errors)} errors, {len(report.warnings)} warnings ({stats})")
    return 1 if report.errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
