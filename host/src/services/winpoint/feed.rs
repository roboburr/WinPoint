//! GitHub's release list of WinPoint (D-G74a): `GET {api}repos/roboburr/WinPoint/releases?per_page=10`, parsed
//! into stable releases with this PC's asset set, and the choice of the one to offer.
//!
//! * Read per release: `tag_name` (`vX.Y.Z`, no prerelease part), `draft` and `prerelease` (both false), `name`
//!   (else "WinPoint X.Y.Z"), `html_url` (once parsed, exactly the release's own page
//!   `{web}roboburr/WinPoint/releases/tag/{tag}`, kept in that normal form: a dot segment, `%2e%2e` or a backslash
//!   cannot point the card's links at another repository), `published_at` and `assets[]` (`name`, `size`, `state`
//!   "uploaded", `browser_download_url` exactly `{web}roboburr/WinPoint/releases/download/{tag}/{name}`, `digest`
//!   "sha256:<hex>" or null). The release body is never shown: the card links to the release page.
//! * The asset set of a version V on ARCH (`uname -m`): `winpoint-V-linux-ARCH.tar.gz` (1 byte to 256 MiB) and its
//!   `.sha256` (at most 4 KiB). There is no signature (D-G74, no signing key): the tarball's `digest` from GitHub's
//!   API is the trust anchor (verify.rs); a release whose tarball has none is offered with the reason `noDigest`,
//!   which WinPoint never downloads itself. Other assets are ignored.
//! * An entry that fails a check is skipped; a body that is not a JSON array is a feed error.
use super::version::Version;
use serde_json::{Value, json};

pub(crate) const REPO: &str = "roboburr/WinPoint";
/// The largest feed body (the API list of 10 releases is a few tens of KiB).
pub(crate) const MAX_FEED: u64 = 2 << 20;
pub(crate) const MAX_TARBALL: u64 = 256 << 20;
pub(crate) const MAX_SMALL: u64 = 4 << 10;
const MAX_NAME: usize = 200;

/// Where the feed and the release pages live: GitHub, or the test seam's loopback origin for both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Origin {
    pub api: url::Url,
    pub web: url::Url,
}
impl Origin {
    pub(crate) fn github() -> Self {
        Self {
            api: url::Url::parse("https://api.github.com/").expect("constant URL"),
            web: url::Url::parse("https://github.com/").expect("constant URL"),
        }
    }
    pub(crate) fn test(origin: &url::Url) -> Self {
        Self {
            api: origin.clone(),
            web: origin.clone(),
        }
    }
    pub(crate) fn feed_url(&self) -> url::Url {
        self.api
            .join(&format!("repos/{REPO}/releases?per_page=10"))
            .expect("constant path")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Asset {
    pub name: String,
    pub size: u64,
    pub url: url::Url,
    /// GitHub's own "sha256:<64 hex>", when the API gives one
    pub digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Assets {
    pub tarball: Asset,
    pub sha256: Asset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Release {
    pub version: Version,
    pub tag: String,
    pub name: String,
    pub published_at: Option<String>,
    /// the release page (`html_url`): its notes and its downloads
    pub notes_url: String,
    /// None: no complete asset set for this PC's architecture
    pub assets: Option<Assets>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FeedError {
    TooLarge,
    NotJson,
    NotArray,
}

/// `winpoint-V-linux-ARCH.tar.gz` (frozen from 0.9.0 on: the updater of version N reads the assets of N+1).
pub(crate) fn tarball_name(version: &Version, arch: &str) -> String {
    format!("winpoint-{version}-linux-{arch}.tar.gz")
}

/// This PC's architecture as `uname -m` prints it (`x86_64`, `aarch64`).
pub(crate) fn machine() -> String {
    let mut name: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut name) } != 0 {
        return String::new();
    }
    let bytes: Vec<u8> = name
        .machine
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn digest_of(value: &Value) -> Result<Option<String>, ()> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) => {
            let hex = s.strip_prefix("sha256:").ok_or(())?;
            if hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                Ok(Some(s.clone()))
            } else {
                Err(())
            }
        }
        _ => Err(()),
    }
}

/// One uploaded asset whose download URL is exactly the release's own; None for anything else.
fn asset(value: &Value, download: &str) -> Option<Asset> {
    let name = value["name"].as_str()?;
    if name.is_empty() || name.len() > MAX_NAME || value["state"] != "uploaded" {
        return None;
    }
    let url = value["browser_download_url"].as_str()?;
    if url != format!("{download}{name}") {
        return None;
    }
    Some(Asset {
        name: name.to_owned(),
        size: value["size"].as_u64()?,
        url: url::Url::parse(url).ok()?,
        digest: digest_of(value.get("digest").unwrap_or(&Value::Null)).ok()?,
    })
}

