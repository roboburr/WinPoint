#!/usr/bin/env python3
"""Bundle source + prebuilt standalone apps, excluding build caches and private configuration."""
from pathlib import Path
import hashlib
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import install
from install import BINS
import icon_bundle

# Messages: --lang nl|en, else WIN11_LANG, else nl (a developer script; it reads no user settings).
root = Path(sys.argv[1]).resolve()
language = os.environ.get("WIN11_LANG", "nl")
for index, argument in enumerate(sys.argv[2:], 2):
    if argument.startswith("--lang="):
        language = argument.split("=", 1)[1]
    elif argument == "--lang" and index + 1 < len(sys.argv):
        language = sys.argv[index + 1]
install.set_language(language.strip().lower())
subprocess.run([sys.executable, str(root/"scripts/verify-ui.py")], check=True,
               env=dict(os.environ, WIN11_LANG=install.LANGUAGE))
icon_bundle.validate(root/"vendor")
build = Path(os.environ.get("WIN11_BUILD_DIR", str(root/"target/release")))
# all 12, the native shell and Calendar included (WIN11_NATIVE_BUILD_DIR first for those two, as install.py does)
missing = install.missing_binaries(build)
if missing:
    raise SystemExit("\n".join(install.t("missingBinary", source=path) for path in missing))
version = tomllib.loads((root/"host/Cargo.toml").read_text())["package"]["version"]
destination = Path(os.environ.get("WIN11_PACKAGE_DIR", str(root/"dist"))).resolve()
destination.mkdir(parents=True, exist_ok=True)
archive = destination/f"win11-gnome-{version}-linux-{platform.machine()}.tar.gz"
with tempfile.TemporaryDirectory(prefix="win11-gnome-package-") as temp:
    stage = Path(temp)/"win11-gnome"
    stage.mkdir()
    # native/: the GTK 4 shell and Calendar sources (their own workspace), next to host/
    for name in ("scripts", "extension", "host", "native", "vendor"):
        shutil.copytree(root/name, stage/name, symlinks=True,
                        ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "target", ".git"))
    # GW5: host/data holds the four system files of scripts/install-system.py (notes/claude/windows-update-gnome.md §7);
    # the tarball must carry all of them, the two helpers executable.
    for rel in install.SYSTEM_FILES:
        shipped = stage/"host/data"/Path(rel).name
        if not shipped.is_file() or shipped.is_symlink():
            raise SystemExit(install.t("badTable", file=root/"host/data"/Path(rel).name))
        shipped.chmod(0o755 if rel.startswith("usr/libexec/") else 0o644)
    # LICENSE, THIRD_PARTY_NOTICES.md and LICENSES/ travel with the binaries (the notices of the linked crates).
    # README.md is the English README, README.nl.md the Dutch one (2026-11-13; README.en.md is gone).
    for name in ("Cargo.toml", "Cargo.lock", "LICENSE", "THIRD_PARTY_NOTICES.md", "README.md", "README.nl.md", "RELEASE_PLAN.md"):
        shutil.copy2(root/name, stage/name)
    # The screenshots README.md shows (docs/screenshots/*.png, own captures; the icons in them stay GPL-3.0).
    if (root/"docs/screenshots").is_dir():
        shutil.copytree(root/"docs/screenshots", stage/"docs/screenshots")
    shutil.copytree(root/"LICENSES", stage/"LICENSES")
    # The read-only inventory of an older Win11 Shell for Hyprland install, for machines without a checkout.
    (stage/"tools").mkdir()
    shutil.copy2(root/"tools/h-remnants.sh", stage/"tools/h-remnants.sh")
    # Include only an existing report; this script never invents a passing test result.
    for report in ("VALIDATION.md", "VALIDATION_TABS.md", "VALIDATION_FILES.md", "VALIDATION_FILES_COMPLETE.md", "VALIDATION_SETTINGS.md", "VALIDATION_FILES_DESKTOP.md", "VALIDATION_SEARCH.md", "VALIDATION_WATCH.md", "VALIDATION_ACTIONS.md", "VALIDATION_REGRESSIONS_0.5.md", "VALIDATION_APPS.md", "VALIDATION_APPS_CATALOG.md", "VALIDATION_APPS_DEFAULTS.md", "VALIDATION_APPS_RECENT.md", "VALIDATION_TOPBAR.md", "VALIDATION_ALLES_PROGRESS.md", "VALIDATION_TRAY.md", "VALIDATION_APP_ACTIVATION.md", "VALIDATION_SHORTCUTS.md", "VALIDATION_DESKTOP.md", "VALIDATION_WALLPAPER.md", "VALIDATION_INDEX_LIFECYCLE.md", "VALIDATION_PROCESS_MEMORY.md", "PACKAGING_VALIDATION.md"):
        if (root/report).is_file():
            shutil.copy2(root/report, stage/report)
    if (root/"validation").is_dir():
        shutil.copytree(root/"validation", stage/"validation")
    for preview in ("preview-gnome.png", "preview-explorer.png", "preview-settings.png", "preview-apps-catalog.png", "logo-circle.png"):
        if (root/preview).is_file():
            shutil.copy2(root/preview, stage/preview)
    (stage/"bin").mkdir()
    for name in BINS:
        shutil.copy2(install.binary_source(build, name), stage/"bin"/name)
        (stage/"bin"/name).chmod(0o755)
    with tarfile.open(archive, "w:gz") as bundle:
        bundle.add(stage, arcname=stage.name)
checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
archive.with_suffix(archive.suffix+".sha256").write_text(f"{checksum}  {archive.name}\n")
print(archive)
