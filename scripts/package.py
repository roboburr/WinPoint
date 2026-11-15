#!/usr/bin/env python3
"""Bundle source + prebuilt standalone apps, excluding build caches and private configuration.

The release asset (D-G74a, frozen from 0.9.0 on: WinPoint's update check looks for these names):
dist/winpoint-<version>-linux-<arch>.tar.gz with one top folder winpoint-<version>/, and next to it the .sha256
"<64 hex>  <archive name>\n". The version is the root Cargo.toml's [workspace.package] version, the one source;
extension/metadata.json's version-name must equal it, and the build's win11-panel must carry its marker
"WinPoint <version>\n" (host/src/lib.rs VERSION_LINE), so a stale build is never packed under a new name."""
from pathlib import Path
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
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
# vendor/ui against vendor/ui-sha256.json: the wallpapers and icons install.py ships (its HTML apps are history,
# git show 6ebf169f:vendor/ui); tools/ui-manifest.py records a change on purpose.
subprocess.run([sys.executable, str(root/"scripts/verify-ui.py")], check=True,
               env=dict(os.environ, WIN11_LANG=install.LANGUAGE))
icon_bundle.validate(root/"vendor")
build = Path(os.environ.get("WIN11_BUILD_DIR", str(root/"target/release")))
# all 19, the native shell, Calendar and apps included (WIN11_NATIVE_BUILD_DIR first for those, as install.py does)
missing = install.missing_binaries(build)
if missing:
    raise SystemExit("\n".join(install.t("missingBinary", source=path) for path in missing))
# D-G74a: one version, the root [workspace.package]; metadata.json and the build must agree with it
version = install.bundle_version(root)
if not version:
    raise SystemExit(install.t("badTable", file=root/"Cargo.toml"))
try:
    version_name = json.loads((root/"extension/metadata.json").read_text(encoding="utf-8")).get("version-name")
except (OSError, UnicodeDecodeError, ValueError, AttributeError):
    raise SystemExit(install.t("badTable", file=root/"extension/metadata.json"))
if version_name != version:
    raise SystemExit(install.t("packageVersion", file=root/"extension/metadata.json", found=version_name, version=version))
panel = install.binary_source(build, "win11-panel")
if f"WinPoint {version}\n".encode() not in panel.read_bytes():
    raise SystemExit(install.t("packageMarker", binary=panel, version=version))
destination = Path(os.environ.get("WIN11_PACKAGE_DIR", str(root/"dist"))).resolve()
destination.mkdir(parents=True, exist_ok=True)
archive = destination/f"winpoint-{version}-linux-{platform.machine()}.tar.gz"
with tempfile.TemporaryDirectory(prefix="winpoint-package-") as temp:
    stage = Path(temp)/f"winpoint-{version}"
    stage.mkdir()
    # native/: the GTK 4 shell, Calendar, kit and app sources (their own workspace), next to host/
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
    # What changed per release (tools/release.py takes the release notes from it).
    if (root/"CHANGELOG.md").is_file():
        shutil.copy2(root/"CHANGELOG.md", stage/"CHANGELOG.md")
    # The screenshots README.md shows (docs/screenshots/*.png, own captures; the icons in them stay GPL-3.0).
    if (root/"docs/screenshots").is_dir():
        shutil.copytree(root/"docs/screenshots", stage/"docs/screenshots")
    shutil.copytree(root/"LICENSES", stage/"LICENSES")
    # The read-only inventory of an older Win11 Shell for Hyprland install, for machines without a checkout.
    (stage/"tools").mkdir()
    shutil.copy2(root/"tools/h-remnants.sh", stage/"tools/h-remnants.sh")
    # Include only an existing report; this script never invents a passing test result. Most are history (the WebKit
    # apps up to commit 6ebf169f): they ship as the record they are, not as a claim about this version.
    for report in ("VALIDATION.md", "VALIDATION_TABS.md", "VALIDATION_FILES.md", "VALIDATION_FILES_COMPLETE.md", "VALIDATION_SETTINGS.md", "VALIDATION_FILES_DESKTOP.md", "VALIDATION_SEARCH.md", "VALIDATION_WATCH.md", "VALIDATION_ACTIONS.md", "VALIDATION_REGRESSIONS_0.5.md", "VALIDATION_APPS.md", "VALIDATION_APPS_CATALOG.md", "VALIDATION_APPS_DEFAULTS.md", "VALIDATION_APPS_RECENT.md", "VALIDATION_TOPBAR.md", "VALIDATION_ALLES_PROGRESS.md", "VALIDATION_TRAY.md", "VALIDATION_APP_ACTIVATION.md", "VALIDATION_SHORTCUTS.md", "VALIDATION_DESKTOP.md", "VALIDATION_WALLPAPER.md", "VALIDATION_INDEX_LIFECYCLE.md", "VALIDATION_PROCESS_MEMORY.md", "PACKAGING_VALIDATION.md"):
        if (root/report).is_file():
            shutil.copy2(root/report, stage/report)
    if (root/"validation").is_dir():
        shutil.copytree(root/"validation", stage/"validation")
    # The logo README.md shows; the preview-*.png captures show the WebKit apps and stay out.
    if (root/"logo-circle.png").is_file():
        shutil.copy2(root/"logo-circle.png", stage/"logo-circle.png")
    (stage/"bin").mkdir()
    for name in BINS:
        shutil.copy2(install.binary_source(build, name), stage/"bin"/name)
        (stage/"bin"/name).chmod(0o755)
    with tarfile.open(archive, "w:gz") as bundle:
        bundle.add(stage, arcname=stage.name)
checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
archive.with_suffix(archive.suffix+".sha256").write_text(f"{checksum}  {archive.name}\n")
print(archive)
