//! The GSettings keys Settings may read and write (notes/claude/settings-parity.md §6.1): one table per batch, each
//! batch edits only its own file. Every (schema, path pattern, key) is in exactly one table (a test checks it).
//!
//! Never here: ubuntu-dock (org.gnome.shell.extensions.dash-to-dock), DING (org.gnome.shell.extensions.ding),
//! tiling-assistant, `org.gnome.shell` enabled-extensions / disabled-extensions, and mutter experimental-features in a
//! writing mode ([`FORBIDDEN_SCHEMAS`], [`FORBIDDEN_KEYS`]; tests/i18n and the B0 audit check both).
mod accounts;
mod apps;
mod devices;
mod input;
mod network;
mod system;

/// What Settings may do with a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// `gsettings.set` and `gsettings.reset` write it.
    Write,
    /// Only read here.
    Read,
    /// Read here; written only by this host method (it does more than one key, or more than GSettings).
    Service(&'static str),
    /// WinPoint's look holds it (gnome_look::note); written through this WinPoint setting.
    Owned(&'static str),
    /// Shared with this WinPoint setting through the mirror (settings_mirror.rs): Settings writes the GNOME key,
    /// the mirror keeps the setting equal.
    Mirrored(&'static str),
}

impl Mode {
    /// The API's `mode` name.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Mode::Write => "write",
            Mode::Read => "read",
            Mode::Service(_) => "service",
            Mode::Owned(_) => "owned",
            Mode::Mirrored(_) => "mirrored",
        }
    }

    /// Whether `gsettings.set` writes it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn writes(self) -> bool {
        matches!(self, Mode::Write | Mode::Mirrored(_))
    }
}

/// One row of an allow table.
#[derive(Debug)]
pub(crate) struct Allow {
    pub schema: &'static str,
    /// None: the schema's own path. Some: a relocatable path pattern with typed holes ([`hole_ok`]).
    pub path: Option<&'static str>,
    /// Key names; `"*"` is every key of the schema.
    pub keys: &'static [&'static str],
    pub mode: Mode,
    /// The package whose schema may be missing on a machine (the audit skips the row then).
    #[cfg_attr(not(test), allow(dead_code))]
    pub optional: Option<&'static str>,
}

impl Allow {
    pub(crate) const fn new(
        schema: &'static str,
        keys: &'static [&'static str],
        mode: Mode,
    ) -> Allow {
        Allow {
            schema,
            path: None,
            keys,
            mode,
            optional: None,
        }
    }

    pub(crate) const fn at(
        schema: &'static str,
        path: &'static str,
        keys: &'static [&'static str],
        mode: Mode,
    ) -> Allow {
        Allow {
            schema,
            path: Some(path),
            keys,
            mode,
            optional: None,
        }
    }

    pub(crate) const fn optional(self, package: &'static str) -> Allow {
        Allow {
            optional: Some(package),
            ..self
        }
    }

    pub(crate) fn has_key(&self, key: &str) -> bool {
        self.keys.iter().any(|k| *k == "*" || *k == key)
    }
}

/// The six tables, by batch.
pub(crate) const TABLES: &[(&str, &[Allow])] = &[
    ("input", input::TABLE),
    ("devices", devices::TABLE),
    ("network", network::TABLE),
    ("system", system::TABLE),
    ("accounts", accounts::TABLE),
    ("apps", apps::TABLE),
];

/// Schemas no table may name (prefix match).
pub(crate) const FORBIDDEN_SCHEMAS: &[&str] = &[
    "org.gnome.shell.extensions.dash-to-dock",
    "org.gnome.shell.extensions.ubuntu-dock",
    "org.gnome.shell.extensions.ding",
    "org.gnome.shell.extensions.tiling-assistant",
];

/// Keys no table may name in a writing mode.
pub(crate) const FORBIDDEN_KEYS: &[(&str, &str)] = &[
    ("org.gnome.shell", "enabled-extensions"),
    ("org.gnome.shell", "disabled-extensions"),
    ("org.gnome.mutter", "experimental-features"),
];

