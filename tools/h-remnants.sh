#!/usr/bin/env bash
# What Win11 Shell for Hyprland (the older "H" install) left on this computer. READ ONLY: it prints, it never
# changes, moves or removes anything, and it starts nothing (plan-critique F20; gnome-map §6.10).
#
#   tools/h-remnants.sh [--lang nl|en]
#
# Language: --lang, else WIN11_LANG, else Win11's own setting (~/.config/win11-gnome/settings.json, read only),
# else Dutch. Exit 0 when nothing of H is left, 1 when something is (the list says what), 2 on a bad option.
# Paths follow H's install.sh: XDG_CONFIG_HOME (~/.config), XDG_DATA_HOME (~/.local/share), ~/.local/bin,
# ~/.themes, /usr/share/wayland-sessions and /usr/local/bin. W11_REMNANTS_SYSROOT prefixes the two system paths
# (tests only).
set -uo pipefail
export LC_ALL=C.UTF-8

CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
SHARE="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN="$HOME/.local/bin"
SYSROOT="${W11_REMNANTS_SYSROOT:-}"
CONFIG="$CONFIG_HOME/win11shell"
APPDIR="$SHARE/win11shell"

MSG_LANG="${WIN11_LANG:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --lang) MSG_LANG="${2:-}"; shift 2 || shift ;;
    --lang=*) MSG_LANG="${1#--lang=}"; shift ;;
    -h|--help) MSG_LANG="${MSG_LANG:-nl}"; HELP=1; shift ;;
    *) BAD="$1"; shift ;;
  esac
done
if [ -z "$MSG_LANG" ] && [ -r "$CONFIG_HOME/win11-gnome/settings.json" ] && command -v python3 >/dev/null; then
  MSG_LANG="$(python3 -I -c 'import json,sys
try: d=json.load(open(sys.argv[1], encoding="utf-8"))
except Exception: d={}
print("en" if isinstance(d, dict) and d.get("language") == "en" else "nl")' "$CONFIG_HOME/win11-gnome/settings.json" 2>/dev/null)"
fi
MSG_LANG="$(printf '%s' "${MSG_LANG:-nl}" | tr 'A-Z' 'a-z')"

# i18n: pairs (tools/i18n/check_pairs.py): pair NL EN [ARG…] prints the printf format of the chosen language.
pair() { local nl="$1" en="$2"; shift 2; if [ "$MSG_LANG" = en ]; then printf -- "$en\n" "$@"; else printf -- "$nl\n" "$@"; fi; }

case "$MSG_LANG" in
  nl|en) ;;
  *) bad_lang="$MSG_LANG"; MSG_LANG=nl
     pair 'Onbekende taal: %s (kies nl of en).' 'Unknown language: %s (choose nl or en).' "$bad_lang" >&2; exit 2 ;;
esac
if [ -n "${BAD:-}" ]; then
  pair 'Onbekende optie: %s' 'Unknown option: %s' "$BAD" >&2
  pair 'Gebruik: tools/h-remnants.sh [--lang nl|en]' 'Usage: tools/h-remnants.sh [--lang nl|en]' >&2
  exit 2
fi
if [ -n "${HELP:-}" ]; then
  pair 'Gebruik: tools/h-remnants.sh [--lang nl|en]' 'Usage: tools/h-remnants.sh [--lang nl|en]'
  pair 'Laat zien wat Win11 Shell voor Hyprland op deze computer achterliet. Er verandert niets.' \
       'Shows what Win11 Shell for Hyprland left on this computer. Nothing is changed.'
  exit 0
fi

