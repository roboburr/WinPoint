#!/usr/bin/env bash
# GW5: the two root helpers, the two polkit policies, the root scripts of scripts/install-system.py, its dry run and
# its refusals, and the installer hint (notes/claude/windows-update-gnome.md §6.3, §7; notes/claude/wu/gw5.md).
#
#   tests/update/test_helpers.sh        every check; exit 0 when all pass, 1 otherwise
#
# Nothing here runs as root, runs pkexec, or touches /etc, /usr or the live session:
# * the helpers run as you on a temporary etc (WIN11_HELPER_ETC, honoured only when not root), the install/remove
#   scripts on a temporary root (WIN11_SYSTEM_ROOT, the same rule), both from `env -i`;
# * scripts/install-system.py runs only with --dry-run or with arguments it refuses, inside bubblewrap (read-only
#   system, your home hidden, this repository read-only, only the case directory writable, no network, private /tmp,
#   /run/user and /run/dbus) with a logging pkexec stub first on PATH, under strace: its only writes must be its
#   stage directory under the private XDG_RUNTIME_DIR. The root refusal runs in a user namespace mapped to uid 0.
# Work files go to a fresh directory under ${W11_TEST_TMP:-<repo>/.rt/test-helpers} that is removed at the end
# (KEEP=1 keeps it).
set -uo pipefail
G=$(cd "$(dirname "$0")/../.." && pwd -P)
[ "$(id -u)" != 0 ] || { echo "test_helpers.sh: run this as yourself, not as root" >&2; exit 2; }
BASE=${W11_TEST_TMP:-$G/.rt/test-helpers}
mkdir -p "$BASE"
T=$(mktemp -d "$BASE/run.XXXXXX")
[ "${KEEP:-0}" = 1 ] || trap 'rm -rf "$T"' EXIT
HELPER=$G/host/data/release-upgrades-helper
ATTACH=$G/host/data/release-upgrade-attach
FIX=$G/tests/update/fixtures/release
DEFAULT=$FIX/etc/update-manager/release-upgrades
PASS=0 FAIL=0
ok() { PASS=$((PASS + 1)); printf 'ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf 'FAIL %s\n' "$1"; [ -z "${2:-}" ] || printf '%s\n' "$2" | sed 's/^/     /'; }
# check NAME COMMAND…: ok when the command succeeds
check() { local name=$1; shift; if "$@"; then ok "$name"; else bad "$name"; fi; }
# same NAME WANT GOT
same() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "$(printf 'want: %q\ngot:  %q' "$2" "$3")"; fi; }

# ------------------------------------------------------------------------------------------- the Prompt helper

N=0
# mketc [file-content]: a fresh temp etc with update-manager/ and release-upgrades.d/ (the .d file of this machine)
mketc() {
  N=$((N + 1))
  local etc=$T/etc$N
  mkdir -p "$etc/update-manager/release-upgrades.d"
  cp "$FIX/etc/update-manager/release-upgrades.d/ubuntu-advantage-upgrades.cfg" "$etc/update-manager/release-upgrades.d/"
  if [ "$#" -gt 0 ]; then printf '%s' "$1" > "$etc/update-manager/release-upgrades"; fi
  ETC=$etc
  FILE=$etc/update-manager/release-upgrades
}
# helper ETC|- ARGS…: run it as you from env -i; sets OUT and RC
helper() {
  local etc=$1; shift
  local env=(env -i PATH=/usr/bin:/bin)
  [ "$etc" = - ] || env+=("WIN11_HELPER_ETC=$etc")
  RC=0
  OUT=$("${env[@]}" "$HELPER" "$@" 2>&1) || RC=$?
}

check "helper: sh -n" sh -n "$HELPER"
check "helper: executable" test -x "$HELPER"
mketc "$(cat "$DEFAULT")
"
before=$(sha256sum < "$FILE")
for args in "" "set-prompt" "set-prompt lts extra" "get-prompt lts" "set-prompt LTS" "set-prompt no" "-- set-prompt lts" "set-prompt ''"; do
  eval "helper \"\$ETC\" $args"
  same "helper refuses '$args': rc 2" 2 "$RC"
  same "helper refuses '$args': marker" "@@fail usage" "$OUT"
done
same "refused calls changed nothing" "$before" "$(sha256sum < "$FILE")"
helper - set-prompt lts
same "not root, no WIN11_HELPER_ETC: refused" "3 @@fail notroot" "$RC $OUT"
helper relative/etc set-prompt lts
same "a relative WIN11_HELPER_ETC: refused" "3 @@fail etc" "$RC $OUT"

