//! Phased and held updates, classified with apt's own rule (design §4.6, critique M1): PackageKit's apt
//! backend has no phasing code, so after every GetUpdates one batched, read-only `python3 -I -c <SCRIPT>`
//! opens `apt_pkg.Cache(None)` + `DepCache` (no lock) and prints, per package, its candidate,
//! `phasing_applied`, `Phased-Update-Percentage` and the selection hold. Without python3-apt the fallback is
//! one `apt-cache show name=version …`: any `Phased-Update-Percentage` counts as phased (do-release-upgrade's
//! rule, conservative). Both programs come only from the tool table (stubs in tests).
//!
//! Review F5: the fallback runs on **every** failure of the script (an exit code, a missing end marker, the
//! timeout, a spawn error). A package that neither answered for the version PackageKit offers stays
//! unclassified and gets the safe rule of [`unknown`]: only an update from the `-security` pocket (never phased
//! by Ubuntu) is offered; every other one is held back as phased ("Wordt later aangeboden") until the next
//! check classifies it. A missing stub (test mode) stops at once: no further program, `test_seam`.
use super::{
    model::{Pkg, valid_arch, valid_name},
    paths::{SeamError, Tool, Tools},
};
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::unix::process::CommandExt,
    process::{Child, Stdio},
    time::{Duration, Instant},
};

/// Read-only: no lock, no write, no network. One input line per package: `name<TAB>arch`. One output line
/// per input line: `name<TAB>arch<TAB>candidate|-<TAB>phasing_applied 0/1<TAB>percentage|-<TAB>held 0/1<TAB>
/// installed|-` (GetUpdates has no installed version, so the snapshot's `from` comes from here).
/// Exit 3 with `@@no-apt-pkg` when python3-apt is missing.
/// WU-FIX (GW2b review n3): the text lives in `host/data/phasing.py`, which `tools/update/probe.py --phasing` runs
/// too (byte for byte: the test `probe_py_runs_the_script_this_module_ships`), so R1 measures what ships.
const SCRIPT: &str = include_str!("../../../data/phasing.py");
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT: u64 = 4 << 20;

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
    /// the `apt_pkg` script (apt's own per-machine rule)
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

/// The parser of [`SCRIPT`]'s output; None when the end marker is missing (a crash or a cut pipe).
pub(crate) fn parse_script(output: &str) -> Option<HashMap<(String, String), Phasing>> {
    let mut out = HashMap::new();
    let mut ended = false;
    for line in output.lines() {
        if line == "@@end" {
            ended = true;
            break;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 7 || !valid_name(f[0]) || !valid_arch(f[1]) {
            continue;
        }
        out.insert(
            (f[0].to_owned(), f[1].to_owned()),
            Phasing {
                candidate: (f[2] != "-").then(|| f[2].to_owned()),
                phased: f[3] == "1",
                pup: f[4].parse().ok().filter(|p| *p <= 100),
                held: f[5] == "1",
                installed: (f[6] != "-").then(|| f[6].to_owned()),
            },
        );
    }
    ended.then_some(out)
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
pub(crate) fn apply(pkgs: &mut [Pkg], script: Option<&HashMap<(String, String), Phasing>>, show: Option<&HashMap<(String, String), Phasing>>) -> usize {
    let mut unclassified = 0;
    for pkg in pkgs.iter_mut() {
        if let Some(found) = script.and_then(|m| m.get(&(pkg.name.clone(), pkg.arch.clone()))) {
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

/// Classify `pkgs` in place: the script; on any failure of it the `apt-cache show` fallback; what neither
/// answered gets [`unknown`]'s safe rule (review F5). Only valid names and versions reach the programs.
/// `Err`: a stub is missing (test mode, fail closed): no further program ran and every package has the safe
/// rule; the caller reports `test_seam`.
pub(crate) fn classify(tools: &Tools, pkgs: &mut [Pkg]) -> Result<Classified, SeamError> {
    let wanted: Vec<&Pkg> = pkgs.iter().filter(|p| valid_name(&p.name) && valid_arch(&p.arch)).collect();
    if wanted.is_empty() {
        apply(pkgs, None, None);
        return Ok(if pkgs.is_empty() { Classified::Apt } else { Classified::Unknown });
    }
    let input: String = wanted.iter().map(|p| format!("{}\t{}\n", p.name, p.arch)).collect();
    let specs: Vec<String> = wanted
        .iter()
        .filter(|p| p.to.bytes().all(|b| b.is_ascii_alphanumeric() || b"+-.:~".contains(&b)))
        .map(|p| format!("{}={}", p.name, p.to))
        .collect();
    match run(tools, Tool::Python3, &["-I", "-c", SCRIPT], input.as_bytes()) {
        Ok((0, out)) => {
            if let Some(parsed) = parse_script(&out) {
                apply(pkgs, Some(&parsed), None);
                return Ok(Classified::Apt);
            }
        }
        Err(Failure::Seam(error)) => {
            apply(pkgs, None, None);
            return Err(error);
        }
        // exit 3 (no python3-apt), a traceback, the timeout, a spawn error: the fallback
        Ok(_) | Err(Failure::Failed) => {}
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

// WU-FIX (GW2b review n3): one phasing script for the service and the R1 probe.
#[cfg(test)]
mod tests {
    use super::SCRIPT;
    use std::path::Path;

    /// `tools/update/probe.py --phasing` runs exactly the bytes this module ships. Read as text, not run (no
    /// program starts outside the tool table, `no_absolute_exec_path_outside_the_tool_table`): the embedded
    /// script is the file `host/data/phasing.py` byte for byte, and probe.py has no script text of its own: it reads
    /// that file (`parents[2]` of `tools/update/probe.py` is the repository root) and passes what it read as the
    /// `-c` program in both of its calls (`--live` and the stub directory).
    #[test]
    fn probe_py_runs_the_script_this_module_ships() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the repository root");
        let shared = std::fs::read(root.join("host/data/phasing.py")).expect("host/data/phasing.py");
        assert_eq!(SCRIPT.as_bytes(), shared.as_slice(), "SCRIPT is the shared file");
        let probe = std::fs::read_to_string(root.join("tools/update/probe.py")).expect("tools/update/probe.py");
        let code: Vec<&str> = probe.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
        let has = |needle: &str| code.iter().filter(|l| l.contains(needle)).count();
        assert_eq!(has("PHASING_FILE = Path(__file__).resolve().parents[2] / 'host' / 'data' / 'phasing.py'"), 1);
        assert_eq!(has("return PHASING_FILE.read_bytes().decode('utf-8')"), 1);
        assert_eq!(has("script = phasing_script()"), 1);
        assert_eq!(has("'-I', '-c', script]"), 2, "the live call and the stub call run the file's text");
        assert_eq!(has("-c'"), 2, "no other python3 -c call");
        assert!(!probe.contains("import apt_pkg"), "probe.py holds no script text of its own");
        // the parser here reads what the script prints: seven fields per package and the end marker
        assert!(SCRIPT.contains("print(\"@@end\")") && SCRIPT.contains("@@no-apt-pkg"));
        assert!(!SCRIPT.contains("release"), "no word the release-upgrader detection (guard.rs) matches");
    }
}