/// Whether (schema, key) may never be written.
pub(crate) fn forbidden(schema: &str, key: &str) -> bool {
    FORBIDDEN_SCHEMAS.iter().any(|p| schema.starts_with(p))
        || FORBIDDEN_KEYS
            .iter()
            .any(|(s, k)| *s == schema && *k == key)
}

fn all<F: Fn(char) -> bool>(s: &str, f: F) -> bool {
    s.chars().all(f)
}

fn lower_hex(c: char) -> bool {
    c.is_ascii_digit() || ('a'..='f').contains(&c)
}

fn vidpid(s: &str) -> bool {
    s.len() == 9 && s.as_bytes()[4] == b':' && all(&s[..4], lower_hex) && all(&s[5..], lower_hex)
}

/// One typed hole of a path pattern against one path segment.
pub(crate) fn hole_ok(hole: &str, segment: &str) -> bool {
    match hole {
        "appid" => {
            (1..=128).contains(&segment.len())
                && all(segment, |c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
                })
        }
        "custom" => segment
            .strip_prefix("custom")
            .is_some_and(|n| (1..=4).contains(&n.len()) && all(n, |c| c.is_ascii_digit())),
        "vidpid" => vidpid(segment),
        // libmutter-18: stylus/%lx/ and stylus/default-%.4x:%.4x/ (amendment 9)
        "serial" => {
            ((1..=16).contains(&segment.len()) && all(segment, lower_hex))
                || segment.strip_prefix("default-").is_some_and(vidpid)
        }
        // libmutter-18: tablets/%.4x:%.4x/%s%c%s/: buttonA, ringA-ccw, stripB-up, dialA-cw ...
        "padctl" => ["button", "ring", "strip", "dial"].iter().any(|kind| {
            segment.strip_prefix(kind).is_some_and(|rest| {
                let mut chars = rest.chars();
                chars.next().is_some_and(|c| c.is_ascii_uppercase())
                    && match chars.as_str() {
                        "" => true,
                        tail => matches!(tail, "-cw" | "-ccw" | "-up" | "-down"),
                    }
            })
        }),
        "button" => segment
            .strip_prefix("button")
            .is_some_and(|rest| rest.len() == 1 && rest.chars().all(|c| c.is_ascii_uppercase())),
        "service" => {
            (1..=32).contains(&segment.len())
                && all(segment, |c| c.is_ascii_lowercase() || c == '-')
        }
        _ => false,
    }
}

/// Whether a relocatable `path` matches `pattern` (both start and end with `/`; a hole is a whole segment).
pub(crate) fn path_matches(pattern: &str, path: &str) -> bool {
    if !path.starts_with('/') || !path.ends_with('/') || path.contains("//") || path.len() > 512 {
        return false;
    }
    let (pattern, path): (Vec<&str>, Vec<&str>) =
        (pattern.split('/').collect(), path.split('/').collect());
    pattern.len() == path.len()
        && pattern.iter().zip(&path).all(|(p, s)| {
            match p.strip_prefix('{').and_then(|h| h.strip_suffix('}')) {
                Some(hole) => hole_ok(hole, s),
                None => p == s,
            }
        })
}

/// Every row in every table.
pub(crate) fn rows() -> impl Iterator<Item = (&'static str, &'static Allow)> {
    TABLES
        .iter()
        .flat_map(|(batch, table)| table.iter().map(move |row| (*batch, row)))
}

/// The row of (schema, path, key): for a fixed-path row the path is None (the caller has checked it against the
/// schema's own path), for a relocatable row it must match the pattern.
pub(crate) fn find(schema: &str, path: Option<&str>, key: &str) -> Option<&'static Allow> {
    rows().map(|(_, row)| row).find(|row| {
        row.schema == schema
            && row.has_key(key)
            && match (row.path, path) {
                (None, None) => true,
                (Some(pattern), Some(path)) => path_matches(pattern, path),
                _ => false,
            }
    })
}
