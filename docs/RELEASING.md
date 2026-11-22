# Publishing WinPoint

Every WinPoint release includes the GitHub release, both repository READMEs and **winpoint.nl**. Update the website
as part of the same release, without waiting for a separate request. A release is complete only after its public
download, checksum, version labels, release notes and install command agree with GitHub's latest stable release.

1. Run `python3 -B tools/release.py bump X.Y.Z`, move the completed Unreleased notes into `## X.Y.Z`, and update
   the current-release descriptions in README.md and README.nl.md. Preserve unrelated local files. Commit the
   reviewed release source on main before packaging.
2. Build the host and native workspaces offline with the machine-wide heavy-task guard and the external build
   targets. Stage all nineteen programs from those targets; do not use an older shell or host binary. Run the
   relevant isolated regressions and the release version checks. Leave the user's active session untouched.
3. Use `tools/release.py build X.Y.Z --build-dir DIR --host-target HOST/release --native-target NATIVE/release`
   against the release commit. Verify the archive, SHA-256, all binary identities and the installer's
   `systemFiles` compatibility result. Changes to system files require the manual-install route in the notes.
4. Push main and the annotated version tag. Upload both assets to a draft release; verify GitHub's asset sizes
   and SHA-256 digests before publishing it as the latest stable release. Verify the actual updater sees it.
5. Immediately update the existing winpoint.nl source at `/run/media/robin/22TB/VPS/winpoint`: version labels,
   JSON-LD, download/checksum/release-note links, exact unpack command, size and release summary. Build the static
   site. Back up the remote index, verify its expected old hash, and atomically replace it with the reviewed build.
   Caddy serves the new static page without a restart. Verify the public HTTPS page and download links.

Publishing does not install or restart WinPoint in the user's session. If the extension changed, tell users that
they will need to sign out and back in after installing to load it; do not sign them out automatically.
