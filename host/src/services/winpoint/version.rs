//! WinPoint versions (D-G74a): SemVer 2.0.0 `MAJOR.MINOR.PATCH[-pre]` with the precedence of its §11. Releases are
//! stable only (`vX.Y.Z` tags); the prerelease part stays because a development build may be `0.10.0-dev`.
use std::cmp::Ordering;

/// One dot-separated prerelease identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ident {
    Numeric(u64),
    Alpha(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<Ident>,
}

/// A numeric part: ASCII digits, no leading zero (except "0" itself), within u64.
fn number(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0'))
    {
        return None;
    }
    s.parse().ok()
}

impl Version {
    /// `1.2.3` or `1.2.3-rc.1`. Refused: leading zeros, empty identifiers, build metadata (`+…`), anything
    /// that is not ASCII, and more than 64 bytes.
    pub(crate) fn parse(s: &str) -> Option<Self> {
        if s.is_empty() || s.len() > 64 || !s.is_ascii() || s.contains('+') {
            return None;
        }
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (s, None),
        };
        let mut parts = core.split('.');
        let (major, minor, patch) = (
            number(parts.next()?)?,
            number(parts.next()?)?,
            number(parts.next()?)?,
        );
        if parts.next().is_some() {
            return None;
        }
        let mut idents = Vec::new();
        if let Some(pre) = pre {
            for ident in pre.split('.') {
                if ident.is_empty()
                    || !ident
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return None;
                }
                if ident.bytes().all(|b| b.is_ascii_digit()) {
                    idents.push(Ident::Numeric(number(ident)?));
                } else {
                    idents.push(Ident::Alpha(ident.to_owned()));
                }
            }
        }
        Some(Self {
            major,
            minor,
            patch,
            pre: idents,
        })
    }
    /// A release tag: `v` and a version (`v0.9.0`).
    pub(crate) fn parse_tag(tag: &str) -> Option<Self> {
        Self::parse(tag.strip_prefix('v')?)
    }
    pub(crate) fn is_stable(&self) -> bool {
        self.pre.is_empty()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (i, ident) in self.pre.iter().enumerate() {
            f.write_str(if i == 0 { "-" } else { "." })?;
            match ident {
                Ident::Numeric(n) => write!(f, "{n}")?,
                Ident::Alpha(s) => f.write_str(s)?,
            }
        }
        Ok(())
    }
}

impl Ord for Ident {
    /// §11.4: numeric identifiers compare as numbers and rank below alphanumeric ones, which compare in ASCII.
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Ident::Numeric(a), Ident::Numeric(b)) => a.cmp(b),
            (Ident::Numeric(_), Ident::Alpha(_)) => Ordering::Less,
            (Ident::Alpha(_), Ident::Numeric(_)) => Ordering::Greater,
            (Ident::Alpha(a), Ident::Alpha(b)) => a.as_bytes().cmp(b.as_bytes()),
        }
    }
}
impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    /// §11: major, minor, patch; then a version with a prerelease ranks below the same one without; then the
    /// identifiers one by one, and a longer list ranks higher when all before are equal.
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}
impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_semver_and_refuses_the_rest() {
        let v = Version::parse("0.9.0").unwrap();
        assert_eq!(
            (v.major, v.minor, v.patch, v.pre.is_empty()),
            (0, 9, 0, true)
        );
        assert_eq!(Version::parse("10.20.30").unwrap().to_string(), "10.20.30");
        let rc = Version::parse("1.0.0-rc.1").unwrap();
        assert_eq!(rc.pre, [Ident::Alpha("rc".into()), Ident::Numeric(1)]);
        assert_eq!(rc.to_string(), "1.0.0-rc.1");
        assert!(!rc.is_stable());
        assert_eq!(
            Version::parse("1.0.0-x-y.0").unwrap().to_string(),
            "1.0.0-x-y.0"
        );
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2.3-",
            "1.2.3-a..b",
            "1.2.3-01",
            "1.2.3+build",
            "1.2.3-rc.1+b",
            "v1.2.3",
            "1.2.3 ",
            " 1.2.3",
            "1.2.x",
            "1.2.3-é",
            "١.2.3",
            "-1.2.3",
            "1.-2.3",
            "99999999999999999999.0.0",
            "1.2.3-a_b",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert!(
            Version::parse(&format!("1.2.3-{}", "a".repeat(64))).is_none(),
            "over 64 bytes"
        );
        assert_eq!(Version::parse_tag("v0.9.1"), Version::parse("0.9.1"));
        assert_eq!(Version::parse_tag("0.9.1"), None);
        assert_eq!(Version::parse_tag("V0.9.1"), None);
    }

    #[test]
    fn precedence_is_semver_paragraph_11() {
        let chain = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0",
            "10.0.0",
        ];
        for pair in chain.windows(2) {
            let (a, b) = (
                Version::parse(pair[0]).unwrap(),
                Version::parse(pair[1]).unwrap(),
            );
            assert!(a < b, "{a} < {b}");
            assert!(b > a);
        }
        assert!(Version::parse("0.10.0-dev").unwrap() > Version::parse("0.9.0").unwrap());
        assert!(Version::parse("0.10.0-dev").unwrap() < Version::parse("0.10.0").unwrap());
        assert_eq!(
            Version::parse("1.0.0")
                .unwrap()
                .cmp(&Version::parse("1.0.0").unwrap()),
            Ordering::Equal
        );
    }
}
