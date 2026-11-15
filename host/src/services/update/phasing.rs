//! Phased and held updates, classified by apt itself (design §4.6, critique M1, D-G73): PackageKit's apt backend
//! has no phasing code, so after every GetUpdates three batched, read-only apt programs answer for this machine, in
//! this order:
//! * `apt-cache policy ?phasing`: the packages apt's own rule holds back here (`pkgDepCache::PhasingApplied`: the
//!   machine id, `APT::Get::Always/Never-Include-Phased-Updates`, the `-security` exemption, the chroot check), the
//!   rule `apt upgrade` and unattended-upgrades follow; the pattern is apt 2.7.11's;
//! * `apt-cache policy -o APT::Cmd::Pattern-Only=true <name:arch>…`: per package the installed version, the
//!   candidate and the candidate's `(phased N%)` (apt's stored Phased-Update-Percentage);
//! * `apt-mark showhold`: the selection holds.
//!
//! All three name a package the way apt does (`FullName(true)`: the bare name for the native architecture and
//! `all`, `name:arch` for a foreign one), so their answers join on that name. When one of them fails (an exit code,
//! the timeout, a spawn error; an apt without `?phasing` exits 100) the fallback is one
//! `apt-cache show name=version …`: any `Phased-Update-Percentage` counts as phased (do-release-upgrade's rule,
//! conservative). The programs come only from the tool table (stubs in tests). Until D-G73 the first answer came from
//! a python3-apt script (`git show 6ebf169f:host/data/phasing.py`); `phasing_matches_the_python3_apt_script`
//! keeps its answers on a recorded real system.
//!
//! Review F5: the fallback runs on **every** failure of the apt answers. A package that neither answered for the
//! version PackageKit offers stays unclassified and gets the safe rule of [`unknown`]: only an update from the
//! `-security` pocket (never phased by Ubuntu) is offered; every other one is held back as phased ("Wordt later
//! aangeboden") until the next check classifies it. A missing stub (test mode) stops at once: no further program,
//! `test_seam`.
use super::{
    model::{Pkg, valid_arch, valid_name},
    paths::{SeamError, Tool, Tools},
};
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    os::unix::process::CommandExt,
    process::{Child, Stdio},
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(30);
/// `apt-cache policy` prints about 0.5 KiB per package (its version table); a release's worth of updates fits.
const MAX_OUTPUT: u64 = 16 << 20;
/// The apt pattern of "phasing holds this package back on this machine" (apt-patterns(7), apt 2.7.11 or later).
const PHASING: &str = "?phasing";
/// Names only: without it apt-cache searches a name it does not know as a regular expression (`libstdc+` prints
/// dozens of other packages). An older apt ignores the key.
const EXACT: [&str; 2] = ["-o", "APT::Cmd::Pattern-Only=true"];

/// The classification of one package.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Phasing {
    pub candidate: Option<String>,
    pub phased: bool,
    pub pup: Option<u32>,
    pub held: bool,
    pub installed: Option<String>,
}

/// Why one program gave no answer.
#[derive(Debug)]
enum Failure {
    Seam(SeamError),
    Failed,
}

/// Who classified the packages of one GetUpdates (stored as the snapshot's `phasing`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Classified {
    /// apt's own per-machine rule (`apt-cache policy ?phasing` with the candidates and holds)
    Apt,
    /// the `apt-cache show` fallback (any `Phased-Update-Percentage` is phased)
    Fallback,
    /// neither answered: [`unknown`]'s safe rule for every package
    Unknown,
}
impl Classified {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Classified::Apt => "apt",
            Classified::Fallback => "fallback",
            Classified::Unknown => "unknown",
        }
    }
}

/// The safe rule for a package nobody classified (review F5): Ubuntu never phases the `-security` pocket, so
/// such an update stays offered; any other is treated as phased until a later check classifies it.
pub(crate) fn unknown(pkg: &mut Pkg) {
    pkg.phased = !pkg.data.contains("-security");
}

/// One package block of `apt-cache policy` (the tools run with `LANG=C.UTF-8`: apt's untranslated labels).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Policy {
    pub installed: Option<String>,
    pub candidate: Option<String>,
    /// `(phased N%)` on the candidate's line of the version table (apt prints it only below 100)
    pub pup: Option<u32>,
}