# Ubuntu's own file: only the Prompt line changes, and back again byte for byte
cmp -s "$FILE" "$DEFAULT" && ok "the temp etc holds Ubuntu's default file" || bad "fixture copy"
helper "$ETC" set-prompt lts
same "lts: output" "@@ok prompt=lts" "$OUT"
same "lts: rc" 0 "$RC"
same "lts: only the Prompt line differs" "< Prompt=normal
---
> Prompt=lts" "$(diff "$DEFAULT" "$FILE" | sed 1d)"
same "lts: mode 0644, owned by you" "644 $(id -u):$(id -g)" "$(stat -c '%a %u:%g' "$FILE")"
check "lts: no temp file left" test ! -e "$ETC/update-manager/release-upgrades.win11-gnome.tmp"
helper "$ETC" set-prompt never
same "never" "@@ok prompt=never" "$OUT"
grep -qx 'Prompt=never' "$FILE" && ok "never: written" || bad "never: written"
helper "$ETC" set-prompt normal
check "normal again: byte-identical to Ubuntu's file" cmp -s "$FILE" "$DEFAULT"
# the fallback runs the same text through sh -c (admin.rs: pkexec /bin/sh -c '<script>' win11-gnome-prompt …)
RC=0; OUT=$(env -i PATH=/usr/bin:/bin WIN11_HELPER_ETC="$ETC" /bin/sh -c "$(cat "$HELPER")" win11-gnome-prompt set-prompt lts 2>&1) || RC=$?
same "fallback through sh -c: output" "0 @@ok prompt=lts" "$RC $OUT"
grep -qx 'Prompt=lts' "$FILE" && ok "fallback: written" || bad "fallback: written"
RC=0; OUT=$(env -i PATH=/usr/bin:/bin WIN11_HELPER_ETC="$ETC" bash --posix "$HELPER" set-prompt normal 2>&1) || RC=$?
same "bash --posix runs it too" "0 @@ok prompt=normal" "$RC $OUT"
check "bash: byte-identical again" cmp -s "$FILE" "$DEFAULT"

# rewrite rules: (input, value, expected output)
rule() {
  local name=$1 input=$2 value=$3 want=$4
  mketc "$input"
  helper "$ETC" set-prompt "$value"
  same "rule $name: output" "@@ok prompt=$value" "$OUT"
  same "rule $name: file" "$want" "$(cat "$FILE"; printf x)"
}
rule "a key after the DEFAULT section's last line, before the next section" \
  '# comment
[DEFAULT]
# just a comment
Foo=bar

[Other]
Prompt=lts
' never '# comment
[DEFAULT]
# just a comment
Foo=bar
Prompt=never

[Other]
Prompt=lts
x'
rule "case, colon and spaces of the key; its indentation stays (a deeper next line stays a key of its own)" '[DEFAULT]
  prompt : lts
  Other=1
' normal '[DEFAULT]
  Prompt=normal
  Other=1
x'
rule "continuation lines and a duplicate go" '[DEFAULT]
Prompt=lts
  continued
# kept
Prompt=never
Other=1
' normal '[DEFAULT]
Prompt=normal
# kept
Other=1
x'
rule "a file without DEFAULT" '# only a comment
[Sources]
AllowThirdParty=yes
' lts '# only a comment
[Sources]
AllowThirdParty=yes
[DEFAULT]
Prompt=lts
x'
rule "no newline at the end" '[DEFAULT]
Prompt=normal' lts '[DEFAULT]
Prompt=lts
x'
rule "DEFAULT at the end, with blank lines" '[DEFAULT]
X=1


' never '[DEFAULT]
X=1
Prompt=never


x'
mketc
rm -f "$FILE"
helper "$ETC" set-prompt lts
same "a missing file is created" "@@ok prompt=lts|[DEFAULT]
Prompt=lts" "$OUT|$(cat "$FILE")"
same "created 0644" 644 "$(stat -c %a "$FILE")"

# The helper reads files as Ubuntu does (GW5 fix round, m1): the corpus in fixtures/release/cfg, its configparser
# verdicts in expected.tsv. For each file: python3's configparser (MetaRelease._parse_cfg) still gives the recorded
# verdict; as the main file, each set-prompt either answers @@ok and configparser then reads exactly that value, or
# answers @@fail parse and leaves the file byte for byte (and the expected one of the two happens); as a .d file, the
# helper names it as an override exactly when configparser reads a Prompt from it (unless it is "unjudged").
CORPUS=$FIX/cfg
CASES=$(grep -vc '^#' "$CORPUS/expected.tsv")
RC=0; OUT=$(python3 -I - "$CORPUS" "$HELPER" "$T/corpus" <<'EOF' 2>&1
import configparser, subprocess, sys
from pathlib import Path
corpus, helper, work = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
def verdict(path):
    parser = configparser.ConfigParser()
    try:
        parser.read(path, encoding="utf-8")
    except configparser.Error:
        return "error"
    if parser.has_option("DEFAULT", "Prompt"):
        return "value:" + parser.get("DEFAULT", "Prompt").lower().replace("\n", "\\n")
    return "none"
def run(etc, value):
    env = {"PATH": "/usr/bin:/bin", "WIN11_HELPER_ETC": str(etc)}
    return subprocess.run([helper, "set-prompt", value], env=env, capture_output=True, text=True).stdout.splitlines()
problems, files, runs = [], 0, 0
for line in (corpus / "expected.tsv").read_text().splitlines():
    if line.startswith("#") or not line:
        continue
    name, want, outcome, override = line.split("\t")
    files += 1
    source = corpus / f"{name}.cfg"
    data = source.read_bytes()
    if verdict(source) != want:
        problems.append(f"{name}: configparser now reads {verdict(source)}, expected.tsv says {want}")
    for value in ("lts", "normal", "never"):
        runs += 1
        etc = work / f"{name}-{value}"
        (etc / "update-manager/release-upgrades.d").mkdir(parents=True)
        main = etc / "update-manager/release-upgrades"
        main.write_bytes(data)
        out = run(etc, value)
        if out[:1] == [f"@@ok prompt={value}"]:
            got = "ok"
            if verdict(main) != f"value:{value}":
                problems.append(f"{name} {value}: @@ok, but Ubuntu reads {verdict(main)} (a silent no-op)")
        elif out == ["@@fail parse"]:
            got = "parse"
            if main.read_bytes() != data:
                problems.append(f"{name} {value}: @@fail parse changed the file")
        else:
            got = repr(out)
        if got != outcome:
            problems.append(f"{name} {value}: {got}, expected {outcome}")
        if sorted(p.name for p in (etc / "update-manager").iterdir()) != ["release-upgrades", "release-upgrades.d"]:
            problems.append(f"{name} {value}: a temp file was left")
        if override == "unjudged":
            continue
        d = work / f"{name}-{value}-d"
        (d / "update-manager/release-upgrades.d").mkdir(parents=True)
        (d / "update-manager/release-upgrades").write_text("[DEFAULT]\nPrompt=normal\n")
        cfg = d / "update-manager/release-upgrades.d/50-corpus.cfg"
        cfg.write_bytes(data)
        named = f"@@override {cfg}" in run(d, value)
        if named != (override == "yes") or (override == "yes") != want.startswith("value:"):
            problems.append(f"{name} {value}: as a .d file named={named}, expected {override}")
        if cfg.read_bytes() != data:
            problems.append(f"{name} {value}: the .d file was changed")
print(f"files={files} runs={runs} problems={len(problems)}")
print("\n".join(problems[:30]))
EOF
) || RC=$?
same "corpus: the helper reads and writes as Ubuntu's configparser does (no silent no-op; a refusal keeps the file)" \
  "0 files=$CASES runs=$((CASES * 3)) problems=0" "$RC $(printf '%s\n' "$OUT" | head -1)"
