# Phasing fixtures (D-G73)

`recorded/` holds the real answers of one machine at one moment (2026-11-14: Ubuntu 26.04.1, apt 3.2.1,
python3-apt 3.1.0), read by `phasing_matches_the_python3_apt_script` in `host/src/services/update/phasing.rs`:

| file | what printed it (`env -i LANG=C.UTF-8 PATH=/usr/bin:/bin`) |
|---|---|
| `input.tsv` | the 19 `name<TAB>arch` pairs asked for: Ubuntu packages only, eight of them held back by phasing, three with a percentage this machine already passed (python3-software-properties, software-properties-common, snapd), `libc6` for `i386` and `amd64`, `all` packages, `hello` (not installed) and `zzz-nonexistent` |
| `policy.txt` | `apt-cache policy <name:arch>…` for those pairs (the service adds `-o APT::Cmd::Pattern-Only=true`, which prints the same for names apt knows) |
| `phasing.txt` | `apt-cache policy ?phasing` |
| `showhold.txt` | `apt-mark showhold`, trimmed to the requested packages (none of them is held) |
| `phasing-py.txt` | the python3-apt script the classifier replaced (`git show 6ebf169f:host/data/phasing.py`) on `input.tsv` |

The mirror `nl.archive.ubuntu.com` is written as `archive.ubuntu.com`, and `showhold.txt` keeps only the requested
packages; nothing else is changed.