/// apt's name of a package as `apt-cache policy` and `apt-mark showhold` print it: `name` or `name:arch`, both
/// valid. None for anything else (a label, a notice, a cut line).
fn apt_name(text: &str) -> Option<&str> {
    let valid = match text.split_once(':') {
        Some((name, arch)) => valid_name(name) && valid_arch(arch),
        None => valid_name(text),
    };
    valid.then_some(text)
}

/// The parser of `apt-cache policy <packages>` (`apt-private/private-show.cc`, apt 3.2): per package a line
/// `<apt name>:`, then `  Installed: <version|(none)>`, `  Candidate: <version|(none)>`, `  Version table:` and
/// per version ` *** <version> <priority>` (the installed one) or five spaces and the same, followed by
/// ` (phased N%)` when its percentage is below 100; the source lines under a version are indented further. In
/// output order; a header that is not an apt name starts no block.
pub(crate) fn parse_policy(output: &str) -> Vec<(String, Policy)> {
    let mut out: Vec<(String, Policy, Vec<(String, Option<u32>)>)> = Vec::new();
    let mut open = false;
    for line in output.lines() {
        if !line.starts_with(' ') {
            let name = line.strip_suffix(':').and_then(apt_name);
            open = name.is_some();
            if let Some(name) = name {
                out.push((name.to_owned(), Policy::default(), Vec::new()));
            }
            continue;
        }
        let Some((_, policy, versions)) = out.last_mut().filter(|_| open) else {
            continue;
        };
        let value = |v: &str| Some(v.trim()).filter(|v| !v.is_empty() && *v != "(none)").map(str::to_owned);
        if let Some(v) = line.strip_prefix("  Installed:") {
            policy.installed = value(v);
        } else if let Some(v) = line.strip_prefix("  Candidate:") {
            policy.candidate = value(v);
        } else if let Some(rest) = line.strip_prefix(" *** ").or_else(|| line.strip_prefix("     ")) {
            let f: Vec<&str> = rest.split(' ').collect();
            // a source line (`       %4i <index>`) has more spaces before its priority
            if f.len() < 2 || f[0].is_empty() || f[1].parse::<i64>().is_err() {
                continue;
            }
            let pup = match &f[2..] {
                ["(phased", percent] => percent.strip_suffix("%)").and_then(|p| p.parse().ok()).filter(|p| *p <= 100),
                _ => None,
            };
            versions.push((f[0].to_owned(), pup));
        }
    }
    out.into_iter()
        .map(|(name, mut policy, versions)| {
            policy.pup = versions.iter().find(|(v, _)| Some(v) == policy.candidate.as_ref()).and_then(|(_, p)| *p);
            (name, policy)
        })
        .collect()
}

/// The parser of `apt-mark showhold`: one apt name per line.
pub(crate) fn parse_holds(output: &str) -> HashSet<String> {
    output.lines().filter_map(|line| apt_name(line.trim())).map(str::to_owned).collect()
}

/// Join apt's three answers per requested `(name, arch)`. `name:arch` in `policy` is that package; a bare name
/// (native architecture or `all`) is the one requested package of that name no qualified name claimed (two such stay
/// out, and get the safe rule). Out too: a package `?phasing` lists with another candidate than `policy` (the lists
/// changed between the two calls). `?phasing` runs first, so a change in between always shows: either as that
/// difference, or as a `policy` candidate that is not the version PackageKit offers ([`apply`]'s safe rule).
pub(crate) fn join(
    wanted: &[(String, String)],
    policy: &[(String, Policy)],
    phasing: &[(String, Policy)],
    holds: &HashSet<String>,
) -> HashMap<(String, String), Phasing> {
    let blocks: HashMap<&str, &Policy> = policy.iter().map(|(name, p)| (name.as_str(), p)).collect();
    let phased: HashMap<&str, &Policy> = phasing.iter().map(|(name, p)| (name.as_str(), p)).collect();
    let mut names: HashMap<&(String, String), String> = HashMap::new();
    for key in wanted {
        let qualified = format!("{}:{}", key.0, key.1);
        if blocks.contains_key(qualified.as_str()) {
            names.insert(key, qualified);
        }
    }
    for name in blocks.keys().filter(|name| !name.contains(':')) {
        let open: Vec<&(String, String)> =
            wanted.iter().filter(|key| key.0 == *name && !names.contains_key(key)).collect();
        if let [key] = open.as_slice() {
            names.insert(key, (*name).to_owned());
        }
    }
    let mut out = HashMap::new();
    for (key, name) in names {
        let block = blocks[name.as_str()];
        let held_back = phased.get(name.as_str());
        if held_back.is_some_and(|p| p.candidate != block.candidate) {
            continue;
        }
        out.insert(
            key.clone(),
            Phasing {
                candidate: block.candidate.clone(),
                phased: held_back.is_some(),
                pup: block.pup,
                held: holds.contains(&name),
                installed: block.installed.clone(),
            },
        );
    }
    out
}

