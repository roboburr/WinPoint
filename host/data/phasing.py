# Win11 Shell, Windows Update: the read-only phasing classifier (notes/claude/windows-update-gnome.md 4.6).
# host/src/services/update/phasing.rs embeds this file (include_str!) and runs it as `python3 -I -c <text>`;
# tools/update/probe.py --phasing runs the same bytes, so R1 measures what ships. Input: name<TAB>arch per line.
# Output per input line: name<TAB>arch<TAB>candidate|-<TAB>phasing_applied 0/1<TAB>Phased-Update-Percentage|-
# <TAB>held 0/1<TAB>installed|-, then @@end; exit 3 with @@no-apt-pkg without python3-apt. No lock, no write.
import sys
try:
    import apt_pkg
except Exception:
    print("@@no-apt-pkg")
    sys.exit(3)
apt_pkg.init()
cache = apt_pkg.Cache(None)
dep = apt_pkg.DepCache(cache)
records = apt_pkg.PackageRecords(cache)
for line in sys.stdin:
    parts = line.rstrip("\n").split("\t")
    if len(parts) != 2:
        continue
    name, arch = parts
    try:
        pkg = cache[(name, arch)]
    except Exception:
        print("\t".join([name, arch, "-", "0", "-", "0", "-"]))
        continue
    cand = dep.get_candidate_ver(pkg)
    pup = "-"
    if cand is not None and cand.file_list:
        records.lookup(cand.file_list[0])
        value = apt_pkg.TagSection(records.record).get("Phased-Update-Percentage")
        if value is not None and value.strip().isdigit():
            pup = value.strip()
    phased = 1 if dep.phasing_applied(pkg) else 0
    held = 1 if pkg.selected_state == apt_pkg.SELSTATE_HOLD else 0
    current = pkg.current_ver.ver_str if pkg.current_ver is not None else "-"
    print("\t".join([name, arch, cand.ver_str if cand is not None else "-", str(phased), pup, str(held), current]))
print("@@end")