[ "$(printf '%s\n' "$OUT" | grep -c .)" -le 1 ] || printf '%s\n' "$OUT" | sed 's/^/     /'
# the reviewer's probes by name: a CRLF file keeps its CRLF line ends; a header with a comment after it is DEFAULT;
# a file Ubuntu ignores is refused and kept
mketc; cp "$CORPUS/crlf.cfg" "$FILE"
helper "$ETC" set-prompt lts
same "CRLF: @@ok, only the Prompt line differs and keeps its CRLF" "@@ok prompt=lts|$(sed 's/^Prompt=normal/Prompt=lts/' "$CORPUS/crlf.cfg" | od -An -c)" \
  "$OUT|$(od -An -c < "$FILE")"
mketc; cp "$CORPUS/header-comment.cfg" "$FILE"
helper "$ETC" set-prompt lts
same "'[DEFAULT] ; main': the key there changes, no second DEFAULT" "@@ok prompt=lts|[DEFAULT] ; main
Prompt=lts" "$OUT|$(cat "$FILE")"
mketc; cp "$CORPUS/key-before-section.cfg" "$FILE"
before=$(sha256sum < "$FILE")
helper "$ETC" set-prompt normal
same "a file Ubuntu ignores (a key before any section): refused, kept" "1 @@fail parse|$before" "$RC $OUT|$(sha256sum < "$FILE")"
check "  and no temp file left" test ! -e "$ETC/update-manager/release-upgrades.win11-gnome.tmp"