/// The parser of `apt-cache show name=version …`: stanzas separated by blank lines; any
/// `Phased-Update-Percentage` → phased. Keyed by (name, version).
pub(crate) fn parse_show(output: &str) -> HashMap<(String, String), Phasing> {
    let mut out = HashMap::new();
    for stanza in output.split("\n\n") {
        let field = |name: &str| {
            stanza
                .lines()
                .find_map(|l| l.strip_prefix(name).and_then(|rest| rest.strip_prefix(':')))
                .map(|v| v.trim().to_owned())
        };
        let (Some(package), Some(version)) = (field("Package"), field("Version")) else {
            continue;
        };
        let pup = field("Phased-Update-Percentage");
        out.insert(
            (package, version.clone()),
            Phasing {
                candidate: Some(version),
                phased: pup.is_some(),
                pup: pup.and_then(|p| p.parse().ok()).filter(|p| *p <= 100),
                held: false,
                installed: None,
            },
        );
    }
    out
}

/// Apply a classification to GetUpdates' packages: phased and held ones never enter a row or a job.
/// A held package (info 9) stays held whatever apt says. A package without an answer for the offered version
/// (not in the output, no candidate, or apt's candidate is another version) gets [`unknown`]'s safe rule.
/// Returns how many packages stayed unclassified.
pub(crate) fn apply(pkgs: &mut [Pkg], apt: Option<&HashMap<(String, String), Phasing>>, show: Option<&HashMap<(String, String), Phasing>>) -> usize {
    let mut unclassified = 0;
    for pkg in pkgs.iter_mut() {
        if let Some(found) = apt.and_then(|m| m.get(&(pkg.name.clone(), pkg.arch.clone()))) {
            pkg.pup = found.pup;
            pkg.from = found.installed.clone().or(pkg.from.take());
            pkg.held |= found.held;
            // apt's candidate must be the version PackageKit offers, or the rule talks about another version
            if found.candidate.as_deref() == Some(pkg.to.as_str()) {
                pkg.phased = found.phased;
                continue;
            }
        } else if let Some(found) = show.and_then(|m| m.get(&(pkg.name.clone(), pkg.to.clone()))) {
            pkg.phased = found.phased;
            pkg.pup = found.pup;
            continue;
        }
        unknown(pkg);
        unclassified += 1;
    }
    unclassified
}

/// Run a tool with `input` on stdin, niced (10) and idle I/O priority, at most [`TIMEOUT`]; stdout or None.
fn run(tools: &Tools, tool: Tool, args: &[&str], input: &[u8]) -> Result<(i32, String), Failure> {
    let mut command = tools.command(tool).map_err(Failure::Seam)?;
    command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            libc::nice(10);
            // IOPRIO_WHO_PROCESS = 1, class idle (3) << 13
            libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|_| Failure::Failed)?;
    let mut stdout = child.stdout.take().ok_or(Failure::Failed)?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stdout).take(MAX_OUTPUT).read_to_end(&mut bytes);
        bytes
    });
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    let status = wait(&mut child);
    let bytes = reader.join().unwrap_or_default();
    match status {
        Some(code) => Ok((code, String::from_utf8_lossy(&bytes).into_owned())),
        None => Err(Failure::Failed),
    }
}
/// Wait up to [`TIMEOUT`]; kill our own child after it.
fn wait(child: &mut Child) -> Option<i32> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.code().unwrap_or(-1)),
            Ok(None) if started.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// One of apt's answers: its stdout after exit 0; None after any other end.