FOUND=0
tilde() { case "$1" in "$HOME"/*) printf '~/%s' "${1#"$HOME"/}" ;; *) printf '%s' "$1" ;; esac; }
item() { FOUND=$((FOUND + 1)); printf '  • %s\n' "$*"; }
none() { pair '  (niets)' '  (nothing)'; }
section() { printf '\n'; pair "$@"; }
files() { if [ "$1" = 1 ]; then pair '1 bestand' '1 file'; else pair '%s bestanden' '%s files' "$1"; fi; }
lines() { if [ "$1" = 1 ]; then pair '1 regel' '1 line'; else pair '%s regels' '%s lines' "$1"; fi; }

pair 'Wat Win11 Shell voor Hyprland (H) op deze computer achterliet. Alleen lezen: er verandert niets.' \
     'What Win11 Shell for Hyprland (H) left on this computer. Read only: nothing is changed.'

section '1. Programma’s in %s' '1. Programs in %s' "$(tilde "$BIN")"
n=0
for name in win11shell win11shell-ctl win11shell-session; do
  f="$BIN/$name"
  if [ -L "$f" ]; then item "$(tilde "$f") → $(readlink -- "$f")"; n=$((n + 1))
  elif [ -e "$f" ]; then item "$(tilde "$f")"; n=$((n + 1)); fi
done
[ "$n" -gt 0 ] || none

section '2. Programmamap %s' '2. Program folder %s' "$(tilde "$APPDIR")"
if [ -d "$APPDIR" ]; then
  size="$(du -sh -- "$APPDIR" 2>/dev/null | cut -f1)"
  files="$(find "$APPDIR" -type f 2>/dev/null | wc -l)"
  item "$(tilde "$APPDIR"), $(files "$files"), ${size:-?}"
  [ -d "$APPDIR/ui" ] && item "$(pair 'met een eigen interface: %s' 'with its own interface: %s' "$(tilde "$APPDIR/ui")")"
else
  none
fi

section '3. Instellingen %s' '3. Settings %s' "$(tilde "$CONFIG")"
n=$FOUND
if [ -d "$CONFIG" ]; then
  if [ -f "$CONFIG/settings.json" ]; then
    lang="$(python3 -I -c 'import json,sys
try: d=json.load(open(sys.argv[1], encoding="utf-8"))
except Exception: d=None
print("?" if not isinstance(d, dict) else d.get("language", "-") if isinstance(d.get("language", "-"), str) else "?")' "$CONFIG/settings.json" 2>/dev/null || printf '?')"
    item "$(pair '%s (taal: %s)' '%s (language: %s)' "$(tilde "$CONFIG/settings.json")" "$lang")"
  fi
  for sub in installer hypr; do
    [ -d "$CONFIG/$sub" ] && item "$(tilde "$CONFIG/$sub"): $(files "$(find "$CONFIG/$sub" -type f 2>/dev/null | wc -l)")"
  done
  [ -f "$CONFIG/hyprland-monitors.conf" ] && item "$(tilde "$CONFIG/hyprland-monitors.conf")"
  [ "$FOUND" -gt "$n" ] || item "$(tilde "$CONFIG")"
else
  none
fi

section '4. Snelkoppelingen in het app-overzicht (%s)' '4. Shortcuts in the app overview (%s)' "$(tilde "$SHARE/applications")"
n=0
for f in "$SHARE"/applications/win11shell-*.desktop; do
  [ -e "$f" ] || continue
  title="$(sed -n 's/^Name=//p' -- "$f" | head -n 1)"
  item "$(basename -- "$f") (Name=${title:-?})"; n=$((n + 1))
done
[ "$n" -gt 0 ] || none

section '5. Standaard-apps die naar H wijzen (mimeapps.list)' '5. Default apps that point to H (mimeapps.list)'
n=0
for f in "$CONFIG_HOME/mimeapps.list" "$CONFIG_HOME"/*-mimeapps.list "$SHARE/applications/mimeapps.list"; do
  [ -f "$f" ] || continue
  while IFS= read -r line; do
    item "$(tilde "$f"): $line"; n=$((n + 1))
  done < <(grep -n 'win11shell-' -- "$f" 2>/dev/null)
done
[ "$n" -gt 0 ] || none

section '6. Sessie op het inlogscherm' '6. Session on the sign-in screen'
n=0
for f in "$SYSROOT/usr/share/wayland-sessions/win11shell.desktop" "$SYSROOT/usr/share/wayland-sessions/win11shell.desktop.uit" \
         "$SYSROOT/usr/share/xsessions/win11shell.desktop" "$SYSROOT/usr/local/bin/win11shell-session"; do
  [ -e "$f" ] && { item "${f#"$SYSROOT"}"; n=$((n + 1)); }
done
[ "$n" -gt 0 ] || none

section '7. systemd-gebruikersbestanden (%s)' '7. systemd user files (%s)' "$(tilde "$CONFIG_HOME/systemd/user")"
n=0
for f in "$CONFIG_HOME/systemd/user/win11shell-session.target" \
         "$CONFIG_HOME/systemd/user/hyprpolkitagent.service.d/win11shell-not-in-gnome.conf" \
         "$CONFIG_HOME/systemd/user/hypridle.service.d/win11shell-not-in-gnome.conf"; do
  if [ -L "$f" ]; then item "$(tilde "$f") → $(readlink -- "$f")"; n=$((n + 1))
  elif [ -e "$f" ]; then item "$(tilde "$f")"; n=$((n + 1)); fi
done
for f in "$CONFIG_HOME"/systemd/user/*.wants/win11shell*; do
  [ -e "$f" ] || [ -L "$f" ] || continue
  item "$(tilde "$f")"; n=$((n + 1))
done
[ "$n" -gt 0 ] || none

section '8. Thema’s en pictogrammen die H installeerde' '8. Themes and icons that H installed'
n=0
if [ -r "$CONFIG/installer/themes" ]; then
  while IFS=$'\t' read -r name rec; do
    case "$name" in ""|*/*|.*) continue ;; esac
    kind="${rec##* }"
    if [ "$kind" = icons ]; then dest="$SHARE/icons/$name"; else dest="$HOME/.themes/$name"; fi
    if [ -d "$dest" ]; then item "$(tilde "$dest")"; n=$((n + 1)); fi
  done < "$CONFIG/installer/themes"
fi
[ "$n" -gt 0 ] || none

section '9. Blokken in GTK-stijlbestanden' '9. Blocks in GTK style files'
n=0
for f in "$CONFIG_HOME/gtk-4.0/gtk.css" "$CONFIG_HOME/gtk-3.0/gtk.css"; do
  [ -f "$f" ] || continue
  c="$(grep -c -i 'win11 \?shell' -- "$f" 2>/dev/null)"
  [ "${c:-0}" -gt 0 ] && { item "$(tilde "$f"): $(lines "$c")"; n=$((n + 1)); }
done
[ "$n" -gt 0 ] || none

section '10. Lopende H-processen' '10. Running H processes'
n=0
if command -v pgrep >/dev/null; then
  while IFS= read -r line; do
    [ -n "$line" ] && { item "$line"; n=$((n + 1)); }
  done < <(pgrep -a -f -- 'win11shell/host|win11shell-session|bin/win11shell(-ctl)?( |$)' 2>/dev/null | grep -v -- 'h-remnants' | cut -c1-160)
fi
[ "$n" -gt 0 ] || none

printf '\n'
if [ "$FOUND" -eq 0 ]; then
  pair 'Niets van H gevonden.' 'Nothing of H was found.'
  exit 0
fi
pair 'Gevonden: %s onderdelen van H. Niets is veranderd; opruimen is een aparte keuze.' \
     'Found: %s parts of H. Nothing was changed; cleaning up is a separate decision.' "$FOUND"
exit 1