fn text(value: &Value, max: usize) -> Option<String> {
    let s = value.as_str()?.trim();
    (!s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)).then(|| s.to_owned())
}

/// The release's own page from its `html_url`, parsed first and compared in the normal form (the one stored and
/// opened): `{web}roboburr/WinPoint/releases/tag/{tag}`, no query, no fragment. A prefix test on the raw string would
/// take `…/releases/../../../someone/else/…`, which a browser opens as another repository's page.
pub(crate) fn release_page(raw: &str, origin: &Origin, tag: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    let page = origin
        .web
        .join(&format!("{REPO}/releases/tag/{tag}"))
        .ok()?;
    (parsed == page).then(|| page.as_str().to_owned())
}

/// One stable, published release of WinPoint, with this PC's asset set when it is complete.
fn release(value: &Value, origin: &Origin, arch: &str) -> Option<Release> {
    let tag = value["tag_name"].as_str()?;
    let version = Version::parse_tag(tag)?;
    if !version.is_stable() || value["draft"] != false || value["prerelease"] != false {
        return None;
    }
    let notes_url = release_page(value["html_url"].as_str()?, origin, tag)?;
    let download = format!("{}{REPO}/releases/download/{tag}/", origin.web);
    let assets: Vec<Asset> = value["assets"]
        .as_array()?
        .iter()
        .filter_map(|a| asset(a, &download))
        .collect();
    let named = |name: &str| assets.iter().find(|a| a.name == name).cloned();
    let tarball = tarball_name(&version, arch);
    let set = match (named(&tarball), named(&format!("{tarball}.sha256"))) {
        (Some(tarball), Some(sha256))
            if (1..=MAX_TARBALL).contains(&tarball.size) && sha256.size <= MAX_SMALL =>
        {
            Some(Assets { tarball, sha256 })
        }
        _ => None,
    };
    Some(Release {
        name: text(&value["name"], MAX_NAME).unwrap_or_else(|| format!("WinPoint {version}")),
        published_at: text(&value["published_at"], 40).filter(|s| s.is_ascii()),
        notes_url,
        tag: tag.to_owned(),
        version,
        assets: set,
    })
}

/// The releases of a feed body (newest first as GitHub sends them; [`choose`] does not rely on the order).
pub(crate) fn parse_releases(
    body: &[u8],
    origin: &Origin,
    arch: &str,
) -> Result<Vec<Release>, FeedError> {
    if body.len() as u64 > MAX_FEED {
        return Err(FeedError::TooLarge);
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| FeedError::NotJson)?;
    let list = value.as_array().ok_or(FeedError::NotArray)?;
    Ok(list
        .iter()
        .filter_map(|r| release(r, origin, arch))
        .collect())
}

/// The release to offer: the highest one newer than `current` that has a complete asset set; when none has,
/// the highest newer one (the card says there is no download for this PC yet). Never a downgrade.
pub(crate) fn choose<'a>(all: &'a [Release], current: &Version) -> Option<&'a Release> {
    let newer = || all.iter().filter(|r| r.version > *current);
    newer()
        .filter(|r| r.assets.is_some())
        .max_by(|a, b| a.version.cmp(&b.version))
        .or_else(|| newer().max_by(|a, b| a.version.cmp(&b.version)))
}

impl Asset {
    fn json(&self) -> Value {
        json!({"name":self.name,"size":self.size,"url":self.url.as_str(),"digest":self.digest})
    }
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            name: value["name"].as_str()?.to_owned(),
            size: value["size"].as_u64()?,
            url: url::Url::parse(value["url"].as_str()?).ok()?,
            digest: value["digest"].as_str().map(str::to_owned),
        })
    }
}

impl Release {
    /// state.json `latest`. The job never downloads from these: it asks GitHub again when it starts (job.rs).
    pub(crate) fn json(&self) -> Value {
        let assets = self
            .assets
            .as_ref()
            .map(|a| json!({"tarball":a.tarball.json(),"sha256":a.sha256.json()}));
        json!({"version":self.version.to_string(),"tag":self.tag,"name":self.name,"publishedAt":self.published_at,
               "notesUrl":self.notes_url,"assets":assets})
    }
    pub(crate) fn from_json(value: &Value) -> Option<Self> {
        let assets = match &value["assets"] {
            Value::Null => None,
            a => Some(Assets {
                tarball: Asset::from_json(&a["tarball"])?,
                sha256: Asset::from_json(&a["sha256"])?,
            }),
        };
        Some(Self {
            version: Version::parse(value["version"].as_str()?)?,
            tag: value["tag"].as_str()?.to_owned(),
            name: value["name"].as_str()?.to_owned(),
            published_at: value["publishedAt"].as_str().map(str::to_owned),
            notes_url: value["notesUrl"].as_str()?.to_owned(),
            assets,
        })
    }
}