fn answer(result: Result<(i32, String), Failure>) -> Result<Option<String>, SeamError> {
    match result {
        Ok((0, out)) => Ok(Some(out)),
        Ok(_) | Err(Failure::Failed) => Ok(None),
        Err(Failure::Seam(error)) => Err(error),
    }
}

/// apt's three answers for `wanted`, joined ([`join`]); None as soon as one program gave no answer.
fn from_apt(tools: &Tools, wanted: &[(String, String)]) -> Result<Option<HashMap<(String, String), Phasing>>, SeamError> {
    let Some(phasing) = answer(run(tools, Tool::AptCache, &["policy", PHASING], b""))? else {
        return Ok(None);
    };
    let specs: Vec<String> = wanted.iter().map(|(name, arch)| format!("{name}:{arch}")).collect();
    let mut args: Vec<&str> = vec!["policy"];
    args.extend(EXACT);
    args.extend(specs.iter().map(String::as_str));
    let Some(policy) = answer(run(tools, Tool::AptCache, &args, b""))? else {
        return Ok(None);
    };
    let Some(holds) = answer(run(tools, Tool::AptMark, &["showhold"], b""))? else {
        return Ok(None);
    };
    Ok(Some(join(wanted, &parse_policy(&policy), &parse_policy(&phasing), &parse_holds(&holds))))
}