# overrides: every .d/*.cfg that sets Prompt in DEFAULT, in byte order; never changed
mketc "$(cat "$DEFAULT")
"
D=$ETC/update-manager/release-upgrades.d
printf '[DEFAULT]\nPrompt=lts\n' > "$D/10-a.cfg"
printf '[DEFAULT]\nOther=1\n' > "$D/20-b.cfg"
printf '[Sources]\nPrompt=never\n' > "$D/30-c.cfg"
printf '[DEFAULT]\n  prompt = never\n' > "$D/40-d.cfg"
printf '[DEFAULT]\nPrompt=never\n' > "$D/50-e.conf"
sums=$(cd "$D" && sha256sum ./*)
helper "$ETC" set-prompt normal
same "overrides are named in order" "@@ok prompt=normal
@@override $D/10-a.cfg
@@override $D/40-d.cfg" "$OUT"
same "overrides are left alone" "$sums" "$(cd "$D" && sha256sum ./*)"

# refusals that keep everything as it was
mketc "$(cat "$DEFAULT")
"
printf 'Prompt=normal\n' > "$T/target"
rm "$FILE"; ln -s "$T/target" "$FILE"
helper "$ETC" set-prompt lts
same "a symlinked file: refused" "1 @@fail symlink" "$RC $OUT"
same "the link target is untouched" "Prompt=normal" "$(cat "$T/target")"
mketc
rm -r "$ETC/update-manager"
helper "$ETC" set-prompt lts
same "no update-manager directory: refused" "1 @@fail nodir" "$RC $OUT"
mketc "$(cat "$DEFAULT")
"
mkdir "$FILE.d-is-not-a-file" && rm "$FILE" && mv "$FILE.d-is-not-a-file" "$FILE"
helper "$ETC" set-prompt lts
same "a directory in the file's place: refused" "1 @@fail notfile" "$RC $OUT"
mketc "$(cat "$DEFAULT")
"
printf 'stale\n' > "$ETC/update-manager/release-upgrades.win11-gnome.tmp"
helper "$ETC" set-prompt lts
same "a stale temp file is replaced" "0 @@ok prompt=lts" "$RC $OUT"
check "  and gone afterwards" test ! -e "$ETC/update-manager/release-upgrades.win11-gnome.tmp"
mketc "$(cat "$DEFAULT")
"
printf 'keep\n' > "$T/victim"
ln -s "$T/victim" "$ETC/update-manager/release-upgrades.win11-gnome.tmp"
helper "$ETC" set-prompt lts
same "a temp-file symlink is removed, never followed" "0 @@ok prompt=lts|keep" "$RC $OUT|$(cat "$T/victim")"

# ------------------------------------------------------------------------------------------- the attach helper

check "attach: sh -n" sh -n "$ATTACH"
check "attach: executable" test -x "$ATTACH"
RC=0; OUT=$(env -i PATH=/usr/bin:/bin "$ATTACH" now 2>&1) || RC=$?
same "attach refuses any argument" "2 @@fail usage" "$RC $OUT"
RC=0; OUT=$(env -i PATH=/usr/bin:/bin "$ATTACH" 2>&1) || RC=$?
same "attach refuses to run as you" "3 @@fail notroot" "$RC $OUT"
same "attach execs exactly Ubuntu's screen session" "exec /usr/bin/screen -d -r ubuntu-release-upgrade-screen-window" \
  "$(grep '^exec ' "$ATTACH")"
for h in "$HELPER" "$ATTACH"; do
  grep -qx 'set -u' "$h" && grep -qx 'PATH=/usr/sbin:/usr/bin:/sbin:/bin' "$h" && grep -qx 'umask 022' "$h" \
    && ok "$(basename "$h"): set -u, fixed PATH, umask 022" || bad "$(basename "$h"): set -u, fixed PATH, umask 022"
  LC_ALL=C grep -q '[^ -~]' "$h" && bad "$(basename "$h"): ASCII only" || ok "$(basename "$h"): ASCII only"
done

# ------------------------------------------------------------------------------------------- the polkit policies

RC=0; OUT=$(python3 -I - "$G/host/data" <<'EOF' 2>&1
import sys, xml.etree.ElementTree as ET
from pathlib import Path
d = Path(sys.argv[1])
NL = "{http://www.w3.org/XML/1998/namespace}lang"
want = {
    "io.github.win11shell.gnome.release-upgrades": dict(
        exec="/usr/libexec/win11-gnome/release-upgrades-helper", defaults=("auth_admin", "auth_admin", "auth_admin_keep"),
        description=("Choose which Ubuntu versions are offered", "Kiezen welke Ubuntu-versies worden aangeboden"),
        message=("Win11 Shell wants to set whether you get only LTS versions or every new version of Ubuntu.",
                 "Win11 Shell wil instellen of je alleen LTS-versies of elke nieuwe versie van Ubuntu krijgt.")),
    "io.github.win11shell.gnome.release-upgrade-attach": dict(
        exec="/usr/libexec/win11-gnome/release-upgrade-attach", defaults=("auth_admin", "auth_admin", "auth_admin"),
        description=("Show the Ubuntu upgrade window again", "Het venster van de Ubuntu-upgrade weer tonen"),
        message=("Win11 Shell wants to show the window of the running Ubuntu upgrade again.",
                 "Win11 Shell wil het venster van de lopende Ubuntu-upgrade weer tonen.")),
}
for action_id, w in want.items():
    path = d / f"{action_id}.policy"
    root = ET.parse(path).getroot()
    assert root.tag == "policyconfig", root.tag
    actions = root.findall("action")
    assert [a.get("id") for a in actions] == [action_id], [a.get("id") for a in actions]
    a = actions[0]
    for tag in ("description", "message"):
        texts = {e.get(NL): e.text for e in a.findall(tag)}
        assert texts == {None: w[tag][0], "nl": w[tag][1]}, (tag, texts)
    got = tuple(a.find("defaults").find(k).text for k in ("allow_any", "allow_inactive", "allow_active"))
    assert got == w["defaults"], got
    notes = {n.get("key"): n.text for n in a.findall("annotate")}
    assert notes == {"org.freedesktop.policykit.exec.path": w["exec"]}, notes
    print("parsed", path.name)
assert "_keep" not in (d / "io.github.win11shell.gnome.release-upgrade-attach.policy").read_text(), "attach must never keep"
print("attach has no _keep")
EOF
) || RC=$?
same "policies parse with xml.etree, texts and defaults as designed, attach without _keep" "0 parsed io.github.win11shell.gnome.release-upgrades.policy
parsed io.github.win11shell.gnome.release-upgrade-attach.policy
attach has no _keep" "$RC $OUT"

# ------------------------------------------------------------------------------------------- install-system.py

PY=$G/scripts/install-system.py
check "install-system.py: py_compile" python3 -I -c 'import ast, sys; ast.parse(open(sys.argv[1]).read())' "$PY"
# the constants and pure functions, imported (its main() does not run on import)
python3 -I - "$PY" "$T" <<'EOF' || bad "install-system.py: import"
import importlib.util, sys
spec = importlib.util.spec_from_file_location("install_system", sys.argv[1])
m = importlib.util.module_from_spec(spec)
sys.dont_write_bytecode = True
spec.loader.exec_module(m)
open(sys.argv[2] + "/install.sh", "w").write(m.INSTALL_SCRIPT)
open(sys.argv[2] + "/remove.sh", "w").write(m.REMOVE_SCRIPT)
cases = [((126, ""), ("noPassword", {"paths": []})), ((127, ""), ("wrongPassword", {"paths": []})),
         ((126, "@@fail checksum\n"), ("failed", {"code": "checksum", "paths": []})),
         ((0, "@@ok /a\n@@ok /b\n@@done\n"), ("ok", {"paths": ["/a", "/b"]})),
         ((0, ""), ("failed", {"code": "rc 0", "paths": []})), ((1, "@@ok /a\n"), ("failed", {"code": "rc 1", "paths": ["/a"]})),
         ((1, "@@ok /a\n@@ok /b\n@@fail install\n"), ("failed", {"code": "install", "paths": ["/a", "/b"]})),
         ((0, "@@done\n"), ("ok", {"paths": []}))]
for args, want in cases:
    got = m.outcome(*args)
    assert got == want, (args, got, want)
for argv, want in [([], (False, False)), (["--dry-run"], (True, False)), (["--remove"], (False, True)),
                   (["--remove", "--dry-run"], (True, True)), (["--dry-run", "--dry-run"], None), (["--lang", "en"], None),
                   (["-n"], None), (["install"], None), (["--dry-run", "x"], None), (["--DRY-RUN"], None)]:
    assert m.parse(argv) == want, (argv, m.parse(argv))
print("ok")
EOF
[ -s "$T/install.sh" ] && ok "install-system.py: outcome() maps pkexec 126/127 and the markers; parse() takes only --dry-run/--remove"
# what main() prints after pkexec (GW5 fix round, m3), in-process: subprocess.run and shutil.which are replaced, so
# nothing is executed; the stage goes to a private 0700 XDG_RUNTIME_DIR under $T and is removed again
mkdir -m 0700 "$T/main-run" && mkdir -p "$T/main-config/win11-gnome"
RC=0; OUT=$(env -i PATH=/usr/bin:/bin HOME="$T" XDG_RUNTIME_DIR="$T/main-run" XDG_CONFIG_HOME="$T/main-config" \
  python3 -I - "$PY" "$T/main-config/win11-gnome/settings.json" <<'EOF' 2>&1
import contextlib, importlib.util, io, subprocess, sys
spec = importlib.util.spec_from_file_location("install_system", sys.argv[1])
m = importlib.util.module_from_spec(spec)
sys.dont_write_bytecode = True
spec.loader.exec_module(m)
calls = []
def fake_run(argv, **kwargs):
    calls.append(argv)
    return subprocess.CompletedProcess(argv, m_rc, stdout=m_out, stderr=None)
m.subprocess.run = fake_run
m.shutil.which = lambda name: "/nonexistent/pkexec-stub"
def main(args, rc, out, lang):
    global m_rc, m_out
    m_rc, m_out = rc, out
    open(sys.argv[2], "w").write('{"language":"%s"}' % lang)
    stdout, stderr = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        code = m.main(args)
    return code, stdout.getvalue().splitlines(), stderr.getvalue().splitlines()
half = "@@ok /usr/libexec/win11-gnome/release-upgrades-helper\n@@ok /usr/libexec/win11-gnome/release-upgrade-attach\n@@fail install\n"
code, out, err = main([], 1, half, "en")
assert code == 1 and out[-2:] == ["Installed: /usr/libexec/win11-gnome/release-upgrades-helper", "Installed: /usr/libexec/win11-gnome/release-upgrade-attach"], out
assert err == ["That didn’t work (install). What was already done is listed above; try again."], err
code, out, err = main([], 1, half, "nl")
assert out[-1] == "Geïnstalleerd: /usr/libexec/win11-gnome/release-upgrade-attach", out
assert err == ["Het is niet gelukt (install). Hierboven staat wat al wel is gebeurd; probeer het opnieuw."], err
code, out, err = main([], 97, "", "en")
assert code == 1 and out[-1] == "Ubuntu now asks for your password once." and err == ["That didn’t work (rc 97). Try again."], (out, err)
code, out, err = main(["--remove"], 1, "@@ok /usr/libexec/win11-gnome/release-upgrades-helper\n@@fail remove\n", "en")
assert out[-1] == "Removed: /usr/libexec/win11-gnome/release-upgrades-helper" and err[-1].startswith("That didn’t work (remove). What was"), (out, err)
code, out, err = main([], 0, "@@ok /a\n@@ok /b\n@@done\n", "nl")
assert code == 0 and out[-3:] == ["Geïnstalleerd: /a", "Geïnstalleerd: /b", "Klaar. Instellingen gebruikt de nieuwe bestanden meteen; je hoeft niet opnieuw aan te melden."], out
assert len(calls) == 5 and all(c[0] == "/nonexistent/pkexec-stub" for c in calls)
print("ok")
EOF
) || RC=$?
same "install-system.py after pkexec: a run that stopped halfway lists what was done, then says so (nl, en)" "0 ok" "$RC $OUT"
check "  the stages were removed" test -z "$(ls -A "$T/main-run")"
INSTALL=$(cat "$T/install.sh") REMOVE=$(cat "$T/remove.sh")
check "install script: sh -n" sh -n "$T/install.sh"
check "remove script: sh -n" sh -n "$T/remove.sh"
NAMES="release-upgrades-helper release-upgrade-attach io.github.win11shell.gnome.release-upgrades.policy io.github.win11shell.gnome.release-upgrade-attach.policy"
stage() {  # stage DIR: the four files as install-system.py stages them
  mkdir -p "$1"
  for n in $NAMES; do cp "$G/host/data/$n" "$1/$n"; done
  chmod 0755 "$1/release-upgrades-helper" "$1/release-upgrade-attach"
  chmod 0644 "$1"/*.policy
}
SUMS=$(for n in $NAMES; do sha256sum "$G/host/data/$n" | cut -d' ' -f1; done | tr '\n' ' ')
# sysrun ROOT SCRIPT NAME ARGS…: the root script as you on a temporary root
sysrun() {
  local root=$1 script=$2 name=$3; shift 3
  RC=0; OUT=$(env -i PATH=/usr/bin:/bin ${root:+WIN11_SYSTEM_ROOT=$root} /bin/sh -c "$script" "$name" "$@" 2>&1) || RC=$?
}
tree() { (cd "$1" && find . -printf '%y %m %p\n' | sort && find . -type f -exec sha256sum {} + | sort); }
stage "$T/stage"
R=$T/sysroot
# shellcheck disable=SC2086
sysrun "$R" "$INSTALL" win11-gnome-system-install "$T/stage" $SUMS
same "install script: markers" "0 @@ok $R/usr/libexec/win11-gnome/release-upgrades-helper
@@ok $R/usr/libexec/win11-gnome/release-upgrade-attach
@@ok $R/usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrades.policy
@@ok $R/usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrade-attach.policy
@@done" "$RC $OUT"
same "install script: the four files, modes 0755/0644" "755 755 644 644" \
  "$(stat -c %a "$R/usr/libexec/win11-gnome/release-upgrades-helper" "$R/usr/libexec/win11-gnome/release-upgrade-attach" \
     "$R/usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrades.policy" \
     "$R/usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrade-attach.policy" | tr '\n' ' ' | sed 's/ $//')"
DESTS="usr/libexec/win11-gnome/release-upgrades-helper usr/libexec/win11-gnome/release-upgrade-attach
usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrades.policy
usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrade-attach.policy"
# shellcheck disable=SC2086
same "install script: contents equal host/data" "$(cd "$G/host/data" && sha256sum $NAMES | cut -d' ' -f1)" \
  "$(cd "$R" && sha256sum $DESTS | cut -d' ' -f1)"
first=$(tree "$R")
# shellcheck disable=SC2086
sysrun "$R" "$INSTALL" win11-gnome-system-install "$T/stage" $SUMS
same "install script: idempotent" "0|$first" "$RC|$(tree "$R")"
same "install script: nothing outside usr/libexec/win11-gnome and usr/share/polkit-1/actions" \
  "./usr/libexec/win11-gnome ./usr/share/polkit-1/actions" \
  "$(cd "$R" && find . -type f -printf '%h\n' | sort -u | tr '\n' ' ' | sed 's/ $//')"
R2=$T/sysroot2
bad_sums="$(printf '%064d' 0) ${SUMS#* }"
# shellcheck disable=SC2086
sysrun "$R2" "$INSTALL" win11-gnome-system-install "$T/stage" $bad_sums
same "install script: a wrong checksum installs nothing" "2 @@fail checksum|" "$RC $OUT|$(ls -A "$R2" 2>/dev/null)"
cp "$T/stage/release-upgrades-helper" "$T/stage/extra"
# shellcheck disable=SC2086
sysrun "$R2" "$INSTALL" win11-gnome-system-install "$T/stage" $SUMS
same "install script: an unknown file in the stage" "2 @@fail unknown" "$RC $OUT"
rm "$T/stage/extra"
# shellcheck disable=SC2086
sysrun "$R2" "$INSTALL" win11-gnome-system-install stage $SUMS
same "install script: a relative stage" "2 @@fail stage" "$RC $OUT"
sysrun "$R2" "$INSTALL" win11-gnome-system-install "$T/stage"
same "install script: the checksums are required" "2 @@fail usage" "$RC $OUT"
# shellcheck disable=SC2086
sysrun "" "$INSTALL" win11-gnome-system-install "$T/stage" $SUMS
same "install script: as you without WIN11_SYSTEM_ROOT: refused" "3 @@fail notroot" "$RC $OUT"
sysrun "$R" "$REMOVE" win11-gnome-system-remove extra
same "remove script: no arguments" "2 @@fail usage" "$RC $OUT"
sysrun "$R" "$REMOVE" win11-gnome-system-remove
same "remove script: exactly the four paths and the empty directory" "0 @@ok $R/usr/libexec/win11-gnome/release-upgrades-helper
@@ok $R/usr/libexec/win11-gnome/release-upgrade-attach
@@ok $R/usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrades.policy
@@ok $R/usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrade-attach.policy
@@ok $R/usr/libexec/win11-gnome
@@done" "$RC $OUT"
same "remove script: the polkit actions directory stays" "d ./usr/share/polkit-1/actions" "$(cd "$R" && find . -path ./usr/share/polkit-1/actions -printf '%y %p\n')"
sysrun "$R" "$REMOVE" win11-gnome-system-remove
same "remove script: again, nothing left to do" "0 @@done" "$RC $OUT"
mkdir -p "$R/usr/libexec/win11-gnome" && touch "$R/usr/libexec/win11-gnome/not-ours"
sysrun "$R" "$REMOVE" win11-gnome-system-remove
same "remove script: another file keeps the directory" "0 @@done|not-ours" "$RC $OUT|$(ls "$R/usr/libexec/win11-gnome")"

# install-system.py itself: only --dry-run and refused arguments, in bubblewrap, with a pkexec stub, under strace
REAL_HOME=${HOME:?}
C=$T/case
mkdir -p "$C/stubs" "$C/home" "$C/config/win11-gnome" "$C/logs"
mkdir -m 0700 "$C/run"
printf '#!/bin/sh\nfor a in "$@"; do printf "%%s\\0" "$a"; done >> %s/logs/pkexec.argv\necho "pkexec stub: blocked" >&2\nexit 97\n' "$C" > "$C/stubs/pkexec"
chmod 0755 "$C/stubs/pkexec"
: > "$C/logs/pkexec.argv"
CASE_N=0
# box ARGS…: run install-system.py ARGS… in the layer (BOX_BW: extra bwrap options); sets OUT ERR RC TRACE
BOX_BW=()
box() {
  CASE_N=$((CASE_N + 1)); TRACE=$C/logs/$CASE_N.strace
  RC=0
  /usr/bin/bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --tmpfs /var/tmp --tmpfs /run/user --tmpfs /run/dbus \
    --tmpfs "$REAL_HOME" --ro-bind "$G" "$G" --bind "$C" "$C" --unshare-all --die-with-parent --new-session \
    --chdir "$C" "${BOX_BW[@]}" -- \
    /usr/bin/env -i PATH="$C/stubs:/usr/bin:/bin" HOME="$C/home" XDG_CONFIG_HOME="$C/config" XDG_RUNTIME_DIR="$C/run" \
      LANG=C.UTF-8 PYTHONDONTWRITEBYTECODE=1 \
      strace -qq -y -e trace=%file -e signal=none -o "$TRACE" /usr/bin/python3 "$PY" "$@" \
    > "$C/logs/out" 2> "$C/logs/err" || RC=$?
  OUT=$(cat "$C/logs/out") ERR=$(cat "$C/logs/err")
}
# writes TRACE ALLOWED-PREFIX: the trace's mutating file calls outside ALLOWED-PREFIX (sandbox.py's parser)
writes() {
  python3 -I - "$1" "$2" "$G/tests/installer" <<'EOF'
import os, sys
sys.path.insert(0, sys.argv[3])
from sandbox import strace_paths
trace, allowed = sys.argv[1], sys.argv[2]
bad, inside = [], 0
for line in open(trace, errors="replace"):
    parsed = strace_paths(line)
    if not parsed:
        continue
    call, paths = parsed
    for p in paths:
        if allowed and (p == allowed or p.startswith(allowed)):
            inside += 1
        elif not p.startswith(("/dev/null", "/dev/tty", "/proc/self/")):
            bad.append(f"{call} {p}")
print(f"inside={inside} outside={len(bad)}")
for b in bad[:20]:
    print("  " + b)
EOF
}
box --dry-run
same "dry run: rc 0, nothing on stderr" "0|" "$RC|$ERR"
same "dry run: Dutch without a settings.json (C1)" "Win11 installeert vier systeembestanden. Daarmee zegt Ubuntu bij ‘Nieuwe Ubuntu-versies’ en ‘Venster weer openen’ waarvoor je wachtwoord nodig is:" "$(printf '%s\n' "$OUT" | head -1)"
python3 -I - "$OUT" "$C" "$T/install.sh" "$SUMS" <<'EOF' && ok "dry run prints the exact pkexec argv (the stub on PATH, /bin/sh -c <INSTALL_SCRIPT>, the stage, the checksums)" || bad "dry run argv"
import shlex, sys, os, re
out, case, script, sums = sys.argv[1], sys.argv[2], open(sys.argv[3]).read(), sys.argv[4].split()
lines = out.split("\n")
start = lines.index("Opdracht:") + 1
end = lines.index("Controlegetallen (sha256):")
argv = shlex.split("\n".join(lines[start:end]))
assert argv[:5] == [f"{case}/stubs/pkexec", "/bin/sh", "-c", script, "win11-gnome-system-install"], argv[:2]
assert re.fullmatch(re.escape(case) + r"/run/win11-gnome-system-[^/]+", argv[5]), argv[5]
assert not os.path.exists(argv[5]), "the stage was not removed"
assert argv[6:] == sums, (argv[6:], sums)
dest = ["/usr/libexec/win11-gnome/release-upgrades-helper", "/usr/libexec/win11-gnome/release-upgrade-attach",
        "/usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrades.policy",
        "/usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrade-attach.policy"]
assert lines[end + 1:end + 5] == [f"{s}  {d}" for s, d in zip(sums, dest)], lines[end + 1:]
assert lines[1:5] == [f"  {d}" for d in dest], lines[1:5]
EOF
same "dry run: pkexec was not run" "" "$(tr '\0' ' ' < "$C/logs/pkexec.argv")"
w=$(writes "$TRACE" "$C/run/win11-gnome-system-" | head -1)
[[ $w =~ ^inside=[1-9][0-9]*\ outside=0$ ]] && ok "dry run: writes only inside its stage under XDG_RUNTIME_DIR ($w)" \
  || bad "dry run: writes only inside its stage" "$(writes "$TRACE" "$C/run/win11-gnome-system-")"
check "dry run: the stage dir is gone" test -z "$(ls -A "$C/run")"
printf '{"language":"en"}\n' > "$C/config/win11-gnome/settings.json"
for args in "--remove --dry-run" "--dry-run --remove"; do
  # shellcheck disable=SC2086
  box $args
  same "$args: rc 0" "0|" "$RC|$ERR"
  python3 -I - "$OUT" "$C" "$T/remove.sh" <<'EOF' && ok "$args: argv pkexec /bin/sh -c <REMOVE_SCRIPT> win11-gnome-system-remove" || bad "$args argv"
import shlex, sys
out, case, script = sys.argv[1], sys.argv[2], open(sys.argv[3]).read()
lines = out.split("\n")
assert lines[0] == "Win11 removes its four system files:", lines[0]
start = lines.index("Command:") + 1
assert shlex.split("\n".join(lines[start:])) == [f"{case}/stubs/pkexec", "/bin/sh", "-c", script, "win11-gnome-system-remove"]
EOF
  same "$args: no Dutch in English" "" "$(printf '%s\n' "$OUT" | sed -n '/^Command:/q;p' | tr 'A-Z' 'a-z' | grep -owFf <(grep -v '^#' "$G/tests/i18n/dutch-words.txt" | grep .) | sort -u | tr '\n' ' ')"
  same "$args: no write at all" "inside=0 outside=0" "$(writes "$TRACE" "" | head -1)"
done
rm "$C/config/win11-gnome/settings.json"
for args in "--bogus" "--lang en" "--dry-run --dry-run" "install" "--dry-run x" "-n" "--remove --remove" "--DRY-RUN"; do
  # shellcheck disable=SC2086
  box $args
  same "refuses '$args': rc 2, usage on stderr, nothing on stdout" "2|Gebruik: python3 scripts/install-system.py [--dry-run] [--remove]|" "$RC|$ERR|$OUT"
  same "refuses '$args': no write at all" "inside=0 outside=0" "$(writes "$TRACE" "" | head -1)"
done
printf '{"language":"en"}\n' > "$C/config/win11-gnome/settings.json"
box --bogus
same "the usage line follows Win11's language" "2|Usage: python3 scripts/install-system.py [--dry-run] [--remove]" "$RC|$ERR"
BOX_BW=(--unshare-user --uid 0 --gid 0)
box --dry-run
BOX_BW=()
same "as root (a user namespace mapped to 0): refused" "1|Don’t run this as root or with sudo: pkexec asks for your password itself.|" "$RC|$ERR|$OUT"
same "  and no write at all" "inside=0 outside=0" "$(writes "$TRACE" "" | head -1)"
chmod 0755 "$C/run"
box --dry-run
same "a shared XDG_RUNTIME_DIR: refused" "1|There’s no private XDG_RUNTIME_DIR (a folder of yours with mode 0700), so nothing was done." "$RC|$ERR"
same "  and no write at all" "inside=0 outside=0" "$(writes "$TRACE" "" | head -1)"
chmod 0700 "$C/run"
same "pkexec was never run" "" "$(tr '\0' ' ' < "$C/logs/pkexec.argv")"

# ------------------------------------------------------------------------------------------- the installer hint

python3 -I - "$G/scripts" "$T/hint" <<'EOF' && ok "install.py/uninstall.py hint: missing → install line, present → remove line, nl and en" || bad "installer hint"
import sys
from pathlib import Path
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
import install
root = Path(sys.argv[2])
script = Path(sys.argv[1]).resolve() / "install-system.py"
install.set_language("nl")
assert install.system_hint(False, root) == f"Optioneel, één keer met je wachtwoord: python3 {script} installeert de systeembestanden voor ‘Nieuwe Ubuntu-versies’ en ‘Venster weer openen’."
assert install.system_hint(True, root) is None
install.set_language("en")
assert install.system_hint(False, root) == f"Optional, once with your password: python3 {script} installs the system files for ‘New Ubuntu versions’ and ‘Show the upgrade window’."
(root / install.SYSTEM_FILES[0]).parent.mkdir(parents=True)
(root / install.SYSTEM_FILES[0]).write_text("x")
assert install.system_hint(False, root) is not None, "one of four is still missing"
assert install.system_hint(True, root) == f"Win11’s system files are still installed; remove them with: python3 {script} --remove"
for name in install.SYSTEM_FILES:
    (root / name).parent.mkdir(parents=True, exist_ok=True)
    (root / name).write_text("x")
assert install.system_hint(False, root) is None
install.set_language("nl")
assert install.system_hint(True, root) == f"De systeembestanden van Win11 staan er nog; verwijder ze met: python3 {script} --remove"
EOF
grep -q 'hint and sys.stdout.isatty()' "$G/scripts/install.py" && grep -q 'hint and sys.stdout.isatty()' "$G/scripts/uninstall.py" \
  && ok "the hint is printed on a terminal only (captured output stays 3b898e6's)" || bad "hint TTY rule"

printf '\ntest_helpers.sh: %d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" = 0 ]
