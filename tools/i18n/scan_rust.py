#!/usr/bin/env python3
"""Rust user-visible-literal scanner for host/src (GE0b, 2026-11-05).

    python3 -B tools/i18n/scan_rust.py [--root G] [--out-all F] [--out-frozen F] [--summary]

Lists every string literal in ``host/src/**/*.rs`` outside test code, plus every string value of the embedded text
tables (``include_str!`` JSON: the four ``_STR`` tables, the Wi-Fi pairs, ``_FRIENDLY``/``_GSD_NL``/``_GVFS_NL``,
``apps/catalog-data.json``, ``files/types.json``, ``files/launcher-names.json``), and classifies each one:

* ``user-visible-dutch``    Dutch text a user can see (ApiError messages, labels, names, Err(String) texts ...);
* ``user-visible-english``  English text a user can see (window titles, English-only messages);
* ``bilingual-pair``        one half of an nl/en pair: a helper with ``nl``/``en`` parameters (``t``, ``err``,
                            ``blocked``), an ``if lang == "en" {..} else {..}`` or ``match lang`` branch, a
                            ``{"nl":..,"en":..}`` object, an adjacent nl/en tuple, or a JSON table half;
* ``log``                   eprintln!/println!/panic!/unreachable!/assert*/.expect texts and log helpers;
* ``technical``             paths, ids, method and D-Bus names, keys, codes, match patterns and comparisons,
                            command arguments, CSS markers, format-only strings, language-neutral names.

Test code is excluded: files reached through ``#[cfg(test)] mod x;`` (and their children), and every item that
carries ``#[cfg(test)]`` or ``#[test]`` (modules, functions, fields, statements, blocks).

Outputs (both deterministic: no timestamps, sorted):

* ``tests/i18n/rust-literals.tsv``  every literal: file, line, col, id, source, class, lang, side, context, key,
  reason, text (escapes ``\\t``, ``\\n``, ``\\r``, ``\\\\``);
* ``tests/i18n/rust-nl-frozen.tsv`` every Dutch user-visible text (``user-visible-dutch``, the nl half of every
  ``bilingual-pair``, and the Dutch file markers that must stay byte-identical, such as the gtk.css caption
  markers), keyed by file + ``sha256(text)[:12]``, with an occurrence count; never by line number.

Summary counts (``--summary``) go to stdout as JSON. The classification is heuristic: review the reason column.

Language detection uses ``tests/i18n/dutch-words.txt`` (H's committed list) plus the small EXTRA_NL set below, and
``/usr/share/dict/{american,british}-english`` (falls back to a built-in list when missing). The id column is
``<file>#<hash>#<n>``, n = occurrence of that text in that file in source order.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_ROOT = HERE.parent.parent

# ---------------------------------------------------------------------------------------------------- words

# Dutch words that occur in G's Rust literals but are not in H's dutch-words.txt (that list was generated from H's
# catalogues). Reviewed by hand on 2026-11-05; add only unambiguous Dutch words (no English homographs).
EXTRA_NL = {
    "aangemaakt", "afgebroken", "afgesloten", "beëindigd", "beëindigen", "bestandskiezer", "bestandsnaam",
    "bezig", "dienst", "diensten", "doelmap", "geannuleerd", "gekoppeld", "geopend", "gestart", "gesloten",
    "gevonden", "geweigerd", "gewijzigd", "gezocht", "kiezer", "koppelingen", "lukte", "mislukt", "ongeldig",
    "ongeldige", "opdracht", "opdrachtregel", "proces", "processen", "reageert", "selectie", "starten",
    "taakbeheer", "venster", "vensters", "verkenner", "verplaatsen", "wachtrij", "zoekopdracht", "bureaublad",
    "schijf", "schijven", "gebruiker", "melding", "meldingen", "prullenbak", "kopie", "afbeelding",
    "systeemschijf", "deze", "geen", "niet", "wordt", "worden", "kan", "kon", "het", "een", "van", "naar",
    "uit", "bij", "voor", "nog", "ook", "maar", "dan", "wel", "zijn", "heeft", "hebben", "moet", "mag", "niets",
    "alleen", "gelukt", "bestaat", "meer", "map", "mappen", "bestand", "bestanden", "openen", "opslaan",
    "en", "je", "te", "om", "er", "nu", "uw", "jouw", "wat", "waar", "hoe", "welke", "dubbele", "contacten",
}
# Language-neutral names: counted in neither language.
PROPER = {"gnome", "bluetooth", "wi", "fi", "usb", "hdmi", "snap", "snaps", "flatpak", "appimage", "nvidia", "amd",
          "intel", "linux", "ubuntu", "win11", "windows", "ok", "pc", "cpu", "gpu", "ram", "ssd", "hdd", "nvme",
          "efi", "uefi", "ntfs", "fat", "exfat", "btrfs", "ext", "xfs", "pipewire", "wayland", "hyprland", "zip",
          "tar", "dbus", "systemd", "webkit", "gtk", "gio", "udisks", "packagekit", "snapd", "x11", "shell", "id",
          "mb", "gb", "tb", "kb", "ghz", "mhz", "hz", "url", "uri", "json", "http", "api", "ip", "dns", "vpn",
          "lan", "wlan", "ethernet", "mpris", "sni"}
# Dutch morphology for words that neither list knows (compounds such as "instellingendienst", "integratie").
NL_SHAPE = re.compile(r"(ij|aa|uu|oe(?!s$)|sch|heid|elijk|ingen|atie|tie$|iek$|isch|baar|loos|^ge[a-z]{3,}[dt]$|"
                      r"^ver[a-z]{3,}|^ont[a-z]{3,}|[^aeiou]en$|ele$|eer$|eren$)")
# Words that the Dutch list contains but that are ordinary English in these sources (never count them as Dutch).
NOT_NL = {"in", "is", "over", "die", "had", "door", "want", "bad", "of", "a", "as", "an", "or",
          "me", "we", "wild", "hand", "land", "test", "type", "model", "status", "data", "index", "link"}
# Words that tip a text to English even without the system dictionaries.
EN_COMMON = {
    "the", "a", "an", "is", "are", "was", "not", "no", "of", "to", "in", "on", "for", "with", "this", "that",
    "file", "files", "folder", "folders", "could", "can", "cannot", "can't", "be", "been", "has", "have", "and",
    "or", "from", "by", "at", "it", "its", "you", "your", "please", "try", "again", "failed", "error", "unknown",
    "invalid", "settings", "explorer", "manager", "task", "disks", "default", "apps", "app", "selection",
    "installer", "start", "taskbar", "desktop", "everything", "missing", "unsupported", "denied", "timeout",
    "service", "window", "open", "close", "copy", "move", "delete", "rename", "name", "new", "document", "text",
    "system", "drive", "this", "pc", "recycle", "bin", "links", "minute", "minutes", "hour", "hours", "items",
}
WORD = re.compile(r"[A-Za-zÀ-ÖØ-öø-ÿ]+(?:['’][a-zë]+)?")
PH = re.compile(r"\{[^{}]*\}")


class Words:
    def __init__(self, root: Path):
        self.sources: list[str] = []
        self.nl = set(EXTRA_NL)
        f = root / "tests/i18n/dutch-words.txt"
        if f.is_file():
            data = f.read_bytes()
            self.sources.append(f"{f.relative_to(root)} sha256:{hashlib.sha256(data).hexdigest()[:12]}")
            self.nl |= {ln.strip() for ln in data.decode("utf-8").split("\n") if ln.strip() and not ln.startswith("#")}
        else:
            self.sources.append("tests/i18n/dutch-words.txt missing (EXTRA_NL only)")
        self.nl -= NOT_NL
        self.en = set(EN_COMMON)
        for name in ("american-english", "british-english"):
            p = Path("/usr/share/dict") / name
            if p.is_file():
                data = p.read_bytes()
                self.sources.append(f"{p} sha256:{hashlib.sha256(data).hexdigest()[:12]}")
                self.en |= {w.strip().lower() for w in data.decode("utf-8", "replace").split("\n") if w.strip()}
        self.en -= self.nl
        self.long_nl = {w for w in self.nl if len(w) >= 6}

    def split(self, text: str) -> tuple[list[str], list[str], list[str]]:
        raw = [w.replace("’", "'") for w in WORD.findall(re.sub(r"\w*\d\w*", " ", PH.sub(" ", text)))]
        pairs = [(w, w.lower()) for w in raw if w.lower() not in PROPER]
        words = [lw for _, lw in pairs]
        nl, en = [], []
        for orig, w in pairs:
            base = w[:-2] if w.endswith("'s") else w
            if w in self.nl or base in self.nl:
                nl.append(w)
            elif w in self.en or base in self.en:
                en.append(w)
            elif (len(base) >= 5 and not re.search(r"[a-z][A-Z]", orig)
                  and (NL_SHAPE.search(base) or self.compound(base))):
                nl.append(w)  # unknown word with Dutch shape; camelCase/CamelCase tokens never count
        return words, nl, en

    def compound(self, word: str) -> bool:
        """An unknown word that contains a Dutch word of 6+ letters (lettertypebestand, instellingendienst)."""
        n = len(word)
        return any(word[i:j] in self.long_nl for i in range(n) for j in range(i + 6, n + 1) if j - i < n)

    def lang(self, text: str) -> str:
        words, nl, en = self.split(text)
        if not words:
            return "none"
        if nl and len(nl) * 3 >= len(en):
            return "nl"
        if en and not nl:
            return "en"
        if en and nl:
            return "en" if len(en) > 3 * len(nl) else "mixed"
        return "none"


# ---------------------------------------------------------------------------------------------------- lexer

@dataclass
class Tok:
    kind: str  # ident, str, char, num, punct, lifetime
    text: str  # identifier/punct text, or the decoded value of a string literal
    line: int
    col: int
    flavor: str = ""  # str flavours: str, raw, bytes, rawbytes, c


PUNCT3 = ("..=", "...", "<<=", ">>=")
PUNCT2 = ("::", "=>", "==", "!=", "->", "<=", ">=", "&&", "||", "..", "+=", "-=", "*=", "/=", "%=", "^=",
          "&=", "|=", "<<", ">>")
RAW_START = re.compile(r'(b|c)?r(#*)"')
STR_START = re.compile(r'(b|c)?"')
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
NUMBER = re.compile(r"[0-9][0-9A-Za-z_]*(?:\.[0-9][0-9A-Za-z_]*)?")
ESC = {"n": "\n", "r": "\r", "t": "\t", "\\": "\\", "0": "\0", "'": "'", '"': '"'}


def unescape(body: str) -> str:
    out: list[str] = []
    i = 0
    while i < len(body):
        c = body[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        d = body[i + 1] if i + 1 < len(body) else ""
        if d in "\n\r":
            i += 2
            while i < len(body) and body[i] in " \t\n\r":
                i += 1
        elif d in ESC:
            out.append(ESC[d])
            i += 2
        elif d == "x":
            out.append(chr(int(body[i + 2:i + 4], 16)))
            i += 4
        elif d == "u":
            j = body.index("}", i)
            out.append(chr(int(body[i + 3:j].replace("_", ""), 16)))
            i = j + 1
        else:
            out.append("\\" + d)
            i += 2
    return "".join(out)


def lex(src: str) -> list[Tok]:
    toks: list[Tok] = []
    i, n, line, ls = 0, len(src), 1, 0

    def newlines(a: int, b: int) -> None:
        nonlocal line, ls
        k = src.count("\n", a, b)
        if k:
            line += k
            ls = src.rfind("\n", a, b) + 1

    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            ls = i + 1
            i += 1
            continue
        if c in " \t\r":
            i += 1
            continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            newlines(i, j)
            i = j
            continue
        start_line, start_col = line, i - ls + 1
        m = RAW_START.match(src, i)
        if m:
            hashes = m.group(2)
            body_start = m.end()
            end = src.index('"' + hashes, body_start)
            flavor = "rawbytes" if m.group(1) == "b" else "raw"
            toks.append(Tok("str", src[body_start:end], start_line, start_col, flavor))
            newlines(i, end)
            i = end + 1 + len(hashes)
            continue
        m = STR_START.match(src, i)
        if m:
            j = m.end()
            while src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            flavor = {"b": "bytes", "c": "c"}.get(m.group(1) or "", "str")
            toks.append(Tok("str", unescape(src[m.end():j]), start_line, start_col, flavor))
            newlines(i, j)
            i = j + 1
            continue
        if c == "'" or (c == "b" and src.startswith("b'", i)):
            q = i + (1 if c == "'" else 2)
            if src[q] == "\\":
                if src[q + 1] == "u":
                    j = src.index("}", q) + 1
                elif src[q + 1] == "x":
                    j = q + 4
                else:
                    j = q + 2
                toks.append(Tok("char", src[q:j], start_line, start_col))
                i = j + 1
                continue
            if q + 1 < n and src[q + 1] == "'":
                toks.append(Tok("char", src[q], start_line, start_col))
                i = q + 2
                continue
            if c == "'":
                m = IDENT.match(src, i + 1)
                toks.append(Tok("lifetime", "'" + (m.group(0) if m else ""), start_line, start_col))
                i = m.end() if m else i + 1
                continue
        if src.startswith("r#", i) and IDENT.match(src, i + 2):
            m = IDENT.match(src, i + 2)
            toks.append(Tok("ident", m.group(0), start_line, start_col))
            i = m.end()
            continue
        m = IDENT.match(src, i)
        if m:
            toks.append(Tok("ident", m.group(0), start_line, start_col))
            i = m.end()
            continue
        m = NUMBER.match(src, i)
        if m:
            text = m.group(0)
            if "." in text and src.startswith("..", i + text.index(".")):
                text = text[: text.index(".")]
            toks.append(Tok("num", text, start_line, start_col))
            i += len(text)
            continue
        for p in PUNCT3 + PUNCT2:
            if src.startswith(p, i):
                toks.append(Tok("punct", p, start_line, start_col))
                i += len(p)
                break
        else:
            toks.append(Tok("punct", c, start_line, start_col))
            i += 1
    return toks


# ---------------------------------------------------------------------------------------------------- structure

KEYWORDS = {
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct",
    "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "yield",
}
LANG_IDENTS = {"lang", "language", "english", "is_en", "en", "nl", "locale", "dutch"}
LANG_CODES = {"en", "nl", "NL", "US", "GB", "en_US", "nl_NL", "en_GB", "C", "C.UTF-8", "en-US", "nl-NL"}
ARM_CONTINUES = {",", ".", "?", "else", "as", "+", "-", "*", "/", "%", "==", "!=", "&&", "||", "<", ">", "<=", ">=",
                 ")", "]", "}", ";", "|", "..", "=>"}
LOG_MACROS = {"eprintln!", "println!", "eprint!", "print!", "panic!", "unreachable!", "todo!", "unimplemented!",
              "assert!", "assert_eq!", "assert_ne!", "debug_assert!", "debug_assert_eq!", "debug_assert_ne!", "dbg!"}
LOG_CALLS = {".expect", ".unwrap_or_else_log", "trace"}
STRING_OPS = {
    ".starts_with", ".ends_with", ".contains", ".strip_prefix", ".strip_suffix", ".split", ".rsplit", ".splitn",
    ".rsplitn", ".split_once", ".rsplit_once", ".replace", ".replacen", ".find", ".rfind", ".trim_matches",
    ".trim_start_matches", ".trim_end_matches", ".matches", ".eq_ignore_ascii_case", ".split_terminator",
    ".get", ".remove", ".contains_key", ".entry", ".lookup_value", ".lookup", ".property", ".string", ".int",
    ".uint", ".boolean", ".set_string", ".set_int", ".set_uint", ".set_boolean", ".value", ".set_value",
    ".has_key", ".is_writable", ".pointer", ".pointer_mut", ".get_mut", ".starts_with_ignore_case",
    ".locale_string", ".locale_string_list", ".has_group", ".string_list", ".groups", ".keys", "local", "set_key",
    ".position", ".is_some_and", ".any", ".all", ".signal_subscribe", ".connect", ".connect_local",
}
TECH_CALLS = {
    "Command::new", ".arg", ".args", ".env", ".env_remove", "env::var", "env::var_os", "std::env::var",
    "std::env::var_os", "Path::new", "PathBuf::from", ".join", ".with_file_name", ".with_extension",
    "OsStr::new", "OsString::from", "include_str!", "include_bytes!", "env!", "concat!", "stringify!",
    "gio::File::for_path", "gio::File::for_uri", "glib::Variant::from", "call", "gnome::call", "gnome::call_api",
    ".call", ".call_sync", "DBusProxy::new_for_bus_sync", "gio::DBusProxy::new_for_bus_sync", ".emit",
    "Settings::new", "gio::Settings::new", ".lookup", "SettingsSchemaSource::lookup", ".return_dbus_error",
    "glib::Error::new", "gio::DBusNodeInfo::for_xml", ".evaluate_script", ".run_javascript", "send_file", "bytes",
    "super::send_file", "super::bytes", ".header", "Header::from_bytes", "tiny_http::Header::from_bytes",
}
USER_KEYS = {"message", "title", "label", "name", "text", "detail", "details", "description", "hint", "body",
             "summary", "subtitle", "reason", "statusText", "tooltip", "caption", "heading", "note", "comment",
             "generic", "displayName", "friendly", "appName", "error", "info", "warning", "confirm", "button",
             "action", "question", "explain", "explanation", "help", "placeholder", "display"}
DISPLAY_NAME = re.compile(r"(title|name|label|message|msg|text|caption|reason|detail|heading|summary|body|hint|"
                          r"desc|friendly|display|protected|refus|human|nice|pretty)", re.I)
TECH_PATTERNS = [
    (re.compile(r"^[^A-Za-zÀ-ÖØ-öø-ÿ]*$"), "no letters"),
    (re.compile(r"^\s*(\{[^{}]*\}\s*[^A-Za-z\s]*\s*)+$"), "format placeholders only"),
    (re.compile(r"^[a-z][a-z0-9+.-]*://|^(file|https?|mailto|data|trash|recent|computer|x-scheme-handler|builtin):"),
     "URI"),
    (re.compile(r"^(/|~/|\./|\.\./|\$)[^\s]*$"), "path"),
    (re.compile(r"^<"), "markup/XML"),
    (re.compile(r"^-{1,2}[A-Za-z0-9{][\w{}-]*(=\S*)?$"), "command-line flag"),
    (re.compile(r"^[A-Za-z0-9_+.{}-]+(/[A-Za-z0-9_.+*{}-]+)+/?$"), "path or MIME type"),
    (re.compile(r"^[A-Za-z_][A-Za-z0-9_-]*(\.[A-Za-z0-9_*-]+)+$"), "dotted id (method, D-Bus, file name)"),
    (re.compile(r"^[a-z_][\w-]*\.$"), "dotted prefix"),
    (re.compile(r"^[a-z][a-z0-9]*([_-][a-z0-9]+)+$"), "snake/kebab identifier"),
    (re.compile(r"^[a-z]+[A-Z][A-Za-z0-9]*$"), "camelCase identifier"),
    (re.compile(r"^[A-Z0-9][A-Z0-9_-]*$"), "constant, unit or env name"),
    (re.compile(r"^[A-Z][a-z0-9]+([A-Z][a-z0-9]*)+$"), "CamelCase identifier"),
    (re.compile(r"^[a-z0-9]*[0-9][a-z0-9]*$"), "lowercase token with digits"),
    (re.compile(r"^[a-z][\w-]*:[\w*.-]+$"), "prefixed token"),
    (re.compile(r"^[\w.-]+=\S*$"), "key=value"),
    (re.compile(r"^[()a-z{}]+$"), "GVariant signature or lowercase token"),
    (re.compile(r"^\.[a-z]"), "CSS selector or extension"),
    (re.compile(r"[{};]\s*$|^\s*[@#.][\w-]+\s*\{|=>|\bfunction\b|\bawait\b"), "CSS or code"),
    (re.compile(r"^[\w*?.\[\]-]*[*?][\w*?.\[\]-]*$"), "glob"),
    (re.compile(r"^%[a-zA-Z]"), "printf/strftime format"),
    (re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-"), "UUID"),
    (re.compile(r"^#[0-9A-Fa-f]{3,8}$"), "colour"),
    (re.compile(r"\r\n|HTTP/1\.[01]|^(private|public|no-cache|no-store|max-age)\b|charset=|default-src"),
     "HTTP protocol text"),
    (re.compile(r"^(>>>|<<<)|^/\*|\*/$|^# \S"), "file marker"),
    (re.compile(r"^X-[A-Za-z0-9-]+$"), "desktop-entry key"),
    (re.compile(r"^([a-z0-9]{1,6} ){5,}[a-z0-9]{1,6}$"), "token list"),
    (re.compile(r"^\s*(for|case|if|then|echo|set|export)\b.*(;|\$)"), "shell script"),
]
TECH_PHRASES = {"Desktop Entry", "Default Applications", "Added Associations", "Removed Associations",
                "D-BUS Service", "Desktop Action", "Win11 Shell", "Win11", "GNOME Shell", "X-GNOME-Autostart-enabled"}
HTTP_REASONS = {"Bad URL", "Bad Request", "Not Found", "Method Not Allowed", "Unauthorized", "Forbidden",
                "Payload Too Large", "Internal Server Error", "Service Unavailable", "Not Modified", "Conflict",
                "Gone", "Too Many Requests", "Request Timeout", "Unsupported Media Type"}
TRANSPARENT = {"format!", "Some", "Ok", "Err", "String::from", ".to_string", ".into", ".to_owned", "()",
               "Cow::Borrowed", "Value::String", "json!", "vec!", ".push", ".insert", "Box::new", "Arc::new"}
STRONG_DISPLAY = re.compile(r"^(title|label|name|message|msg|text|caption|heading|summary|body|hint|detail|"
                            r"description|desc|reason|protected|friendly|display_name|app_name|tooltip|status_text)$")


@dataclass
class Frame:
    kind: str  # call, macro, tuple, array, index, jobj, jarray, struct, block, then, else, match, fnbody, attr
    callee: str = ""
    argidx: int = 0
    in_json: bool = False
    key: str | None = None
    lang_side: str | None = None  # for then/else blocks of a language condition: "nl" / "en" / "?"
    fn_name: str = ""
    match_lang: bool = False
    in_pattern: bool = True
    arm_pattern: list = field(default_factory=list)
    arm_block: bool = False
    fid: int = 0


@dataclass
class Lit:
    file: str
    line: int
    col: int
    text: str
    flavor: str
    role: str = ""
    chain: list = field(default_factory=list)  # [(kind, callee, argidx, key)] innermost first
    key: str | None = None
    binding: str = ""
    fn: str = ""
    side: str | None = None
    side_reason: str = ""
    frame_id: int = 0
    frame_kind: str = ""
    cls: str = ""
    lang: str = ""
    reason: str = ""
    source: str = "rust"
    context: str = ""


def lang_condition(tokens: list[Tok]) -> str | None:
    """'en' when the then-branch is English, 'nl' when it is Dutch, '?' when language-related but unclear."""
    idents = {t.text for t in tokens if t.kind == "ident"}
    strs = [t.text for t in tokens if t.kind == "str"]
    texts = [t.text for t in tokens]
    if not (idents & LANG_IDENTS or "language" in idents or set(strs) & {"en", "nl"}):
        return None
    joined = " ".join(texts)
    if re.search(r'== en\b', joined) or ("en" in strs and "==" in texts):
        return "en"
    if "nl" in strs and "==" in texts:
        return "nl"
    if "en" in strs and "!=" in texts:
        return "nl"
    if not strs and idents & {"english", "is_en", "en"} and "!" not in texts:
        return "en"
    return "?" if idents & LANG_IDENTS else None


def is_cfg_test(attr: list[Tok]) -> bool:
    texts = [t.text for t in attr]
    if texts[:1] == ["test"] and len(texts) == 1:
        return True
    if texts[:1] != ["cfg"]:
        return False
    if "not" in texts:
        return False
    return "test" in texts and (texts[1:3] == ["(", "test"] or "all" in texts)


class FileScanner:
    def __init__(self, rel: str, toks: list[Tok], pair_fns: dict, msg_fns: dict, code_fns: dict):
        self.rel = rel
        self.toks = toks
        self.pair_fns = pair_fns
        self.msg_fns = msg_fns
        self.code_fns = code_fns
        self.lits: list[Lit] = []
        self.mods: list[tuple[str, str | None, bool]] = []  # (name, path attr, cfg_test)
        self.api_error_sites = 0
        self.skipped_tests = 0
        self.skipped_ranges: list[tuple[int, int]] = []
        self.next_fid = 1

    # -- helpers
    def callee_before(self, i: int) -> tuple[str, str]:
        """(kind, callee) for an opening bracket at index i."""
        t = self.toks
        p = t[i - 1] if i else None
        if p is None:
            return "tuple", ""
        if p.text == "!" and i >= 2 and t[i - 2].kind == "ident":
            return "macro", t[i - 2].text + "!"
        j = i - 1
        if p.text == ">":  # turbofish ::<..>(
            depth = 0
            while j > 0:
                if t[j].text == ">":
                    depth += 1
                elif t[j].text == ">>":
                    depth += 2
                elif t[j].text == "<":
                    depth -= 1
                    if depth <= 0:
                        break
                j -= 1
            if j >= 2 and t[j - 1].text == "::" and t[j - 2].kind == "ident":
                j -= 2
                p = t[j]
            else:
                return "tuple", ""
        if p.kind == "ident" and p.text not in KEYWORDS - {"self", "Self", "super", "crate"}:
            parts = [p.text]
            k = j
            while k >= 2 and t[k - 1].text == "::" and t[k - 2].kind == "ident":
                parts.insert(0, t[k - 2].text)
                k -= 2
            if k >= 1 and t[k - 1].text == ".":
                return "call", "." + "::".join(parts)
            if parts[-2:] == ["ApiError", "new"]:  # crate::files::ApiError::new, files::ApiError::new
                return "call", "ApiError::new"
            return "call", "::".join(parts)
        if p.text in (")", "]", "?"):
            return "call", "()"
        return "tuple", ""

    def skip_item(self, i: int) -> int:
        """Index after the item that starts at i (used after #[cfg(test)] / #[test])."""
        t = self.toks
        while i + 1 < len(t) and t[i].text == "#" and t[i + 1].text == "[":
            depth, i = 0, i + 1
            while i < len(t):
                if t[i].text == "[":
                    depth += 1
                elif t[i].text == "]":
                    depth -= 1
                    if depth == 0:
                        break
                i += 1
            i += 1
        keyword_item = i < len(t) and t[i].text in {"fn", "mod", "impl", "struct", "enum", "type", "use", "const",
                                                     "static", "let", "pub", "trait", "unsafe", "async", "extern"}
        depth = 0
        while i < len(t):
            x = t[i].text
            if x in "([{" and t[i].kind == "punct":
                depth += 1
            elif x in ")]}" and t[i].kind == "punct":
                depth -= 1
                if depth < 0:
                    return i
                if depth == 0 and x == "}":
                    nxt = t[i + 1].text if i + 1 < len(t) else ""
                    if nxt == "else":
                        i += 1
                        continue
                    if nxt in (";", ","):
                        return i + 2
                    return i + 1
            elif depth == 0 and (x == ";" or (x == "," and not keyword_item)):
                return i + 1
            i += 1
        return i

    def nearest_key(self, stack: list[Frame]) -> str | None:
        for f in reversed(stack):
            if f.kind in ("jobj", "struct") and f.key:
                return f.key
            if f.kind in ("fnbody",):
                break
        return None

    # -- main walk
    def run(self) -> None:
        t = self.toks
        stack: list[Frame] = []
        pend_if: list[tuple[int, int]] = []  # (depth, cond_start)
        pend_match: list[tuple[int, int]] = []
        pend_else: dict[int, str | None] = {}
        last_then: dict[int, str | None] = {}
        pend_fn: dict[int, str] = {}
        bindings: dict[int, str] = {}
        pend_binding: dict[int, int] = {}  # depth -> start index of pattern (let/const/static)
        i = 0
        while i < len(t):
            tok = t[i]
            depth = len(stack)
            x = tok.text
            # attributes
            if tok.kind == "punct" and x == "#" and i + 1 < len(t) and t[i + 1].text in ("[", "!"):
                j = i + 1 if t[i + 1].text == "[" else i + 2
                if j < len(t) and t[j].text == "[":
                    d, k = 0, j
                    while k < len(t):
                        if t[k].text == "[":
                            d += 1
                        elif t[k].text == "]":
                            d -= 1
                            if d == 0:
                                break
                        k += 1
                    attr = t[j + 1:k]
                    if t[i + 1].text == "[" and is_cfg_test(attr):
                        end = self.skip_item(k + 1)
                        # a cfg(test) `mod x;` declaration names a test file
                        body = t[k + 1:end]
                        path_attr = None
                        names = [b.text for b in body]
                        for q in range(len(body) - 2):
                            if body[q].text == "path" and body[q + 1].text == "=" and body[q + 2].kind == "str":
                                path_attr = body[q + 2].text
                        if "mod" in names:
                            m = names.index("mod")
                            if m + 2 < len(names) and names[m + 2] == ";":
                                self.mods.append((names[m + 1], path_attr, True))
                        self.skipped_tests += 1
                        self.skipped_ranges.append((i, end))
                        i = end
                        continue
                    for a in attr:
                        if a.kind == "str":
                            self.emit(a, stack, bindings, role="attribute", attr_name=attr[0].text if attr else "")
                    # a plain `#[path = "..."] mod x;` declaration
                    if attr and attr[0].text == "path" and len(attr) >= 3 and attr[2].kind == "str":
                        q = k + 1
                        while q < len(t) and t[q].text in ("pub", "(", "crate", "super", ")"):
                            q += 1
                        if q + 2 < len(t) and t[q].text == "mod" and t[q + 2].text == ";":
                            self.mods.append((t[q + 1].text, attr[2].text, False))
                            i = q + 3
                            continue
                    i = k + 1
                    continue
            if tok.kind == "ident":
                if x == "mod" and i + 2 < len(t) and t[i + 1].kind == "ident" and t[i + 2].text == ";":
                    self.mods.append((t[i + 1].text, None, False))
                elif x == "if":
                    if i and t[i - 1].text == "else":
                        pend_else.pop(depth, None)
                    if not (stack and stack[-1].kind == "match" and stack[-1].in_pattern):  # not a match guard
                        pend_if.append((depth, i + 1))
                elif x == "match":
                    pend_match.append((depth, i + 1))
                elif x == "else":
                    pend_else[depth] = last_then.get(depth)
                elif x == "fn" and i + 1 < len(t) and t[i + 1].kind == "ident":
                    pend_fn[depth] = t[i + 1].text
                elif x in ("let", "const", "static") and depth not in pend_binding:
                    pend_binding[depth] = i + 1
            if tok.kind == "str":
                self.emit(tok, stack, bindings, nxt=t[i + 1] if i + 1 < len(t) else None,
                          prv=t[i - 1] if i else None)
                i += 1
                continue
            if tok.kind != "punct":
                if stack and stack[-1].kind == "match" and stack[-1].in_pattern:
                    stack[-1].arm_pattern.append(tok)
                i += 1
                continue
            # punctuation
            top = stack[-1] if stack else None
            if x == "=" and depth in pend_binding:
                start = pend_binding.pop(depth)
                if t[start - 1].text in ("const", "static"):  # const NAME: TYPE = ...
                    names = [q.text for q in t[start:i] if q.kind == "ident" and q.text != "mut"][:1]
                else:
                    names = [q.text for q in t[start:i] if q.kind == "ident" and q.text not in KEYWORDS
                             and not q.text[:1].isupper()]
                bindings[depth] = ",".join(names)
            elif x == ";":
                pend_binding.pop(depth, None)
                bindings.pop(depth, None)
                pend_fn.pop(depth, None)
                last_then.pop(depth, None)
            if x in ("(", "[", "{"):
                fid = self.next_fid
                self.next_fid += 1
                in_json = bool(top and top.in_json and top.kind in ("jobj", "jarray", "macro"))
                if x == "(":
                    kind, callee = self.callee_before(i)
                    if kind == "call" and callee == "ApiError::new":
                        self.api_error_sites += 1
                        if i + 3 < len(t) and t[i + 2].text == "," and t[i + 3].kind == "str":
                            pass
                    fr = Frame(kind, callee, fid=fid,
                               in_json=(kind == "macro" and callee == "json!") or (in_json and kind == "tuple"))
                elif x == "[":
                    p = t[i - 1] if i else None
                    if p is not None and p.text == "!" and i >= 2 and t[i - 2].kind == "ident":
                        fr = Frame("macro", t[i - 2].text + "!", fid=fid)
                    elif in_json:
                        fr = Frame("jarray", fid=fid, in_json=True)
                    elif p is not None and (p.kind in ("ident", "str") and p.text not in KEYWORDS
                                            or p.text in (")", "]", "?")):
                        fr = Frame("index", fid=fid)
                    else:
                        fr = Frame("array", fid=fid)
                else:
                    p = t[i - 1] if i else None
                    if pend_if and pend_if[-1][0] == depth:
                        _, cs = pend_if.pop()
                        side = lang_condition(t[cs:i])
                        fr = Frame("then", lang_side=side, fid=fid)
                    elif pend_match and pend_match[-1][0] == depth:
                        _, ms = pend_match.pop()
                        scrut = t[ms:i]
                        is_lang = bool({q.text for q in scrut if q.kind == "ident"} & LANG_IDENTS)
                        fr = Frame("match", match_lang=is_lang, fid=fid)
                    elif depth in pend_else:
                        prev_side = pend_else.pop(depth)
                        side = {"en": "nl", "nl": "en"}.get(prev_side or "", prev_side)
                        fr = Frame("else", lang_side=side, fid=fid)
                    elif depth in pend_fn:
                        fr = Frame("fnbody", fn_name=pend_fn.pop(depth), fid=fid)
                    elif p is not None and p.text == "!" and i >= 2 and t[i - 2].kind == "ident":
                        fr = Frame("macro", t[i - 2].text + "!", fid=fid, in_json=t[i - 2].text == "json")
                    elif in_json:
                        fr = Frame("jobj", fid=fid, in_json=True)
                    elif (p is not None and p.kind == "ident" and (p.text[:1].isupper() or p.text == "Self")
                          and i >= 2 and t[i - 2].text not in ("struct", "enum", "union", "trait", "impl", "for",
                                                              "mod", "->", "dyn", "type", "where", ":")):
                        fr = Frame("struct", p.text, fid=fid)
                    else:
                        fr = Frame("block", fid=fid)
                    if top and top.kind == "match" and not top.in_pattern and p is not None and p.text == "=>":
                        fr.arm_block = True
                if top and top.kind == "match" and top.in_pattern:
                    top.arm_pattern.append(tok)
                stack.append(fr)
                i += 1
                continue
            if x in (")", "]", "}"):
                if not stack:
                    i += 1
                    continue
                fr = stack.pop()
                d = len(stack)
                for table in (bindings, pend_binding, pend_else, pend_fn, last_then):
                    for k in [k for k in table if k > d]:
                        del table[k]
                pend_if[:] = [p for p in pend_if if p[0] <= d]
                pend_match[:] = [p for p in pend_match if p[0] <= d]
                nxt = t[i + 1].text if i + 1 < len(t) else ""
                if fr.kind == "then":
                    last_then[d] = fr.lang_side
                elif fr.kind == "else":
                    last_then.pop(d, None)
                if fr.kind in ("then", "else", "match", "block", "fnbody") and nxt not in (
                        ";", ".", "?", "else", ",", ")", "]", "as"):
                    bindings.pop(d, None)  # a block-like expression ended its statement
                    pend_binding.pop(d, None)
                if fr.arm_block and stack and stack[-1].kind == "match":
                    if nxt != ",":
                        stack[-1].in_pattern = True
                        stack[-1].arm_pattern = []
                elif (fr.kind in ("then", "else", "match", "block") and stack and stack[-1].kind == "match"
                      and not stack[-1].in_pattern and nxt not in ARM_CONTINUES):
                    stack[-1].in_pattern = True  # `pat => if c {..} else {..}` without a trailing comma
                    stack[-1].arm_pattern = []
                if stack and stack[-1].kind == "match" and stack[-1].in_pattern and not fr.arm_block:
                    stack[-1].arm_pattern.append(tok)
                i += 1
                continue
            if top is not None:
                if x == ",":
                    top.argidx += 1
                    top.key = None
                    if top.kind == "match":
                        top.in_pattern = True
                        top.arm_pattern = []
                elif x == ":" and top.kind in ("jobj", "struct") and top.key is None:
                    p = t[i - 1]
                    if (top.kind == "jobj" and p.kind == "str") or (top.kind == "struct" and p.kind == "ident"):
                        top.key = p.text
                elif x == "=>" and top.kind == "match":
                    top.in_pattern = False
                elif top.kind == "match" and top.in_pattern:
                    top.arm_pattern.append(tok)
            i += 1

    def emit(self, tok: Tok, stack: list[Frame], bindings: dict, role: str = "", nxt: Tok | None = None,
             prv: Tok | None = None, attr_name: str = "") -> None:
        lit = Lit(self.rel, tok.line, tok.col, tok.text, tok.flavor)
        top = stack[-1] if stack else None
        if role:
            lit.role = role
            lit.chain = [("attr", "#[" + attr_name + "]", 0, None)]
        else:
            if top and top.kind == "jobj" and nxt is not None and nxt.text == ":" and top.key is None:
                lit.role = "json-key"
            elif (prv is not None and prv.text in ("==", "!=")) or (nxt is not None and nxt.text in ("==", "!=")):
                lit.role = "compare"
            elif top and top.kind == "match" and top.in_pattern:
                lit.role = "pattern"
                top.arm_pattern.append(tok)
            elif top and top.kind == "index":
                lit.role = "index"
            elif nxt is not None and nxt.text == "|" and prv is not None and prv.text in ("|", "(", ",", "=>"):
                lit.role = "pattern"
            for f in reversed(stack):
                # match patterns nested in tuples: ("en", 1) =>
                if f.kind == "match":
                    if f.in_pattern:
                        lit.role = "pattern"
                        if f is not top:
                            f.arm_pattern.append(tok)
                    break
                if f.kind in ("fnbody", "block", "then", "else"):
                    break
        chain = []
        for f in reversed(stack):
            if f.kind in ("block",):
                continue
            chain.append((f.kind, f.callee or f.fn_name, f.argidx, f.key))
            if f.kind == "fnbody":
                break
        if not lit.chain:
            lit.chain = chain
        lit.key = self.nearest_key(stack)
        for d in sorted(bindings, reverse=True):
            if d <= len(stack) and bindings[d]:
                lit.binding = bindings[d]
                break
        for f in reversed(stack):
            if f.kind == "fnbody":
                lit.fn = f.fn_name
                break
        lit.frame_id = top.fid if top else 0
        lit.frame_kind = top.kind if top else ""
        # pair evidence
        for f in reversed(stack):
            if f.kind in ("then", "else") and f.lang_side:
                lit.side = f.lang_side if f.lang_side != "?" else None
                lit.side_reason = f"{'if' if f.kind == 'then' else 'else'} branch of a language condition"
                if lit.side is None:
                    lit.side = "?"
                break
            if f.kind == "match" and f.match_lang and not f.in_pattern:
                pats = [q.text for q in f.arm_pattern if q.kind == "str"]
                lit.side = "en" if "en" in pats else "nl" if "nl" in pats else "?"
                lit.side_reason = "arm of a match on the language"
                break
            if f.kind in ("call", "macro") and f.callee in self.pair_fns:
                nl_i, en_i = self.pair_fns[f.callee]
                if f.argidx in (nl_i, en_i):
                    lit.side = "nl" if f.argidx == nl_i else "en"
                    lit.side_reason = f"{f.callee}() nl/en parameter"
                break
            if f.kind == "jobj" and f.key in ("nl", "en"):
                lit.side = f.key
                lit.side_reason = '{"nl":..,"en":..} object'
                break
            if f.kind == "fnbody":
                break
        self.lits.append(lit)


# ---------------------------------------------------------------------------------------------------- functions

def fn_signatures(toks: list[Tok]) -> list[tuple[str, list[str], bool]]:
    """[(name, param names without self, is_method)] for every fn in a token stream."""
    out = []
    for i, tok in enumerate(toks):
        if tok.text != "fn" or tok.kind != "ident" or i + 2 >= len(toks) or toks[i + 1].kind != "ident":
            continue
        name = toks[i + 1].text
        j = i + 2
        if toks[j].text == "<":
            d = 0
            while j < len(toks):
                if toks[j].text == "<":
                    d += 1
                elif toks[j].text == ">":
                    d -= 1
                    if d == 0:
                        break
                elif toks[j].text == ">>":
                    d -= 2
                    if d <= 0:
                        break
                j += 1
            j += 1
        if j >= len(toks) or toks[j].text != "(":
            continue
        d, k, params, cur = 0, j, [], []
        while k < len(toks):
            x = toks[k].text
            if x in "([{<" and toks[k].kind == "punct":
                d += 1
                if d > 1:
                    cur.append(toks[k])
            elif x in ")]}>" and toks[k].kind == "punct":
                d -= 1
                if d == 0:
                    break
                cur.append(toks[k])
            elif x == "," and d == 1:
                params.append(cur)
                cur = []
            elif d >= 1:
                cur.append(toks[k])
            k += 1
        if cur:
            params.append(cur)
        names = []
        method = False
        for p in params:
            idents = [q.text for q in p if q.kind == "ident"]
            if "self" in idents[:3] and ":" not in [q.text for q in p][:3]:
                method = True
                continue
            colon = [q.text for q in p].index(":") if ":" in [q.text for q in p] else -1
            pname = [q.text for q in p[:colon] if q.kind == "ident" and q.text not in ("mut", "ref")]
            names.append(pname[-1] if pname else "_")
        out.append((name, names, method))
    return out


def resolve_tables(sigs_by_file: dict[str, list]) -> tuple[dict, dict, dict]:
    """Per-file callee tables: pair fns (nl/en params), message fns, code fns."""
    by_name: dict[tuple[str, bool], list[tuple[str, list[str], bool]]] = defaultdict(list)
    for file, sigs in sigs_by_file.items():
        for name, params, method in sigs:
            by_name[(name, method)].append((file, params, method))

    def pick(file: str, name: tuple[str, bool]):
        cands = by_name.get(name, [])
        if not cands:
            return None
        here = Path(file)
        d, child = str(here.parent), str(here.parent / here.stem)
        for rank in (lambda c: c[0] == file, lambda c: str(Path(c[0]).parent) == d,
                     lambda c: str(Path(c[0]).parent) == child,
                     lambda c: d.startswith(str(Path(c[0]).parent) + "/") or str(Path(c[0]).parent) == "."):
            hit = [c for c in cands if rank(c)]
            if len(hit) == 1 or (hit and rank is not None and hit[0][0] == file):
                return hit[0]
            if len(hit) > 1:
                return sorted(hit)[0]
        return None  # a same-named fn elsewhere is not in scope (closures such as `let t = |..|` shadow it)

    pair_t, msg_t, code_t = {}, {}, {}
    for file in sigs_by_file:
        pair, msg, code = {}, {}, {}
        for name, method in sorted(by_name, key=lambda k: (k[1], k[0])):  # methods last: Self:: prefers them
            c = pick(file, (name, method))
            if not c:
                continue
            _, params, method = c
            keys = ["." + name, "Self::" + name] if method else [name, "Self::" + name]
            if not method:
                keys += [m + "::" + name for m in ("common", "super", "self", "crate")]
            if "nl" in params and "en" in params:
                for key in keys:
                    pair[key] = (params.index("nl"), params.index("en"))
            for pn in ("message", "msg", "label", "title", "nl"):
                if pn in params:
                    for key in keys:
                        msg[key] = params.index(pn)
                    break
            if "code" in params:
                for key in keys:
                    code[key] = params.index("code")
        msg["ApiError::new"] = 1
        code["ApiError::new"] = 0
        pair_t[file], msg_t[file], code_t[file] = pair, msg, code
    return pair_t, msg_t, code_t


# ---------------------------------------------------------------------------------------------------- classify

WEAK_TECH = {"GVariant signature or lowercase token", "snake/kebab identifier", "camelCase identifier",
             "CamelCase identifier", "constant, unit or env name", "lowercase token with digits", "prefixed token",
             "command-line flag", "glob", "token list", "dotted prefix"}


def tech_reason(text: str, lang: str = "none") -> str | None:
    """Why a literal is technical, or None. Dutch text is only technical for strong shapes (paths, ids, code)."""
    s = text.strip()
    if s in TECH_PHRASES:
        return "keyfile group, product or protocol name"
    if s in HTTP_REASONS:
        return "HTTP reason phrase"
    if s in LANG_CODES:
        return "language or country code"
    bare = PH.sub("", s).strip()
    dutch = lang in ("nl", "mixed")
    for rx, why in TECH_PATTERNS:
        probe = bare if why == "CSS or code" else s
        if rx.search(probe) and not (dutch and why in WEAK_TECH):
            return why
    if bare != s and bare and " " not in bare and not dutch:
        for rx, why in TECH_PATTERNS:
            if rx.search(bare):
                return why + " (after placeholders)"
    return None


def display_context(lit: Lit, msg_fns: dict) -> tuple[str, bool]:
    """(why, strong) when the literal flows, through transparent wrappers only, into something shown."""
    for kind, callee, argidx, key in lit.chain:
        if kind in ("call", "macro"):
            if callee in msg_fns and msg_fns[callee] >= 0:
                if argidx == msg_fns[callee]:
                    return f"message of {callee}", True
                return "", False
            if callee in (".title", ".set_title"):
                return f"window title {callee}", True
            if callee in TRANSPARENT:
                continue
            if callee in ("Err", ".ok_or", ".ok_or_else", ".map_err", "io::Error::new", "std::io::Error::new",
                          "io::Error::other", "std::io::Error::other"):
                return f"returned via {callee}", False
            return "", False
        if kind in ("jobj", "struct") and key:
            return (f'value of "{key}"', True) if key in USER_KEYS else ("", False)
        if kind == "fnbody":
            break
    for b in lit.binding.split(","):
        if b and STRONG_DISPLAY.match(b):
            return f"binding {b}", True
    for b in lit.binding.split(","):
        if b and DISPLAY_NAME.search(b):
            return f"binding {b}", False
    if lit.fn and DISPLAY_NAME.search(lit.fn):
        return f"in fn {lit.fn}", False
    return "", False


def classify(lit: Lit, words: Words, msg_fns: dict, code_fns: dict) -> None:
    text = lit.text
    lit.lang = words.lang(text) if lit.flavor not in ("bytes", "rawbytes") else "none"
    callees = [(k, c, a) for k, c, a, _ in lit.chain if k in ("call", "macro")]
    inner_call = callees[0] if callees else ("", "", 0)
    if lit.flavor in ("bytes", "rawbytes"):
        lit.cls, lit.reason = "technical", "byte string"
        return
    if lit.role:
        why = {"json-key": "JSON key", "compare": "comparison operand", "pattern": "match/matches! pattern",
               "index": "index key", "attribute": "attribute argument"}[lit.role]
        if lit.lang in ("nl", "mixed") and lit.role in ("pattern", "compare"):
            why += " (Dutch input word or matched text, not shown)"
        lit.cls, lit.reason = "technical", why
        return
    for k, c, a in callees:
        if c in LOG_MACROS or c in LOG_CALLS:
            lit.cls, lit.reason = "log", f"argument of {c}"
            return
        if c == "matches!" and a >= 1:
            lit.cls, lit.reason = "technical", "matches! pattern"
            return
        if c in TRANSPARENT:
            continue
        break
    if inner_call[0] == "call" and inner_call[1] in STRING_OPS:
        lit.cls, lit.reason = "technical", f"string/lookup operation {inner_call[1]}"
        return
    letters = bool(re.search(r"[A-Za-zÀ-ÖØ-öø-ÿ]", text))
    tech = tech_reason(text, lit.lang)
    if lit.side and text.strip() in LANG_CODES:
        lit.side = None
        lit.cls, lit.reason = "technical", "language or country code"
        return
    if lit.side and letters and tech not in ("path", "URI", "dotted id (method, D-Bus, file name)",
                                              "path or MIME type", "markup/XML", "command-line flag",
                                              "snake/kebab identifier", "camelCase identifier", "dotted prefix"):
        side = lit.side if lit.side != "?" else (lit.lang if lit.lang in ("nl", "en") else "?")
        lit.side = side
        lit.cls, lit.reason = "bilingual-pair", f"{side} half: {lit.side_reason}"
        return
    lit.side = None
    if callees and callees[0][1] in code_fns and callees[0][2] == code_fns[callees[0][1]]:
        lit.cls, lit.reason = "technical", f"error code ({callees[0][1]} arg {callees[0][2]})"
        return
    if tech:
        lit.cls, lit.reason = "technical", tech
        return
    display, strong = display_context(lit, msg_fns)
    words_all, nl, en = words.split(text)
    stripped = text.strip()
    multiword = len(words_all) >= 2 and " " in stripped
    tech_call = next((c for k, c, a in callees if c in TECH_CALLS), "")
    if lit.lang in ("nl", "mixed"):
        why = display or (f"Dutch text in {tech_call}" if tech_call else "Dutch text")
        if lit.lang == "mixed":
            why = "mixed Dutch/English text" + (f"; {display}" if display else "")
        lit.cls, lit.reason = "user-visible-dutch", why
        return
    if tech_call and not strong:
        lit.cls, lit.reason = "technical", f"argument of {tech_call}"
        return
    if lit.lang == "en":
        capital = bool(re.match(r"^[A-Z][a-z]", stripped))
        sentence = multiword and (capital or bool(re.search(r"[.!?]$", stripped)))
        if strong:
            lit.cls, lit.reason = "user-visible-english", display
        elif sentence:
            lit.cls, lit.reason = "user-visible-english", "English sentence or phrase" + (f"; {display}" if display else "")
        elif multiword and display:
            lit.cls, lit.reason = "technical", f"lowercase English detail ({display})"
        elif multiword:
            lit.cls, lit.reason = "technical", "lowercase English phrase outside a display context"
        else:
            lit.cls, lit.reason = "technical", "English word outside a display context"
        return
    if strong and multiword:
        lit.cls, lit.reason = "technical", f"language-neutral text ({display})"
        return
    lit.cls, lit.reason = "technical", "language-neutral token"


def tuple_pairs(lits: list[Lit]) -> None:
    """Adjacent nl/en literals in one tuple/array frame (own_names, NAMES tables ...)."""
    groups: dict[int, list[Lit]] = defaultdict(list)
    for lit in lits:
        if lit.frame_kind in ("tuple", "array", "jarray") and not lit.role:
            groups[lit.frame_id].append(lit)
    for group in groups.values():
        for a, b in zip(group, group[1:]):
            if (a.cls == "user-visible-dutch" and a.lang == "nl" and b.lang == "en" and a.side is None
                    and b.side is None and b.cls in ("user-visible-english", "technical")
                    and not tech_reason(b.text)):
                a.cls, a.side, a.reason = "bilingual-pair", "nl", "nl half: adjacent nl/en literals in a tuple/array"
                b.cls, b.side, b.reason = "bilingual-pair", "en", "en half: adjacent nl/en literals in a tuple/array"


# ---------------------------------------------------------------------------------------------------- JSON tables

def json_strings(src: str) -> list[tuple[list, str, int, int]]:
    """[(path, value, line, col)] for every string value (not keys) of a JSON document."""
    out = []
    i, n = 0, len(src)
    line_starts = [0] + [k + 1 for k, ch in enumerate(src) if ch == "\n"]

    def pos(k: int) -> tuple[int, int]:
        lo, hi = 0, len(line_starts) - 1
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if line_starts[mid] <= k:
                lo = mid
            else:
                hi = mid - 1
        return lo + 1, k - line_starts[lo] + 1

    def ws():
        nonlocal i
        while i < n and src[i] in " \t\r\n":
            i += 1

    def string() -> str:
        nonlocal i
        start = i
        i += 1
        while src[i] != '"':
            i += 2 if src[i] == "\\" else 1
        i += 1
        return json.loads(src[start:i])

    def value(path):
        nonlocal i
        ws()
        c = src[i]
        if c == "{":
            i += 1
            ws()
            if src[i] == "}":
                i += 1
                return
            while True:
                ws()
                k = string()
                ws()
                i += 1  # :
                value(path + [k])
                ws()
                if src[i] == ",":
                    i += 1
                    continue
                i += 1
                return
        if c == "[":
            i += 1
            ws()
            if src[i] == "]":
                i += 1
                return
            idx = 0
            while True:
                value(path + [idx])
                idx += 1
                ws()
                if src[i] == ",":
                    i += 1
                    continue
                i += 1
                return
        if c == '"':
            ln, col = pos(i)
            out.append((path, string(), ln, col))
            return
        m = re.compile(r"-?[0-9.eE+-]+|true|false|null").match(src, i)
        i = m.end()

    value([])
    return out


def json_rule(rel: str, path: list) -> tuple[str, str | None, str]:
    """(class, side, reason) for a JSON table string."""
    p = [str(x) for x in path]
    if p[:1] == ["_STR"] and len(p) >= 3:
        return "bilingual-pair", p[1], f"{p[1]} half: _STR table"
    if rel.endswith("system/strings.json"):
        if p[0] == "system" and len(p) >= 3:
            return "bilingual-pair", p[1], f"{p[1]} half: system strings table"
        if p[0] == "wifi" and len(p) == 3:
            side = "nl" if p[2] == "0" else "en"
            return "bilingual-pair", side, f"{side} half: Wi-Fi [nl, en] pair"
    if rel.endswith("sysmon/text.json"):
        if p[0] == "_FRIENDLY" and len(p) == 3:
            side = "nl" if p[2] == "0" else "en"
            return "bilingual-pair", side, f"{side} half: _FRIENDLY [nl, en] process name"
        if p[0] in ("_GSD_NL", "_GVFS_NL"):
            return "user-visible-dutch", None, f"{p[0]}: Dutch-only unit-name fragment (by design)"
    if rel.endswith("apps/catalog-data.json"):
        if p[0] == "ownApps" and len(p) == 4 and p[3] in ("nl", "en"):
            return "bilingual-pair", p[3], f"{p[3]} half: own app {p[2]}"
        if p[0] == "ownApps" and len(p) >= 3 and p[2] == "keywords":
            return "technical", None, "search keyword (matched, not shown)"
        if p[0] == "twins" and len(p) == 3:
            return "bilingual-pair", p[2], f"{p[2]} half: twin app name"
        if p[0] in ("nlNames", "nlComments"):
            return "user-visible-dutch", None, f"{p[0]}: Dutch override for a GNOME app"
    if rel.endswith("files/types.json") and len(p) == 2:
        side = "nl" if p[1] == "0" else "en"
        return "bilingual-pair", side, f"{side} half: MIME type name"
    if rel.endswith("files/launcher-names.json"):
        if p[0] == "NL_NAMES":
            return "user-visible-dutch", None, "NL_NAMES: Dutch override for a GNOME app"
        if p[0] == "SAME_NAME_AS_OURS" and len(p) == 3:
            return "bilingual-pair", p[2], f"{p[2]} half: twin app name"
    return "technical", None, "table data (ids, file systems, paths)"


JSON_TABLES = ("services/disks/data.json", "services/storage/data.json", "services/sysmon/text.json",
               "services/system/strings.json", "apps/catalog-data.json", "files/types.json",
               "files/launcher-names.json")


# ---------------------------------------------------------------------------------------------------- driver

def module_children(rel: Path, name: str, path_attr: str | None) -> list[Path]:
    base = rel.parent
    if path_attr:
        return [base / path_attr]
    if rel.name in ("mod.rs", "lib.rs", "main.rs"):
        d = base
    else:
        d = base / rel.stem
    return [d / f"{name}.rs", d / name / "mod.rs"]


def esc(s: str) -> str:
    return s.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")


def h(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8", "surrogatepass")).hexdigest()[:12]


def scan(root: Path) -> tuple[list[Lit], dict]:
    src_root = root / "host/src"
    files = sorted(p.relative_to(src_root) for p in src_root.rglob("*.rs"))
    toks = {f: lex((src_root / f).read_text(encoding="utf-8")) for f in files}
    sigs = {f.as_posix(): fn_signatures(toks[f]) for f in files}
    pair_t, msg_t, code_t = resolve_tables(sigs)
    scanners = {}
    for f in files:
        s = FileScanner(f.as_posix(), toks[f], pair_t[f.as_posix()], msg_t[f.as_posix()], code_t[f.as_posix()])
        s.run()
        scanners[f] = s
    # test files: cfg(test) mod declarations and everything they declare
    test_files: set[Path] = set()
    queue: list[Path] = []
    for f, s in scanners.items():
        for name, pattr, cfg_test in s.mods:
            if cfg_test:
                queue += [c for c in module_children(f, name, pattr) if (src_root / c).is_file()]
    while queue:
        c = (src_root / queue.pop()).resolve().relative_to(src_root.resolve())
        if c in test_files or c not in scanners:
            continue
        test_files.add(c)
        for name, pattr, _ in scanners[c].mods:
            queue += [k for k in module_children(c, name, pattr) if (src_root / k).is_file()]
    words = Words(root)
    lits: list[Lit] = []
    stats = {"files": len(files), "test_files": sorted(p.as_posix() for p in test_files), "api_error_sites": 0,
             "skipped_test_items": 0, "word_sources": words.sources}
    for f in files:
        if f in test_files:
            continue
        s = scanners[f]
        stats["api_error_sites"] += s.api_error_sites
        stats["skipped_test_items"] += s.skipped_tests
        for lit in s.lits:
            classify(lit, words, s.msg_fns, s.code_fns)
        tuple_pairs(s.lits)
        lits += s.lits
    for rel in JSON_TABLES:
        p = src_root / rel
        for path, value, ln, col in json_strings(p.read_text(encoding="utf-8")):
            cls, side, why = json_rule(rel, path)
            lit = Lit(rel, ln, col, value, "json", source="json")
            lit.lang = words.lang(value)
            if cls == "technical" and lit.lang in ("nl", "mixed") and "keywords" not in path:
                cls, why = "user-visible-dutch", "Dutch label in table data (" + ".".join(map(str, path[:1])) + ")"
            lit.cls, lit.side, lit.reason = cls, side, why
            lit.context = "json:" + ".".join(str(x) for x in path)
            lits.append(lit)
    # sanity: every #[test] lies in a test file or a skipped cfg(test) item
    total = stray = in_test_files = 0
    for f in files:
        tk = toks[f]
        for k in range(2, len(tk) - 1):
            if tk[k].text == "test" and tk[k - 1].text == "[" and tk[k - 2].text == "#" and tk[k + 1].text == "]":
                total += 1
                if f in test_files:
                    in_test_files += 1
                elif not any(a <= k - 2 < b for a, b in scanners[f].skipped_ranges):
                    stray += 1
    stats["test_attrs_total"] = total
    stats["test_attrs_in_test_files"] = in_test_files
    stats["test_attrs_outside_excluded_code"] = stray
    return lits, stats


def context_of(lit: Lit) -> str:
    if lit.context:
        return lit.context
    parts = []
    for kind, callee, argidx, key in lit.chain[:4]:
        if kind in ("call", "macro"):
            parts.append(f"{callee}#{argidx}")
        elif kind in ("jobj", "struct"):
            parts.append(f"{callee}{{{key or ''}}}")
        elif kind in ("tuple", "array", "jarray"):
            parts.append(f"{'()' if kind == 'tuple' else '[]'}#{argidx}")
        elif kind == "fnbody":
            parts.append(f"fn {callee}")
        elif kind in ("then", "else", "match", "index", "attr"):
            parts.append(kind if kind != "attr" else callee)
    if lit.binding:
        parts.append(f"let {lit.binding}")
    return " < ".join(parts)


SCOPE_SERVICES = re.compile(r"^(services/|settings\.rs|settings_effects\.rs|settings_runtime\.rs|runtime\.rs|"
                            r"backend\.rs|events\.rs)")


def write_outputs(lits: list[Lit], stats: dict, out_all: Path, out_frozen: Path) -> dict:
    lits.sort(key=lambda x: (x.file, x.line, x.col))
    occ: Counter = Counter()
    rows = []
    for lit in lits:
        hh = h(lit.text)
        occ[(lit.file, hh)] += 1
        ident = f"{lit.file}#{hh}#{occ[(lit.file, hh)]}"
        rows.append((lit.file, str(lit.line), str(lit.col), ident, lit.source if lit.source == "json" else lit.flavor,
                     lit.cls, lit.lang, lit.side or "", esc(context_of(lit)), esc(lit.key or ""), esc(lit.reason),
                     esc(lit.text)))
    header = ("# tests/i18n/rust-literals.tsv: every string literal of host/src outside test code, plus every string "
              "of the embedded JSON text tables, classified by tools/i18n/scan_rust.py (GE0b, baseline 3b898e6).\n"
              "# Regenerate: python3 -B tools/i18n/scan_rust.py. Columns as below; text and context escape \\\\ \\t "
              "\\n \\r. id = file#sha256(text)[:12]#occurrence.\n")
    cols = "file\tline\tcol\tid\tsource\tclass\tlang\tside\tcontext\tkey\treason\ttext\n"
    out_all.parent.mkdir(parents=True, exist_ok=True)
    out_all.write_text(header + cols + "".join("\t".join(r) + "\n" for r in rows), encoding="utf-8")
    frozen: dict[tuple[str, str], list] = {}
    for lit in lits:
        marker = lit.cls == "technical" and lit.reason == "file marker" and lit.lang in ("nl", "mixed")
        if lit.cls == "user-visible-dutch" or (lit.cls == "bilingual-pair" and lit.side == "nl") or marker:
            k = (lit.file, h(lit.text))
            if k not in frozen:
                frozen[k] = [lit.file, h(lit.text), 0, set(), lit.text]
            frozen[k][2] += 1
            frozen[k][3].add("user-visible-dutch" if lit.cls == "user-visible-dutch" else
                             "pair-nl" if lit.cls == "bilingual-pair" else "dutch-file-marker")
    fheader = ("# tests/i18n/rust-nl-frozen.tsv: every Dutch user-visible text of host/src (classes user-visible-dutch, "
               "the nl half of bilingual-pair, and Dutch file markers) at baseline 3b898e6,\n# keyed by file + sha256(text)[:12], never by "
               "line. count = occurrences in that file. Generated by tools/i18n/scan_rust.py; do not edit by hand: "
               "an intended Dutch change goes into tests/i18n/BASELINE-CHANGES.\n")
    fcols = "file\thash\tcount\tclass\ttext\n"
    frows = sorted(frozen.values(), key=lambda r: (r[0], r[4], r[1]))
    out_frozen.write_text(fheader + fcols + "".join(
        f"{r[0]}\t{r[1]}\t{r[2]}\t{'+'.join(sorted(r[3]))}\t{esc(r[4])}\n" for r in frows), encoding="utf-8")
    # summary
    summary: dict = {"word_sources": stats["word_sources"], "rust_files": stats["files"],
                     "test_files_excluded": len(stats["test_files"]),
                     "cfg_test_items_skipped": stats["skipped_test_items"],
                     "api_error_new_sites": stats["api_error_sites"]}
    rust = [x for x in lits if x.source != "json"]
    js = [x for x in lits if x.source == "json"]
    summary["literals_rust"] = len(rust)
    summary["strings_json"] = len(js)
    summary["by_class_rust"] = dict(sorted(Counter(x.cls for x in rust).items()))
    summary["by_class_json"] = dict(sorted(Counter(x.cls for x in js).items()))
    for name, rx in (("services_scope", SCOPE_SERVICES),):
        inside = [x for x in rust if rx.match(x.file)]
        outside = [x for x in rust if not rx.match(x.file)]
        for label, group in ((name, inside), ("rest_of_host", outside)):
            nl = [x for x in group if x.cls == "user-visible-dutch"]
            summary[f"{label}_dutch_sites"] = len(nl)
            summary[f"{label}_dutch_unique"] = len({x.text for x in nl})
            summary[f"{label}_pairs"] = Counter(x.side for x in group if x.cls == "bilingual-pair")
            summary[f"{label}_english_visible"] = sum(1 for x in group if x.cls == "user-visible-english")
            summary[f"{label}_logs_dutch"] = sum(1 for x in group if x.cls == "log" and x.lang in ("nl", "mixed"))
    for key in ("test_attrs_total", "test_attrs_in_test_files", "test_attrs_outside_excluded_code"):
        summary[key] = stats[key]
    summary["api_error_new_dutch_literal_message"] = sum(
        1 for x in rust if x.chain and x.chain[0][:3] == ("call", "ApiError::new", 1) and x.cls == "user-visible-dutch")
    summary["api_error_new_dutch_format_message"] = sum(
        1 for x in rust if len(x.chain) > 1 and x.chain[0][1] == "format!" and x.chain[0][2] == 0
        and x.chain[1][:3] == ("call", "ApiError::new", 1) and x.cls == "user-visible-dutch")
    summary["literals_with_dutch_word"] = sum(1 for x in rust if x.lang in ("nl", "mixed"))
    by_dir = Counter()
    for x in rust:
        if x.cls == "user-visible-dutch":
            top = x.file.split("/")[0] if "/" in x.file else x.file
            if x.file.startswith("services/"):
                top = "/".join(x.file.split("/")[:2]).removesuffix(".rs")
            by_dir[top] += 1
    summary["dutch_sites_by_area"] = dict(sorted(by_dir.items()))
    summary["frozen_rows"] = len(frows)
    summary["frozen_occurrences"] = sum(r[2] for r in frows)
    return summary


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    ap.add_argument("--out-all", type=Path)
    ap.add_argument("--out-frozen", type=Path)
    ap.add_argument("--summary", action="store_true", help="print counts as JSON on stdout")
    a = ap.parse_args()
    root = a.root.resolve()
    lits, stats = scan(root)
    summary = write_outputs(lits, stats, a.out_all or root / "tests/i18n/rust-literals.tsv",
                            a.out_frozen or root / "tests/i18n/rust-nl-frozen.tsv")
    summary["test_files"] = stats["test_files"]
    if a.summary:
        print(json.dumps(summary, ensure_ascii=False, indent=1, default=lambda o: dict(sorted(o.items(),
              key=lambda kv: str(kv[0])))))
    return 0


if __name__ == "__main__":
    sys.exit(main())