/// Classify `pkgs` in place: apt's answers; on any failure of them the `apt-cache show` fallback; what neither
/// answered gets [`unknown`]'s safe rule (review F5). Only valid names and versions reach the programs.
/// `Err`: a stub is missing (test mode, fail closed): no further program ran and every package has the safe
/// rule; the caller reports `test_seam`.
pub(crate) fn classify(tools: &Tools, pkgs: &mut [Pkg]) -> Result<Classified, SeamError> {
    let mut wanted: Vec<(String, String)> = Vec::new();
    for p in pkgs.iter().filter(|p| valid_name(&p.name) && valid_arch(&p.arch)) {
        if !wanted.iter().any(|(name, arch)| *name == p.name && *arch == p.arch) {
            wanted.push((p.name.clone(), p.arch.clone()));
        }
    }
    if wanted.is_empty() {
        apply(pkgs, None, None);
        return Ok(if pkgs.is_empty() { Classified::Apt } else { Classified::Unknown });
    }
    let specs: Vec<String> = pkgs
        .iter()
        .filter(|p| valid_name(&p.name) && valid_arch(&p.arch))
        .filter(|p| p.to.bytes().all(|b| b.is_ascii_alphanumeric() || b"+-.:~".contains(&b)))
        .map(|p| format!("{}={}", p.name, p.to))
        .collect();
    match from_apt(tools, &wanted) {
        Ok(Some(found)) => {
            apply(pkgs, Some(&found), None);
            return Ok(Classified::Apt);
        }
        Err(error) => {
            apply(pkgs, None, None);
            return Err(error);
        }
        // an exit code (no `?phasing` in an older apt), the timeout, a spawn error: the fallback
        Ok(None) => {}
    }
    if specs.is_empty() {
        apply(pkgs, None, None);
        return Ok(Classified::Unknown);
    }
    let mut args: Vec<&str> = vec!["show", "--no-all-versions"];
    args.extend(specs.iter().map(String::as_str));
    match run(tools, Tool::AptCache, &args, b"") {
        Ok((code, out)) if code == 0 || !out.trim().is_empty() => {
            let parsed = parse_show(&out);
            apply(pkgs, None, Some(&parsed));
            Ok(if parsed.is_empty() { Classified::Unknown } else { Classified::Fallback })
        }
        Err(Failure::Seam(error)) => {
            apply(pkgs, None, None);
            Err(error)
        }
        Ok(_) | Err(Failure::Failed) => {
            apply(pkgs, None, None);
            Ok(Classified::Unknown)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A file of tests/update/fixtures/phasing (read at run time: the source tarball has no tests/).
    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/update/fixtures/phasing").join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
    fn key(name: &str, arch: &str) -> (String, String) {
        (name.to_owned(), arch.to_owned())
    }
    fn some(text: &str) -> Option<String> {
        Some(text.to_owned())
    }

    /// The answers of the D-G73 classifier on a recorded real system equal those of the python3-apt script it
    /// replaced (`git show 6ebf169f:host/data/phasing.py`), recorded at the same moment on the same machine
    /// (Ubuntu 26.04.1, apt 3.2.1, python3-apt 3.1.0; tests/update/fixtures/phasing/README.md): candidate,
    /// phasing_applied, percentage, hold and installed version of each of the 19 packages, among them eight that
    /// phasing holds back, three with a percentage this machine already passed, a foreign `:i386` and a native
    /// package of the same name, `all` ones, one not installed and one apt does not know.
    #[test]
    fn phasing_matches_the_python3_apt_script() {
        let wanted: Vec<(String, String)> = fixture("recorded/input.tsv")
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(name, arch)| key(name, arch))
            .collect();
        assert_eq!(wanted.len(), 19);
        let got = join(
            &wanted,
            &parse_policy(&fixture("recorded/policy.txt")),
            &parse_policy(&fixture("recorded/phasing.txt")),
            &parse_holds(&fixture("recorded/showhold.txt")),
        );
        let mut rows = 0;
        for line in fixture("recorded/phasing-py.txt").lines().take_while(|l| *l != "@@end") {
            let f: Vec<&str> = line.split('\t').collect();
            assert_eq!(f.len(), 7, "{line}");
            let dash = |v: &str| (v != "-").then(|| v.to_owned());
            let want = Phasing {
                candidate: dash(f[2]),
                phased: f[3] == "1",
                pup: dash(f[4]).map(|p| p.parse().unwrap()),
                held: f[5] == "1",
                installed: dash(f[6]),
            };
            let got = got.get(&key(f[0], f[1])).cloned();
            if want.candidate.is_none() {
                // apt does not know it: no block, so no answer (the script printed dashes; both give the safe rule)
                assert_eq!(got, None, "{line}");
            } else {
                assert_eq!(got, Some(want), "{line}");
            }
            rows += 1;
        }
        assert_eq!(rows, 19);
        assert_eq!(got.values().filter(|p| p.phased).count(), 8);
        assert_eq!(got.values().filter(|p| !p.phased && p.pup.is_some()).count(), 3, "percentage already passed");
    }

    #[test]
    fn policy_blocks_versions_and_percentages() {
        let text = "foo:\n  Installed: 1.0\n  Candidate: 1.1\n  Version table:\n     1.1 500 (phased 40%)\n        500 \
                    http://archive.ubuntu.com/ubuntu resolute-updates/main amd64 Packages\n *** 1.0 100\n        100 \
                    /var/lib/dpkg/status\nfoo:i386:\n  Installed: (none)\n  Candidate: 2:1.1~rc1+b1\n  Version table:\n     \
                    2:1.1~rc1+b1 -10 (phased 0%)\n       1001 /some/where\nPinned packages:\n  Installed: 9\nbar:\n  \
                    Installed: 3\n  Candidate: (none)\n  Version table:\n *** 3 100 (phased 5%)\n";
        let blocks = parse_policy(text);
        let names: Vec<&str> = blocks.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["foo", "foo:i386", "bar"], "a label is no block, and its lines belong to none");
        assert_eq!(blocks[0].1, Policy { installed: some("1.0"), candidate: some("1.1"), pup: Some(40) });
        assert_eq!(blocks[1].1, Policy { installed: None, candidate: some("2:1.1~rc1+b1"), pup: Some(0) });
        // the percentage is the candidate's: none without a candidate, whatever the installed line says
        assert_eq!(blocks[2].1, Policy { installed: some("3"), candidate: None, pup: None });
        // a candidate at 100 % prints no percentage; another version's does not count
        let full = parse_policy("baz:\n  Installed: 1\n  Candidate: 2\n  Version table:\n     2 500\n *** 1 100 (phased 10%)\n");
        assert_eq!(full[0].1.pup, None);
        // odd lines change nothing: an unknown label, a version line without priority, a cut percentage
        let odd = parse_policy("qq:\n  Installed: 1\n  Candidate: 2\n  Something: x\n     2\n     2 500 (phased\n     2 500 (phased 101%)\n");
        assert_eq!(odd[0].1, Policy { installed: some("1"), candidate: some("2"), pup: None });
        assert!(parse_policy("").is_empty());
        assert!(parse_policy("E: input:0-8: error: Unrecognized pattern '?phasing'\n").is_empty());
    }

    #[test]
    fn holds_are_apt_names() {
        let holds = parse_holds("linux-generic\nlibfoo1:i386\n\nN: not a name\nUpper\n");
        assert_eq!(holds, HashSet::from(["linux-generic".to_owned(), "libfoo1:i386".to_owned()]));
    }

    /// The bare name is the native or `all` package; `name:arch` the foreign one; a held package and one that
    /// phasing holds back are found by the same name; two requested packages one bare name could mean stay out.
    #[test]
    fn join_by_apt_name() {
        let block = |candidate: &str| Policy { installed: some("1"), candidate: some(candidate), pup: Some(10) };
        let wanted = vec![key("libfoo1", "amd64"), key("libfoo1", "i386"), key("tool", "all"), key("gone", "amd64"), key("odd", "amd64"), key("odd", "armhf")];
        let policy = vec![
            ("libfoo1".to_owned(), block("2")),
            ("libfoo1:i386".to_owned(), block("2")),
            ("tool".to_owned(), block("5")),
            ("odd".to_owned(), block("7")),
        ];
        let phasing = vec![("libfoo1:i386".to_owned(), block("2")), ("tool".to_owned(), block("5"))];
        let holds = HashSet::from(["libfoo1".to_owned()]);
        let got = join(&wanted, &policy, &phasing, &holds);
        let native = &got[&key("libfoo1", "amd64")];
        assert!(!native.phased && native.held && native.pup == Some(10) && native.installed.as_deref() == Some("1"));
        let foreign = &got[&key("libfoo1", "i386")];
        assert!(foreign.phased && !foreign.held);
        assert!(got[&key("tool", "all")].phased);
        assert!(!got.contains_key(&key("gone", "amd64")), "no block: no answer");
        assert!(!got.contains_key(&key("odd", "amd64")) && !got.contains_key(&key("odd", "armhf")), "ambiguous");
        assert_eq!(got.len(), 3);
        // a package ?phasing lists with another candidate (the lists changed in between): no answer
        let moved = join(&wanted[..1], &policy[..1], &[("libfoo1".to_owned(), block("3"))], &HashSet::new());
        assert!(moved.is_empty());
    }

    /// apply() with apt's answer: the offered version must be apt's candidate, a hold sticks, `from` comes from apt.
    #[test]
    fn apply_uses_apt_only_for_the_offered_version() {
        let pkg = |name: &str, to: &str, data: &str| Pkg {
            name: name.into(),
            arch: "amd64".into(),
            to: to.into(),
            data: data.into(),
            ..Pkg::default()
        };
        let mut pkgs = vec![pkg("a", "2", "ubuntu-resolute-updates"), pkg("b", "9", "ubuntu-resolute-updates"), pkg("c", "1", "ubuntu-resolute-security")];
        let found = HashMap::from([
            (key("a", "amd64"), Phasing { candidate: some("2"), phased: true, pup: Some(30), held: true, installed: some("1") }),
            (key("b", "amd64"), Phasing { candidate: some("8"), phased: false, pup: None, held: false, installed: some("7") }),
        ]);
        assert_eq!(apply(&mut pkgs, Some(&found), None), 2);
        assert!(pkgs[0].phased && pkgs[0].held && pkgs[0].pup == Some(30) && pkgs[0].from.as_deref() == Some("1"));
        assert!(pkgs[1].phased, "apt's candidate is another version: the safe rule");
        assert_eq!(pkgs[1].from.as_deref(), Some("7"));
        assert!(!pkgs[2].phased, "no answer, but -security: offered");
    }
}
