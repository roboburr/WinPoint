//! End to end (D-G74a): the owner's update check against a fake GitHub on 127.0.0.1 (tiny_http,
//! already a host dependency) through the test seam, as Settings sees it: `winpoint.check` forwarded from a
//! Settings host, `winpoint.status`, `winpoint.changed`, the state files and the notification decision. Only
//! loopback is used; `Client::production()` appears only in the TLS test of tests.rs.
use super::{
    WinPoint, check,
    feed::{self, Origin},
    job, notify,
    seam::Seam,
    tests::{Temp, host, releases_at, v},
};
use crate::services::update::{net::Net, store};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const FEED: &str = "/repos/roboburr/WinPoint/releases?per_page=10";
/// Where GitHub sends the API of a renamed repository (a 301 hop, as for any moved repository).
const MOVED: &str = "/repositories/4711/releases?per_page=10";

/// What the fake answers on the moved list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    List,
    NotFound,
    Limited,
    Garbage,
}

struct FakeGitHub {
    origin: url::Url,
    answer: Arc<Mutex<Answer>>,
    requests: Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl FakeGitHub {
    fn start() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let origin = url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        let body = releases_at(origin.as_str());
        let answer = Arc::new(Mutex::new(Answer::List));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (mode, seen, stopping) = (answer.clone(), requests.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let request = match server.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(request)) => request,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                let headers = request
                    .headers()
                    .iter()
                    .map(|h| (h.field.as_str().to_string(), h.value.as_str().to_owned()))
                    .collect();
                seen.lock()
                    .unwrap()
                    .push((request.url().to_owned(), headers));
                let header =
                    |name: &str, value: &str| tiny_http::Header::from_bytes(name, value).unwrap();
                let small = |status: u16, text: &str| {
                    tiny_http::Response::from_string(text)
                        .with_status_code(status)
                        .boxed()
                };
                let response = match (request.url(), *mode.lock().unwrap()) {
                    (FEED, _) => small(301, "")
                        .with_header(header("Location", MOVED))
                        .boxed(),
                    (MOVED, Answer::List) => tiny_http::Response::new(
                        tiny_http::StatusCode(200),
                        vec![header("Content-Type", "application/json; charset=utf-8")],
                        std::io::Cursor::new(body.clone().into_bytes()),
                        None,
                        None,
                    )
                    .boxed(),
                    (MOVED, Answer::NotFound) => small(404, r#"{"message":"Not Found"}"#),
                    (MOVED, Answer::Limited) => {
                        let reset = (store::now_ms() / 1000 + 600).to_string();
                        small(403, r#"{"message":"API rate limit exceeded"}"#)
                            .with_header(header("X-RateLimit-Remaining", "0"))
                            .with_header(header("X-RateLimit-Reset", &reset))
                            .boxed()
                    }
                    (MOVED, Answer::Garbage) => tiny_http::Response::new(
                        tiny_http::StatusCode(200),
                        vec![],
                        std::io::Cursor::new(b"<html>busy</html>".to_vec()),
                        None,
                        None,
                    )
                    .boxed(),
                    _ => small(404, "not here"),
                };
                let _ = request.respond(response);
            }
        });
        Self {
            origin,
            answer,
            requests,
            stop,
            thread: Some(thread),
        }
    }
    fn answer(&self, answer: Answer) {
        *self.answer.lock().unwrap() = answer;
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
impl Drop for FakeGitHub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn status(service: &WinPoint) -> Value {
    service
        .call("winpoint.status", &json!({}))
        .unwrap()
        .unwrap()
}
/// `winpoint.check` and the end of the check it started (`state.json` written twice, check.lock free).
fn check_and_wait(caller: &WinPoint) -> Value {
    let before = status(caller)["rev"].as_u64().unwrap();
    let answer = caller.call("winpoint.check", &json!({})).unwrap().unwrap();
    assert_eq!(answer, json!({"started":true}));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let now = status(caller);
        if now["rev"].as_u64().unwrap() >= before + 2 && now["state"] != "checking" {
            return now;
        }
        assert!(Instant::now() < deadline, "the check did not end: {now}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
/// Move the last check back a minute, past the 30 s that a manual check waits.
fn age(owner: &WinPoint) {
    let shared = owner.shared().unwrap();
    let mut state = shared.store.read(check::STATE).unwrap();
    state["checkedAt"] = json!(store::now_ms() - 60_000);
    shared.write_state(state);
}
/// What this PC is offered from the fixture (it depends on `uname -m`).
fn expected(origin: &url::Url, current: &str) -> feed::Release {
    let all = feed::parse_releases(
        releases_at(origin.as_str()).as_bytes(),
        &Origin::test(origin),
        &feed::machine(),
    )
    .unwrap();
    feed::choose(&all, &v(current)).unwrap().clone()
}

#[test]
fn a_check_from_settings_finds_the_release_through_a_redirect_and_chunked_json() {
    let root = Temp::new("e2e");
    let github = FakeGitHub::start();
    let seam = Seam::for_test(github.origin.clone(), v("0.9.0"));
    let panel = host(&root.0, "panel", false, "en", json!({}));
    let settings = host(&root.0, "settings", false, "en", json!({}));
    let owner = WinPoint::with(panel.clone(), Ok(seam.clone()), notify::Target::Off);
    let reader = WinPoint::with(settings.clone(), Ok(seam), notify::Target::Off);
    assert!(owner.take_over());
    let revs = Arc::new(Mutex::new(Vec::new()));
    let seen = revs.clone();
    settings.events.on(
        "winpoint.changed",
        Arc::new(move |data| seen.lock().unwrap().push(data["rev"].clone())),
    );
    assert_eq!(status(&reader)["state"], "unknown");
    let done = check_and_wait(&reader);
    let want = expected(&github.origin, "0.9.0");
    assert_eq!(done["state"], "available", "{done}");
    assert_eq!(done["latest"]["version"], want.version.to_string());
    assert_eq!(done["latest"]["notesUrl"], want.notes_url);
    assert_eq!(done["latest"]["name"], want.name);
    assert_eq!(done["latest"]["publishedAt"], json!(want.published_at));
    assert_eq!(
        done["latest"]["size"],
        json!(want.assets.as_ref().map(|a| a.tarball.size))
    );
    assert!(
        done["latest"]["notesUrl"]
            .as_str()
            .unwrap()
            .starts_with(github.origin.as_str())
    );
    assert_eq!(
        (done["current"].clone(), done["ownerRunning"].clone()),
        (json!("0.9.0"), json!(true))
    );
    assert!(
        done["checkedAt"].as_u64().is_some()
            && done["nextCheckAt"].as_u64() > done["checkedAt"].as_u64()
    );
    assert_eq!(
        status(&owner)["latest"],
        done["latest"],
        "every host reads the same files"
    );
    // the GET went through GitHub's 301 for a moved repository, with WinPoint's headers
    let requests = github.requests.lock().unwrap().clone();
    assert_eq!(
        requests.iter().map(|(u, _)| u.as_str()).collect::<Vec<_>>(),
        [FEED, MOVED]
    );
    for (_, headers) in &requests {
        let value = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        assert_eq!(
            value("user-agent"),
            Some(format!(
                "WinPoint/{} (+https://github.com/roboburr/WinPoint)",
                crate::VERSION
            ))
        );
        assert_eq!(
            value("accept").as_deref(),
            Some("application/vnd.github+json")
        );
        assert_eq!(value("x-github-api-version").as_deref(), Some("2022-11-28"));
        assert_eq!(value("connection").as_deref(), Some("close"));
    }
    // the state folder is private and the manual check only recorded the version (no notification)
    let dir = root.0.join("cache/winpoint");
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(dir.join("state.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["notified"], want.version.to_string());
    assert!(
        owner
            .shared()
            .unwrap()
            .notifier
            .shown
            .lock()
            .unwrap()
            .is_empty()
    );
    // the pages heard of the start and of the end
    let deadline = Instant::now() + Duration::from_secs(5);
    while revs.lock().unwrap().len() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        revs.lock().unwrap().len() >= 2,
        "{:?}",
        revs.lock().unwrap()
    );
    assert_eq!(revs.lock().unwrap().last(), Some(&done["rev"]));
    // again at once: `recent`, without a request
    let count = github.count();
    assert_eq!(
        reader.call("winpoint.check", &json!({})).unwrap().unwrap()["skipped"],
        "recent"
    );
    assert_eq!(github.count(), count);
    owner.shutdown();
    for ctx in [panel, settings] {
        ctx.events.shutdown();
    }
}

#[test]
fn an_automatic_check_notifies_once_per_version_and_skips_a_metered_or_offline_link() {
    let root = Temp::new("auto");
    let github = FakeGitHub::start();
    let panel = host(&root.0, "panel", false, "nl", json!({}));
    let owner = WinPoint::with(
        panel.clone(),
        Ok(Seam::for_test(github.origin.clone(), v("0.9.0"))),
        notify::Target::Off,
    );
    assert!(owner.take_over());
    let shared = owner.shared().unwrap().clone();
    *shared.net.lock().unwrap() = Some(Net::from_nm(Some(4), Some(4)));
    check::run(&shared, check::Trigger::Auto);
    let want = expected(&github.origin, "0.9.0").version.to_string();
    let shown = shared.notifier.shown.lock().unwrap().clone();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title, format!("WinPoint {want} is beschikbaar"));
    assert_eq!(
        shown[0].body,
        "Open Instellingen › Update en herstel om hem te installeren."
    );
    assert_eq!(shown[0].action, "Bekijken");
    check::run(&shared, check::Trigger::Auto);
    assert_eq!(
        shared.notifier.shown.lock().unwrap().len(),
        1,
        "once per version"
    );
    assert_eq!(github.count(), 4, "two checks, each through the 301");
    for net in [
        Net::from_nm(Some(4), Some(1)),
        Net::from_nm(Some(1), None),
        Net::from_nm(Some(2), None),
    ] {
        *shared.net.lock().unwrap() = Some(net);
        let before = shared.store.read(check::STATE);
        check::run(&shared, check::Trigger::Auto);
        assert_eq!(github.count(), 4, "{net:?}: no request");
        assert_eq!(shared.store.read(check::STATE), before, "{net:?}: no trace");
    }
    owner.shutdown();
    panel.events.shutdown();
}

#[test]
fn with_automatic_checks_off_nothing_is_due_and_nothing_notifies() {
    let root = Temp::new("off");
    let github = FakeGitHub::start();
    let panel = host(
        &root.0,
        "panel",
        false,
        "en",
        json!({"winpointAutoCheck": false}),
    );
    let owner = WinPoint::with(
        panel.clone(),
        Ok(Seam::for_test(github.origin.clone(), v("0.9.0"))),
        notify::Target::Off,
    );
    assert!(owner.take_over());
    let shared = owner.shared().unwrap().clone();
    assert!(!shared.auto_check());
    assert_eq!(status(&owner)["autoCheck"], false);
    assert!(!super::schedule::due(
        store::now_ms(),
        &json!({}),
        shared.auto_check(),
        false
    ));
    *shared.net.lock().unwrap() = Some(Net::from_nm(Some(4), None));
    check::run(&shared, check::Trigger::Auto);
    assert!(shared.notifier.shown.lock().unwrap().is_empty());
    assert_eq!(
        shared.store.read(check::STATE).unwrap()["notified"],
        expected(&github.origin, "0.9.0").version.to_string()
    );
    // the button still works
    age(&owner);
    assert_eq!(check_and_wait(&owner)["state"], "available");
    owner.shutdown();
    panel.events.shutdown();
}

#[test]
fn the_card_follows_github_from_no_release_to_unreachable_rate_limited_and_unreadable() {
    let root = Temp::new("card");
    let github = FakeGitHub::start();
    let panel = host(&root.0, "panel", false, "en", json!({}));
    let owner = WinPoint::with(
        panel.clone(),
        Ok(Seam::for_test(github.origin.clone(), v("0.9.0"))),
        notify::Target::Off,
    );
    assert!(owner.take_over());
    github.answer(Answer::NotFound);
    let none = check_and_wait(&owner);
    assert_eq!(
        (
            none["state"].clone(),
            none["result"].clone(),
            none["latest"].clone()
        ),
        (json!("upToDate"), json!("noReleases"), Value::Null)
    );
    age(&owner);
    github.answer(Answer::List);
    let found = check_and_wait(&owner);
    assert_eq!(found["state"], "available");
    // the repository made private again (as on 13-11): unreachable, the release found before stays below
    age(&owner);
    github.answer(Answer::NotFound);
    let gone = check_and_wait(&owner);
    assert_eq!(gone["state"], "unreachable");
    assert_eq!(
        gone["error"]["message"],
        "WinPoint’s releases can’t be found on GitHub"
    );
    assert_eq!(gone["latest"], found["latest"]);
    age(&owner);
    github.answer(Answer::Limited);
    let limited = check_and_wait(&owner);
    assert_eq!(limited["state"], "rateLimited");
    let retry = limited["retryAfter"].as_u64().unwrap();
    assert!(retry >= store::now_ms() + 3_500_000, "at least an hour");
    let message = limited["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("GitHub asks WinPoint to wait until "),
        "{message}"
    );
    age(&owner);
    let count = github.count();
    assert_eq!(
        owner.call("winpoint.check", &json!({})).unwrap().unwrap(),
        json!({"started":false,"skipped":"rateLimited","retryAfter":retry})
    );
    assert_eq!(github.count(), count, "not before GitHub allows it");
    let shared = owner.shared().unwrap();
    let mut state = shared.store.read(check::STATE).unwrap();
    state["retryAfter"] = Value::Null;
    shared.write_state(state);
    github.answer(Answer::Garbage);
    let unreadable = check_and_wait(&owner);
    assert_eq!(unreadable["state"], "checkFailed");
    assert_eq!(unreadable["error"]["key"], "update.winpoint.err.feed");
    assert_eq!(
        unreadable["error"]["message"],
        "GitHub’s answer couldn’t be read"
    );
    assert!(!unreadable["error"]["detail"].as_str().unwrap().is_empty());
    owner.shutdown();
    panel.events.shutdown();
}

#[test]
fn no_server_is_a_connect_error_on_the_card() {
    let root = Temp::new("down");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let origin = url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
    let panel = host(&root.0, "panel", false, "nl", json!({}));
    let owner = WinPoint::with(
        panel.clone(),
        Ok(Seam::for_test(origin, v("0.9.0"))),
        notify::Target::Off,
    );
    assert!(owner.take_over());
    let down = check_and_wait(&owner);
    assert_eq!(down["state"], "checkFailed");
    assert_eq!(down["error"]["key"], "update.winpoint.err.connect");
    assert_eq!(down["error"]["message"], "Er is geen verbinding met GitHub");
    assert_eq!(
        down["nextCheckAt"].as_u64(),
        down["checkedAt"].as_u64().map(|at| at + 6 * 3_600_000)
    );
    owner.shutdown();
    panel.events.shutdown();
}

/// A `state.json` that is valid JSON but no object (`[]`, `0`, a string: a hand edit, a damaged file) used to panic
/// every check on `state["rev"] = …`, so the file never healed: now the check runs, writes an object and tells the
/// pages, and every reader treats such a file as no state at all.
#[test]
fn a_state_file_that_is_no_object_heals_on_the_next_check() {
    let root = Temp::new("heal");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let origin = url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
    let panel = host(&root.0, "panel", false, "en", json!({}));
    let owner = WinPoint::with(
        panel.clone(),
        Ok(Seam::for_test(origin, v("0.9.0"))),
        notify::Target::Off,
    );
    assert!(owner.take_over());
    let revs = Arc::new(Mutex::new(Vec::new()));
    let seen = revs.clone();
    panel.events.on(
        "winpoint.changed",
        Arc::new(move |data| seen.lock().unwrap().push(data["rev"].clone())),
    );
    let shared = owner.shared().unwrap();
    let file = root.0.join("cache/winpoint/state.json");
    for (n, damaged) in ["[]", "0", "\"x\"", "null", "[{\"rev\":9}]"]
        .into_iter()
        .enumerate()
    {
        std::fs::write(&file, damaged).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(status(&owner)["state"], "unknown", "{damaged}");
        check::run(&shared, check::Trigger::Manual);
        let state: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert!(state.is_object(), "{damaged}: {state}");
        assert_eq!(
            (state["schema"].clone(), state["result"].clone()),
            (json!(1), json!("error")),
            "{damaged}"
        );
        assert!(
            state["rev"].as_u64().is_some_and(|rev| rev >= 1),
            "{damaged}"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while revs.lock().unwrap().len() < 2 * (n + 1) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            revs.lock().unwrap().last(),
            Some(&state["rev"]),
            "{damaged}: winpoint.changed for the healed file"
        );
        assert_eq!(status(&owner)["state"], "checkFailed");
    }
    // a writer handed no object writes one
    shared.write_state(json!([1, 2]));
    assert!(shared.read_state()["rev"].as_u64().is_some());
    owner.shutdown();
    panel.events.shutdown();
}

// ------------------------------------------------------------------------------------------------ the update job

/// The fake release of the job tests: version 9.9.9, a `winpoint-9.9.9/` tree with a fake scripts/install.py that
/// speaks D-G74's frozen contract (design 5.4; the real one is tested in tests/installer). What it does comes from
/// `fake.json` in the bundle, since its environment is exactly the frozen one.
const FAKE_INSTALL: &str = r#"import json, os, shutil, signal, sys, time
root = sys.argv[1]
args = sys.argv[2:]
fake = json.load(open(os.path.join(root, "fake.json")))
if "--check" in args:
    answer = fake.get("check") or {"ok": True, "version": "9.9.9", "extensionChanged": False, "systemFiles": []}
    if fake.get("checkHang"):
        time.sleep(fake["checkHang"])
    print("a line before the answer")
    print(json.dumps(answer))
    sys.exit(0 if answer.get("ok") else 1)
status = args[args.index("--status") + 1]
log = args[args.index("--log") + 1]
fd = os.open(log, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
os.dup2(fd, 1)
os.dup2(fd, 2)
code = 1
try:
    prefix = os.environ["WIN11_INSTALL_PREFIX"]
    print("installing into", prefix)
    seen = {"argv": sys.argv, "env": dict(os.environ),
            "flags": [sys.flags.ignore_environment, sys.flags.no_user_site, sys.flags.dont_write_bytecode],
            "link": os.readlink(os.path.join(root, "icons/go-top.svg"))}
    with open(os.path.join(prefix, "seen.json"), "w") as f:
        json.dump(seen, f)
    if fake.get("ignoreTerm"):
        # a rollback that hangs: SIGTERM is noted and changes nothing
        signal.signal(signal.SIGTERM, lambda *_: open(os.path.join(prefix, "term"), "a").write("term\n"))
    if fake.get("hang"):
        time.sleep(fake["hang"])
    code = fake.get("exit", 0)
    # "payload": the new payload was swapped in (its VERSION written) before the installer stopped
    if code == 0 or fake.get("payload"):
        with open(os.path.join(prefix, "share/win11-gnome/VERSION"), "w") as f:
            f.write(fake.get("version", "9.9.9") + "\n")
    if code == 0:
        os.makedirs(os.path.join(prefix, "bin"), exist_ok=True)
        shutil.copy(os.path.join(root, "bin/win11-panel"), os.path.join(prefix, "bin/win11-panel"))
finally:
    if not fake.get("noStatus"):
        with open(status + ".tmp", "w") as f:
            f.write("%d\n" % code)
        os.replace(status + ".tmp", status)
sys.exit(code)
"#;

const V: &str = "9.9.9";
fn tarball_name() -> String {
    feed::tarball_name(&v(V), &feed::machine())
}
/// The release tarball: `winpoint-9.9.9/` with its version in Cargo.toml, metadata.json and bin/win11-panel's
/// marker, an icon link `./go-up.svg` (kept byte for byte), the fake installer and its `fake.json`.
fn release_tarball(root: &std::path::Path, fake: &Value) -> Vec<u8> {
    let tree = root.join(format!("tree-{}", crate::events::nonce()));
    let top = tree.join(format!("winpoint-{V}"));
    for dir in ["bin", "scripts", "extension", "icons"] {
        std::fs::create_dir_all(top.join(dir)).unwrap();
    }
    std::fs::write(
        top.join("Cargo.toml"),
        format!("[workspace]\nmembers = [\"host\"]\n\n[workspace.package]\nversion = \"{V}\"\n"),
    )
    .unwrap();
    std::fs::write(
        top.join("extension/metadata.json"),
        format!(r#"{{"uuid":"w@x","version-name":"{V}","shell-version":["50"]}}"#),
    )
    .unwrap();
    std::fs::write(
        top.join("bin/win11-panel"),
        format!("\x7fELF...WinPoint {V}\n..."),
    )
    .unwrap();
    std::fs::write(top.join("scripts/install.py"), FAKE_INSTALL).unwrap();
    std::fs::write(top.join("fake.json"), fake.to_string()).unwrap();
    std::fs::write(top.join("icons/go-up.svg"), "<svg/>").unwrap();
    std::os::unix::fs::symlink("./go-up.svg", top.join("icons/go-top.svg")).unwrap();
    let out = tree.join("out.tar.gz");
    let packed = std::process::Command::new("/usr/bin/tar")
        .arg("-czf")
        .arg(&out)
        .arg("-C")
        .arg(&tree)
        .arg(format!("winpoint-{V}"))
        .status()
        .unwrap();
    assert!(packed.success());
    let bytes = std::fs::read(&out).unwrap();
    std::fs::remove_dir_all(&tree).unwrap();
    bytes
}
fn sha256(bytes: &[u8]) -> String {
    let mut hasher = super::verify::Hasher::new();
    hasher.update(bytes);
    super::verify::to_hex(&hasher.finish())
}

/// What the fake GitHub of the job tests serves now.
struct Serve {
    list: String,
    sha256: Vec<u8>,
    tarball: Vec<u8>,
    /// the CDN sends this many bytes, then nothing (a stalled download)
    stall_after: Option<usize>,
}
/// GitHub for one release: the list (chunked, no length), the `.sha256`, and the tarball behind a 302 to another
/// path (the CDN), which answers with a Content-Length. Every request is answered on a thread of its own.
struct Releases {
    origin: url::Url,
    serve: Arc<Mutex<Serve>>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Releases {
    fn start() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let origin = url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        let serve = Arc::new(Mutex::new(Serve {
            list: "[]".into(),
            sha256: Vec::new(),
            tarball: Vec::new(),
            stall_after: None,
        }));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (now, seen, stopping) = (serve.clone(), requests.clone(), stop.clone());
        let download = format!(
            "/roboburr/WinPoint/releases/download/v{V}/{}",
            tarball_name()
        );
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let request = match server.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(request)) => request,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                let url = request.url().to_owned();
                seen.lock().unwrap().push(url.clone());
                let serve = now.lock().unwrap();
                let header =
                    |name: &str, value: &str| tiny_http::Header::from_bytes(name, value).unwrap();
                let body = |status: u16, bytes: Vec<u8>, length: bool| {
                    let size = bytes.len();
                    tiny_http::Response::new(
                        tiny_http::StatusCode(status),
                        vec![],
                        std::io::Cursor::new(bytes),
                        length.then_some(size),
                        None,
                    )
                    .boxed()
                };
                let response = if url == FEED {
                    body(200, serve.list.clone().into_bytes(), false)
                } else if url == download {
                    tiny_http::Response::from_string("")
                        .with_status_code(302)
                        .with_header(header("Location", "/cdn/blob?id=1"))
                        .boxed()
                } else if url == "/cdn/blob?id=1" {
                    match serve.stall_after {
                        None => body(200, serve.tarball.clone(), true),
                        Some(n) => tiny_http::Response::new(
                            tiny_http::StatusCode(200),
                            vec![],
                            Stalling(serve.tarball[..n].to_vec(), stopping.clone()),
                            Some(serve.tarball.len()),
                            None,
                        )
                        .boxed(),
                    }
                } else if url == format!("{download}.sha256") {
                    body(200, serve.sha256.clone(), true)
                } else {
                    body(404, b"not here".to_vec(), true)
                };
                drop(serve);
                std::thread::spawn(move || {
                    let _ = request.respond(response);
                });
            }
        });
        Self {
            origin,
            serve,
            requests,
            stop,
            thread: Some(thread),
        }
    }
    /// Serve `tarball` as release 9.9.9, its `.sha256` saying `sha` (None: the true hash) and GitHub's digest
    /// saying `digest` (None: no digest field; Some(None): the true hash).
    fn publish(&self, tarball: &[u8], sha: Option<&str>, digest: Option<Option<&str>>) {
        let name = tarball_name();
        let truth = sha256(tarball);
        let sha_file = format!("{}  {name}\n", sha.unwrap_or(&truth));
        let digest = match digest {
            None => Value::Null,
            Some(d) => json!(format!("sha256:{}", d.unwrap_or(&truth))),
        };
        let web = self.origin.as_str();
        let base = format!("{web}roboburr/WinPoint/releases/download/v{V}/");
        let list = json!([{
            "tag_name": format!("v{V}"), "draft": false, "prerelease": false, "name": format!("WinPoint {V}"),
            "html_url": format!("{web}roboburr/WinPoint/releases/tag/v{V}"), "published_at": "2026-11-20T10:00:00Z",
            "assets": [
                {"name": name, "size": tarball.len(), "state": "uploaded",
                 "browser_download_url": format!("{base}{name}"), "digest": digest},
                {"name": format!("{name}.sha256"), "size": sha_file.len(), "state": "uploaded",
                 "browser_download_url": format!("{base}{name}.sha256"), "digest": null},
            ],
        }]);
        let mut serve = self.serve.lock().unwrap();
        serve.list = list.to_string();
        serve.sha256 = sha_file.into_bytes();
        serve.tarball = tarball.to_vec();
    }
    fn seen(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
    fn count(&self, what: &str) -> usize {
        self.seen().iter().filter(|u| u.contains(what)).count()
    }
}
impl Drop for Releases {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
/// A body that sends its bytes, then blocks until the server stops.
struct Stalling(Vec<u8>, Arc<AtomicBool>);
impl std::io::Read for Stalling {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.0.is_empty() {
            let n = buf.len().min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0.drain(..n);
            return Ok(n);
        }
        while !self.1.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(std::io::Error::other("stopped"))
    }
}

/// How the installer's start ends ([`Children`]): as it should, or with an error after the installer started (the
/// MainPID poll of a transient unit timed out), or with an error before anything started.
const STARTED: u8 = 0;
const STARTED_BUT_ERR: u8 = 1;
const NOT_STARTED: u8 = 2;

/// The job's two programs as plain children with exactly the given environment, counted.
#[derive(Default)]
struct Children {
    spawned: std::sync::atomic::AtomicUsize,
    checked: std::sync::atomic::AtomicUsize,
    start: std::sync::atomic::AtomicU8,
}
impl job::Spawner for Children {
    fn spawn(
        &self,
        mut command: std::process::Command,
        _tag: &str,
        env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    ) -> Result<u32, String> {
        self.spawned.fetch_add(1, Ordering::SeqCst);
        let start = self.start.load(Ordering::SeqCst);
        if start == NOT_STARTED {
            return Err("the user manager refused the unit".into());
        }
        command
            .env_clear()
            .envs(env)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let pid = child.id();
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        if start == STARTED_BUT_ERR {
            return Err("The program did not start in time".into());
        }
        Ok(pid)
    }
    fn output(
        &self,
        command: std::process::Command,
        env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
        log: std::fs::File,
        timeout: Duration,
        stop: &AtomicBool,
    ) -> Result<(Option<i32>, Vec<u8>), String> {
        self.checked.fetch_add(1, Ordering::SeqCst);
        job::run_child(command, env, log, timeout, stop)
    }
}
impl Children {
    fn counts(&self) -> (usize, usize) {
        (
            self.checked.load(Ordering::SeqCst),
            self.spawned.load(Ordering::SeqCst),
        )
    }
}

/// An owner (panel) and a Settings reader on the fake GitHub, both running from `prefix` (an installed payload of
/// version 9.9.8), with counted children and an online, unmetered link.
struct Rig {
    root: Temp,
    github: Releases,
    panel: Arc<crate::services::ServiceContext>,
    settings: Arc<crate::services::ServiceContext>,
    owner: Arc<WinPoint>,
    reader: Arc<WinPoint>,
    children: Arc<Children>,
    prefix: std::path::PathBuf,
}
impl Rig {
    fn new(tag: &str) -> Self {
        let root = Temp::new(tag);
        let github = Releases::start();
        let prefix = root.0.join("prefix");
        std::fs::create_dir_all(prefix.join("share/win11-gnome")).unwrap();
        std::fs::write(
            prefix.join("share/win11-gnome/.win11-gnome-owned"),
            super::PAYLOAD_MAGIC,
        )
        .unwrap();
        std::fs::write(prefix.join("share/win11-gnome/VERSION"), "9.9.8\n").unwrap();
        let seam = Seam::for_test(github.origin.clone(), v("9.9.8"));
        let panel = host(&root.0, "panel", false, "en", json!({}));
        let settings = host(&root.0, "settings", false, "en", json!({}));
        let owner = WinPoint::with(panel.clone(), Ok(seam.clone()), notify::Target::Off);
        let reader = WinPoint::with(settings.clone(), Ok(seam), notify::Target::Off);
        let children = Arc::new(Children::default());
        for service in [&owner, &reader] {
            let shared = service.shared().unwrap();
            let mut hooks = shared.hooks.lock().unwrap();
            hooks.prefix = Some(prefix.clone());
            hooks.spawner = Some(children.clone());
            drop(hooks);
            *shared.net.lock().unwrap() = Some(Net::from_nm(Some(4), Some(4)));
        }
        assert!(owner.take_over());
        Self {
            root,
            github,
            panel,
            settings,
            owner,
            reader,
            children,
            prefix,
        }
    }
    /// Publish `fake`'s release and let a check find it.
    fn offer(&self, fake: &Value) -> Vec<u8> {
        let tarball = release_tarball(&self.root.0, fake);
        self.github.publish(&tarball, None, Some(None));
        self.check();
        tarball
    }
    fn check(&self) -> Value {
        // past the 30 s within which a check answers `recent` (a job's own request counts as a check too)
        let ago = store::now_ms() - 60_000;
        self.owner
            .shared()
            .unwrap()
            .update_state(|s| s["checkedAt"] = json!(ago));
        check_and_wait(&self.reader)
    }
    fn work(&self) -> std::path::PathBuf {
        self.root.0.join("cache/winpoint/work").join(V)
    }
    /// The owner again, on `seam` (the last one shut down first), with the rig's prefix, children and link.
    fn new_owner(&mut self, seam: Seam) {
        self.new_owner_with(seam, |_| {});
    }
    /// [`Rig::new_owner`] with `change` made to its hooks before it takes over (its watch of an installer reads them).
    fn new_owner_with(&mut self, seam: Seam, change: impl FnOnce(&mut job::Hooks)) {
        self.owner.shutdown();
        let owner = WinPoint::with(self.panel.clone(), Ok(seam), notify::Target::Off);
        let shared = owner.shared().unwrap();
        let mut hooks = shared.hooks.lock().unwrap();
        hooks.prefix = Some(self.prefix.clone());
        hooks.spawner = Some(self.children.clone());
        change(&mut hooks);
        drop(hooks);
        *shared.net.lock().unwrap() = Some(Net::from_nm(Some(4), Some(4)));
        assert!(owner.take_over());
        self.owner = owner;
    }
    /// What job.json says now.
    fn record(&self) -> Value {
        self.owner
            .shared()
            .unwrap()
            .store
            .read(job::JOB)
            .unwrap_or(Value::Null)
    }
    /// Install from Settings (forwarded to the owner) and wait until the job has ended.
    fn install(&self) -> Value {
        let answer = self
            .reader
            .call("winpoint.install", &json!({"version": V}))
            .unwrap()
            .unwrap();
        assert!(
            answer["jobId"].as_str().is_some_and(store::is_token),
            "{answer}"
        );
        self.wait(answer["jobId"].as_str().unwrap())
    }
    fn wait(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let now = status(&self.reader);
            let ended = self.owner.shared().unwrap().job.lock().unwrap().is_none()
                && !matches!(
                    now["state"].as_str(),
                    Some("downloading" | "verifying" | "unpacking" | "preparing" | "installing")
                );
            if ended {
                let record = self
                    .owner
                    .shared()
                    .unwrap()
                    .store
                    .read(job::JOB)
                    .unwrap_or(Value::Null);
                assert!(record.is_null() || record["jobId"] == id, "{record}");
                return status(&self.reader);
            }
            assert!(Instant::now() < deadline, "the job did not end: {now}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.owner.shutdown();
        for ctx in [&self.panel, &self.settings] {
            ctx.events.shutdown();
        }
    }
}

#[test]
fn download_and_install_from_settings_ends_installed_with_the_frozen_argv_and_environment() {
    let rig = Rig::new("job-ok");
    let progress = Arc::new(Mutex::new(Vec::new()));
    let seen = progress.clone();
    rig.settings.events.on(
        "winpoint.progress",
        Arc::new(move |data| seen.lock().unwrap().push(data.clone())),
    );
    let tarball = rig.offer(&json!({}));
    let offered = status(&rig.reader);
    assert_eq!(offered["state"], "available", "{offered}");
    assert_eq!(
        (
            offered["latest"]["canInstall"].clone(),
            offered["latest"]["reason"].clone(),
            offered["latest"]["size"].clone()
        ),
        (json!(true), Value::Null, json!(tarball.len()))
    );
    let before = rig.github.seen().len();
    let done = rig.install();
    assert_eq!(done["state"], "installed", "{done}");
    let job = &done["job"];
    assert_eq!(
        (
            job["version"].clone(),
            job["restart"].clone(),
            job["outcome"].clone(),
            job["error"].clone()
        ),
        (json!(V), json!("restart"), Value::Null, Value::Null)
    );
    assert_eq!(
        (job["canCancel"].clone(), job["canRetry"].clone()),
        (json!(false), json!(false))
    );
    assert!(
        job["logTail"].as_str().unwrap().contains("installing into"),
        "{job}"
    );
    // GitHub asked again first, then the .sha256, then the tarball through its 302
    let requests: Vec<String> = rig.github.seen()[before..].to_vec();
    let name = tarball_name();
    assert_eq!(
        requests,
        [
            FEED.to_owned(),
            format!("/roboburr/WinPoint/releases/download/v{V}/{name}.sha256"),
            format!("/roboburr/WinPoint/releases/download/v{V}/{name}"),
            "/cdn/blob?id=1".to_owned(),
        ]
    );
    // the payload says 9.9.9; the tarball and the bundle are gone, the log stays until the job is cleared
    assert_eq!(
        std::fs::read_to_string(rig.prefix.join("share/win11-gnome/VERSION")).unwrap(),
        "9.9.9\n"
    );
    assert!(!rig.work().join(&name).exists() && !rig.work().join("bundle").exists());
    assert!(rig.work().join("install.log").exists());
    // the frozen contract: argv, -E -s -B, exactly the environment, the link target kept byte for byte
    let seen: Value =
        serde_json::from_slice(&std::fs::read(rig.prefix.join("seen.json")).unwrap()).unwrap();
    let bundle = rig.work().join("bundle");
    let argv: Vec<String> = seen["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        argv,
        [
            bundle.join("scripts/install.py").display().to_string(),
            bundle.display().to_string(),
            "--lang".into(),
            "en".into(),
            "--no-session".into(),
            "--status".into(),
            rig.work().join("install.status").display().to_string(),
            "--log".into(),
            rig.work().join("install.log").display().to_string(),
        ]
    );
    assert_eq!(seen["flags"], json!([1, 1, 1]), "-E -s -B");
    assert_eq!(seen["link"], "./go-up.svg");
    let mut names: Vec<&str> = seen["env"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "HOME",
            "LANG",
            "LOGNAME",
            "PATH",
            "USER",
            "WIN11_BUILD_DIR",
            "WIN11_INSTALL_PREFIX",
            "WIN11_NO_SESSION",
            "XDG_CACHE_HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_RUNTIME_DIR",
        ]
    );
    let env = &seen["env"];
    assert_eq!(
        (
            env["PATH"].clone(),
            env["LANG"].clone(),
            env["WIN11_NO_SESSION"].clone()
        ),
        (json!("/usr/bin:/bin"), json!("C.UTF-8"), json!("1"))
    );
    assert_eq!(
        env["WIN11_BUILD_DIR"],
        bundle.join("bin").display().to_string()
    );
    assert_eq!(
        env["WIN11_INSTALL_PREFIX"],
        rig.prefix.display().to_string()
    );
    // one --check and one install, the installed notification, progress for the pages
    assert_eq!(rig.children.counts(), (1, 1));
    let shown = rig
        .owner
        .shared()
        .unwrap()
        .notifier
        .shown
        .lock()
        .unwrap()
        .clone();
    let installed = shown.last().unwrap();
    assert_eq!(installed.title, "WinPoint 9.9.9 is installed");
    assert_eq!(installed.body, "Restart WinPoint to finish the update.");
    let deadline = Instant::now() + Duration::from_secs(5);
    while progress.lock().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let first = progress.lock().unwrap()[0].clone();
    assert_eq!(
        (
            first["jobId"].clone(),
            first["state"].clone(),
            first["total"].clone()
        ),
        (
            job["jobId"].clone(),
            json!("downloading"),
            json!(tarball.len())
        )
    );
    // the guards: no job runs any more, and a restart (not a sign-out) finishes it
    let cache = rig.root.0.join("cache");
    assert!(!job::job_active(&cache) && !job::installing(&cache) && !job::relogin_pending(&cache));
    assert!(crate::services::update::guard::restart_all_blocked(&rig.panel).is_none());
    // the restarted owner runs 9.9.9: the job is cleared with its work folder
    rig.owner.shutdown();
    let next = WinPoint::with(
        rig.panel.clone(),
        Ok(Seam::for_test(rig.github.origin.clone(), v("9.9.8"))),
        notify::Target::Off,
    );
    {
        let shared = next.shared().unwrap();
        let mut hooks = shared.hooks.lock().unwrap();
        hooks.prefix = Some(rig.prefix.clone());
        hooks.own_version = Some(v("9.9.8"));
    }
    assert!(next.take_over());
    assert_eq!(
        status(&next)["state"],
        "installed",
        "an owner of the old version"
    );
    next.shutdown();
    let last = WinPoint::with(
        rig.panel.clone(),
        Ok(Seam::for_test(rig.github.origin.clone(), v("9.9.9"))),
        notify::Target::Off,
    );
    last.shared().unwrap().hooks.lock().unwrap().own_version = Some(v(V));
    assert!(last.take_over());
    let cleared = status(&last);
    assert_eq!(
        (cleared["state"].clone(), cleared["job"].clone()),
        (json!("upToDate"), Value::Null)
    );
    assert!(!rig.root.0.join("cache/winpoint/job.json").exists());
    assert!(!rig.root.0.join("cache/winpoint/work").exists());
    last.shutdown();
}

#[test]
fn a_tarball_that_is_not_githubs_is_deleted_and_nothing_installs() {
    let rig = Rig::new("job-tamper");
    let mut tarball = rig.offer(&json!({}));
    // GitHub's digest and the .sha256 describe the true file; the CDN sends one byte changed
    let truth = sha256(&tarball);
    let last = tarball.len() - 1;
    tarball[last] ^= 0x55;
    assert_ne!(sha256(&tarball), truth);
    rig.github.serve.lock().unwrap().tarball = tarball.clone();
    let done = rig.install();
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(done["job"]["error"]["code"], "checksum");
    assert_eq!(
        done["job"]["error"]["message"],
        "The download is damaged and was deleted"
    );
    assert_eq!(done["job"]["outcome"], "nothingChanged");
    assert_eq!(done["job"]["outcomeMessage"], "Nothing was changed.");
    assert_eq!(done["job"]["canRetry"], true);
    let name = tarball_name();
    assert!(!rig.work().join(&name).exists());
    assert!(!rig.work().join(format!("{name}.part")).exists());
    assert!(!rig.work().join("bundle").exists());
    assert_eq!(rig.children.counts(), (0, 0), "nothing ran");
    assert_eq!(
        std::fs::read_to_string(rig.prefix.join("share/win11-gnome/VERSION")).unwrap(),
        "9.9.8\n"
    );
}

#[test]
fn a_sha256_file_that_disagrees_with_githubs_digest_stops_before_the_tarball() {
    let rig = Rig::new("job-sha");
    let tarball = release_tarball(&rig.root.0, &json!({}));
    rig.github
        .publish(&tarball, Some(&"ab".repeat(32)), Some(None));
    rig.check();
    let done = rig.install();
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(done["job"]["error"]["code"], "mismatch");
    assert_eq!(
        done["job"]["error"]["message"],
        "The release’s checksum file doesn’t match GitHub; nothing was downloaded"
    );
    assert_eq!(rig.github.count("/cdn/"), 0, "no tarball request");
    assert_eq!(rig.github.count(&tarball_name()), 1, "only the .sha256");
    // a .sha256 for another file name is the same failure
    rig.github.publish(&tarball, None, Some(None));
    {
        let mut serve = rig.github.serve.lock().unwrap();
        serve.sha256 = format!("{}  other.tar.gz\n", sha256(&tarball)).into_bytes();
    }
    rig.check();
    assert_eq!(rig.install()["job"]["error"]["code"], "mismatch");
    assert_eq!(rig.github.count("/cdn/"), 0);
    assert_eq!(rig.children.counts(), (0, 0));
}

#[test]
fn a_release_without_githubs_digest_is_never_downloaded() {
    let rig = Rig::new("job-nodigest");
    let tarball = release_tarball(&rig.root.0, &json!({}));
    rig.github.publish(&tarball, None, None);
    rig.check();
    let offered = status(&rig.reader);
    assert_eq!(offered["state"], "available");
    assert_eq!(
        (
            offered["latest"]["reason"].clone(),
            offered["latest"]["canInstall"].clone()
        ),
        (json!("noDigest"), json!(false))
    );
    assert_eq!(
        offered["latest"]["message"],
        "This release has no checksum from GitHub; download it from the link"
    );
    let before = rig.github.seen().len();
    let error = rig
        .reader
        .call("winpoint.install", &json!({"version": V}))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "noDigest");
    assert_eq!(rig.github.seen().len(), before, "not even the list again");
    // the card had a digest, GitHub's answer at the click has none: the job stops before any download
    rig.github.publish(&tarball, None, Some(None));
    rig.check();
    rig.github.publish(&tarball, None, None);
    let done = rig.install();
    assert_eq!(done["job"]["error"]["code"], "noDigest", "{done}");
    assert_eq!(done["job"]["canRetry"], false);
    assert_eq!(rig.github.count(".sha256"), 0);
    assert_eq!(rig.github.count("/cdn/"), 0);
    assert_eq!(
        status(&rig.reader)["latest"]["reason"],
        "noDigest",
        "the job's answer is the card's"
    );
    assert_eq!(rig.children.counts(), (0, 0));
}

#[test]
fn an_installer_that_rolls_back_is_failed_rolled_back_and_try_again_reuses_the_tarball() {
    let rig = Rig::new("job-rollback");
    rig.offer(&json!({"exit": 3}));
    let done = rig.install();
    assert_eq!(done["state"], "failed", "{done}");
    let job = &done["job"];
    assert_eq!(
        (
            job["error"]["code"].clone(),
            job["error"]["params"]["code"].clone(),
            job["outcome"].clone()
        ),
        (json!("installer"), json!("3"), json!("rolledBack"))
    );
    assert_eq!(job["error"]["message"], "The installer stopped (code 3)");
    assert_eq!(job["outcomeMessage"], "Your previous version was restored.");
    assert_eq!(job["canRetry"], true);
    assert_eq!(
        std::fs::read_to_string(rig.prefix.join("share/win11-gnome/VERSION")).unwrap(),
        "9.9.8\n"
    );
    // Try again: the verified tarball is used again (it still matches the fresh digest), unpacked fresh
    assert!(rig.work().join(tarball_name()).exists());
    let downloads = rig.github.count("/cdn/");
    let again = rig.install();
    assert_eq!(again["job"]["outcome"], "rolledBack");
    assert_eq!(rig.github.count("/cdn/"), downloads, "no second download");
    assert_eq!(rig.children.counts(), (2, 2));
    // exit 1: refused before any change; 4: an incomplete rollback, try again
    for (code, outcome, line) in [
        (1, "nothingChanged", "Nothing was changed."),
        (4, "unknown", "Try again to finish the installation."),
    ] {
        rig.offer(&json!({"exit": code}));
        let ended = rig.install();
        assert_eq!(ended["job"]["outcome"], outcome, "{ended}");
        assert_eq!(ended["job"]["outcomeMessage"], line);
    }
    // an installer that ends without a status (killed): interrupted, and Try again finishes it
    rig.offer(&json!({"noStatus": true, "exit": 0}));
    let cut = rig.install();
    assert_eq!(cut["state"], "interrupted", "{cut}");
    assert_eq!(cut["job"]["outcome"], "unknown");
    assert_eq!(cut["job"]["canRetry"], true);
    rig.offer(&json!({}));
    assert_eq!(rig.install()["state"], "installed");
}

#[test]
fn a_compatibility_refusal_blocks_the_version_without_another_download() {
    let rig = Rig::new("job-block");
    rig.offer(&json!({"check": {"ok": false, "code": "gnomeMismatch", "params": {"needs": "50", "running": "49"},
                                 "message": "GNOME 50 needed"}}));
    let done = rig.install();
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(done["job"]["error"]["code"], "incompatible");
    assert_eq!(
        done["job"]["error"]["message"],
        "WinPoint 9.9.9 is made for GNOME 50; this PC has GNOME 49"
    );
    assert_eq!(
        (
            done["job"]["outcome"].clone(),
            done["job"]["canRetry"].clone()
        ),
        (json!("nothingChanged"), json!(false))
    );
    assert_eq!(done["latest"]["reason"], "incompatible");
    assert_eq!(done["latest"]["canInstall"], false);
    assert_eq!(
        rig.children.counts(),
        (1, 0),
        "the check ran, the installer never"
    );
    let state: Value = serde_json::from_slice(
        &std::fs::read(rig.root.0.join("cache/winpoint/state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        (
            state["blocked"]["version"].clone(),
            state["blocked"]["code"].clone()
        ),
        (json!(V), json!("incompatible"))
    );
    // the next status and a click again: the reason, no download
    let downloads = rig.github.count("/cdn/");
    let error = rig
        .reader
        .call("winpoint.install", &json!({"version": V}))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "incompatible");
    assert_eq!(rig.github.count("/cdn/"), downloads);
    // an automatic check keeps the verdict for 9.9.9 (and announces nothing)
    let shared = rig.owner.shared().unwrap();
    super::check::run(&shared, super::check::Trigger::Auto);
    assert_eq!(status(&rig.reader)["latest"]["reason"], "incompatible");
    assert_eq!(shared.read_state()["blocked"]["code"], "incompatible");
    // "Check for updates" forgets a verdict about this PC (GNOME may be upgraded since): 9.9.9 is offered again, and
    // the click runs the release's --check afresh, which blocks it again. The verdict took its tarball along (a GNOME
    // or library upgrade takes long), so it is downloaded again
    assert!(!rig.work().join(tarball_name()).exists());
    let offered = rig.check();
    assert_eq!(
        (
            offered["latest"]["reason"].clone(),
            offered["latest"]["canInstall"].clone()
        ),
        (Value::Null, json!(true)),
        "{offered}"
    );
    let again = rig.install();
    assert_eq!(again["job"]["error"]["code"], "incompatible", "{again}");
    assert_eq!(
        rig.children.counts(),
        (2, 0),
        "--check again, the installer never"
    );
    assert_eq!(rig.github.count("/cdn/"), downloads + 1);
    // the other verdicts: missing parts, root-owned system files (manual), and a sign-out (extension changed). Each
    // offer() is a manual check, which forgets the verdict before it (incompatible, missingParts) by itself
    for (check, reason, message) in [
        (
            json!({"ok": false, "code": "missingParts", "params": {"packages": ["libgtk-4-1", "libadwaita-1-0"]},
                   "message": "x"}),
            "missingParts",
            "Install these first: sudo apt install libgtk-4-1 libadwaita-1-0",
        ),
        // install.py's own form: the parts as text and the packages it knows as text, here none ("")
        (
            json!({"ok": false, "code": "missingParts", "params": {"missing": "libfoo.so.1", "packages": ""},
                   "message": "x"}),
            "missingParts",
            "Install these first: sudo apt install libfoo.so.1",
        ),
        (
            json!({"ok": true, "version": V, "extensionChanged": false, "systemFiles": ["session"]}),
            "manual",
            "This version also changes system files. Download it from GitHub and install it with scripts/install.sh.",
        ),
    ] {
        rig.offer(&json!({"check": check}));
        let ended = rig.install();
        assert_eq!(ended["job"]["error"]["code"], reason, "{ended}");
        assert_eq!(ended["latest"]["message"], message);
        // a verdict that can go (missing parts) keeps the verified tarball; one of the release itself takes it along
        assert_eq!(
            rig.work().join(tarball_name()).exists(),
            reason == "missingParts",
            "{reason}"
        );
    }
    // `manual` is the release's own: a check keeps it, only another version (here: by hand) clears it
    assert_eq!(rig.check()["latest"]["reason"], "manual");
    rig.owner
        .shared()
        .unwrap()
        .update_state(|s| s["blocked"] = Value::Null);
    rig.offer(
        &json!({"check": {"ok": true, "version": V, "extensionChanged": true, "systemFiles": []}}),
    );
    let relogin = rig.install();
    assert_eq!(
        (relogin["state"].clone(), relogin["job"]["restart"].clone()),
        (json!("installed"), json!("relogin")),
        "{relogin}"
    );
    let shown = rig
        .owner
        .shared()
        .unwrap()
        .notifier
        .shown
        .lock()
        .unwrap()
        .clone();
    assert_eq!(
        shown.last().unwrap().body,
        "Sign out and back in to finish the update."
    );
    // until the sign-out: no restart of WinPoint (new programs against the old extension)
    let cache = rig.root.0.join("cache");
    assert!(job::relogin_pending(&cache));
    let refused = crate::services::update::guard::restart_all_blocked(&rig.panel).unwrap();
    assert_eq!(
        refused.key.as_deref(),
        Some("update.winpoint.err.signOutFirst")
    );
    assert_eq!(
        refused.message,
        "Sign out first to finish the WinPoint update"
    );
    // a new owner in the same session keeps it; after a sign-out (another session key) it is cleared
    rig.owner.shutdown();
    let next = WinPoint::with(
        rig.panel.clone(),
        Ok(Seam::for_test(rig.github.origin.clone(), v("9.9.9"))),
        notify::Target::Off,
    );
    next.shared().unwrap().hooks.lock().unwrap().own_version = Some(v(V));
    assert!(next.take_over());
    assert!(job::relogin_pending(&cache), "same session");
    next.shutdown();
    let signed_in = WinPoint::with(
        rig.panel.clone(),
        Ok(Seam::for_test(rig.github.origin.clone(), v("9.9.9"))),
        notify::Target::Off,
    );
    signed_in
        .shared()
        .unwrap()
        .hooks
        .lock()
        .unwrap()
        .session_key = Some(Some("another".into()));
    assert!(signed_in.take_over());
    assert!(!job::relogin_pending(&cache));
    assert!(!rig.root.0.join("cache/winpoint/job.json").exists());
    signed_in.shutdown();
}

#[test]
fn cancel_ends_a_download_and_removes_its_part_file() {
    let rig = Rig::new("job-cancel");
    let tarball = rig.offer(&json!({}));
    rig.github.serve.lock().unwrap().stall_after = Some(tarball.len() / 2);
    let answer = rig
        .reader
        .call("winpoint.install", &json!({"version": V}))
        .unwrap()
        .unwrap();
    let id = answer["jobId"].as_str().unwrap().to_owned();
    let part = rig.work().join(format!("{}.part", tarball_name()));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !part.exists() || rig.github.count("/cdn/") == 0 {
        assert!(Instant::now() < deadline, "no download");
        std::thread::sleep(Duration::from_millis(20));
    }
    let running = status(&rig.reader);
    assert_eq!(running["state"], "downloading");
    assert_eq!(running["job"]["canCancel"], true);
    // a second install, another id, the installer's own: busy, no_job
    assert_eq!(
        rig.reader
            .call("winpoint.install", &json!({"version": V}))
            .unwrap()
            .unwrap_err()
            .code,
        "busy"
    );
    assert_eq!(
        rig.reader
            .call(
                "winpoint.cancel",
                &json!({"jobId": "0123456789abcdef0123456789abcdef"})
            )
            .unwrap()
            .unwrap_err()
            .code,
        "no_job"
    );
    let started = Instant::now();
    assert_eq!(
        rig.reader
            .call("winpoint.cancel", &json!({"jobId": id}))
            .unwrap()
            .unwrap(),
        json!({"ok":true})
    );
    let done = rig.wait(&id);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        done["state"], "available",
        "a cancelled job is no state of the card: {done}"
    );
    let record = rig.owner.shared().unwrap().store.read(job::JOB).unwrap();
    assert_eq!(
        (record["state"].clone(), record["outcome"].clone()),
        (json!("cancelled"), json!("nothingChanged"))
    );
    assert!(!part.exists());
    assert_eq!(rig.children.counts(), (0, 0));
}

#[test]
fn the_preconditions_of_an_install_each_say_what_stops_it() {
    let rig = Rig::new("job-pre");
    rig.offer(&json!({}));
    let install = |args: Value| {
        rig.owner
            .call("winpoint.install", &args)
            .unwrap()
            .unwrap_err()
    };
    // not the version offered
    assert_eq!(install(json!({"version":"9.9.7"})).code, "stale");
    assert_eq!(
        install(json!({"version":"9.9.7"})).message,
        "This version is no longer offered; check again"
    );
    // a metered link: confirmed by the page (metered:true) or the setting; offline
    let shared = rig.owner.shared().unwrap();
    *shared.net.lock().unwrap() = Some(Net::from_nm(Some(4), Some(1)));
    let metered = install(json!({"version": V}));
    assert_eq!(metered.code, "metered");
    assert_eq!(
        metered.data.as_ref().unwrap()["size"].as_u64().unwrap() > 0,
        true
    );
    *shared.net.lock().unwrap() = Some(Net::from_nm(Some(1), None));
    assert_eq!(install(json!({"version": V})).code, "offline");
    // a restart of every WinPoint process on its way: busy
    *shared.net.lock().unwrap() = Some(Net::from_nm(Some(4), Some(1)));
    rig.panel.emit(
        "shell.restartAll",
        json!({"nonce":"x","origin":1,"at":1,"reason":"update"}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while shared.restart_seen.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(install(json!({"version": V, "metered": true})).code, "busy");
    shared.restart_seen.store(0, Ordering::SeqCst);
    // a development run (no installed payload): dev, with the folder it runs from
    shared.hooks.lock().unwrap().prefix = None;
    let dev = install(json!({"version": V}));
    assert_eq!(dev.code, "dev");
    assert!(
        dev.message.starts_with("This WinPoint runs from "),
        "{}",
        dev.message
    );
    assert_eq!(status(&rig.owner)["latest"]["reason"], "dev");
    assert_eq!(rig.children.counts(), (0, 0));
    assert!(!rig.root.0.join("cache/winpoint/job.json").exists());
    assert_eq!(job::size_text(crate::i18n::Lang::Nl, 71_234_567), "71,2 MB");
    assert_eq!(
        job::size_text(crate::i18n::Lang::En, 1_234_567_890),
        "1.2 GB"
    );
}

/// D-G74: no check (manual or automatic), timer, status read or reattach starts the installer's Python.
#[test]
fn no_check_timer_status_or_reattach_starts_a_program() {
    let rig = Rig::new("job-none");
    rig.offer(&json!({}));
    let shared = rig.owner.shared().unwrap();
    rig.check();
    super::check::run(&shared, super::check::Trigger::Auto);
    rig.owner.tick();
    for _ in 0..3 {
        status(&rig.owner);
        status(&rig.reader);
    }
    // what a last owner can leave: each ends or is watched, nothing is started; a step's half-written files go
    let dead = json!({"pid": 999_999_999u32, "start": "1"});
    std::fs::create_dir_all(rig.work()).unwrap();
    let part = rig.work().join(format!("{}.part", tarball_name()));
    let partial = rig.work().join("bundle.partial");
    for (state, installer, status_file, ends) in [
        ("downloading", Value::Null, None, "failed"),
        ("preparing", Value::Null, None, "failed"),
        ("installing", dead.clone(), Some("0\n"), "installed"),
        ("installing", dead.clone(), Some("3\n"), "failed"),
        ("installing", dead.clone(), None, "interrupted"),
    ] {
        let _ = std::fs::remove_file(rig.work().join("install.status"));
        if let Some(text) = status_file {
            std::fs::write(rig.work().join("install.status"), text).unwrap();
        }
        std::fs::write(&part, "half a download").unwrap();
        std::fs::create_dir_all(partial.join("winpoint-9.9.9")).unwrap();
        std::fs::write(rig.prefix.join("share/win11-gnome/VERSION"), "9.9.9\n").unwrap();
        let mut record = json!({"schema":1,"jobId":"0123456789abcdef0123456789abcdef","version":V,"state":state,
                                "owner":dead,"installer":installer,"prefix":rig.prefix.display().to_string(),
                                "restart":"restart","lang":"en"});
        shared.write_job(&mut record);
        super::job::reattach(&shared, true);
        let after = shared.store.read(job::JOB).unwrap();
        assert_eq!(after["state"], ends, "{state} {status_file:?}: {after}");
        if state != "installing" {
            assert!(
                !part.exists() && !partial.exists(),
                "{state}: the dead owner's .part and bundle.partial"
            );
        }
    }
    assert_eq!(rig.children.counts(), (0, 0), "no program at all");
}

/// Reattach to an installer that still runs (the owner restarted while it installs): it is watched to its end.
#[test]
fn a_new_owner_watches_an_installer_that_still_runs() {
    let rig = Rig::new("job-watch");
    rig.offer(&json!({"hang": 3}));
    let answer = rig
        .reader
        .call("winpoint.install", &json!({"version": V}))
        .unwrap()
        .unwrap();
    let id = answer["jobId"].as_str().unwrap().to_owned();
    let deadline = Instant::now() + Duration::from_secs(20);
    while status(&rig.reader)["state"] != "installing" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let cache = rig.root.0.join("cache");
    assert!(job::installing(&cache) && job::job_active(&cache));
    let power = crate::services::update::guard::power_blocked_in(
        &rig.panel,
        &crate::services::update::paths::resolve(&Default::default()).unwrap(),
        crate::i18n::Lang::En,
    )
    .unwrap();
    assert_eq!(
        power.message,
        "WinPoint is being updated; wait until it’s done"
    );
    assert_eq!(
        rig.reader
            .call("winpoint.cancel", &json!({"jobId": id}))
            .unwrap()
            .unwrap_err()
            .code,
        "cannot_cancel"
    );
    // the owner goes (its watch ends with it); the installer goes on; the next owner watches it to the end
    rig.owner.shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    while rig.owner.shared().unwrap().job.lock().unwrap().is_some() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let next = WinPoint::with(
        rig.panel.clone(),
        Ok(Seam::for_test(rig.github.origin.clone(), v("9.9.8"))),
        notify::Target::Off,
    );
    next.shared().unwrap().hooks.lock().unwrap().prefix = Some(rig.prefix.clone());
    assert!(next.take_over());
    assert!(
        next.shared().unwrap().job.lock().unwrap().is_some(),
        "a watcher"
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let now = status(&next);
        if now["state"] == "installed" {
            break;
        }
        assert!(Instant::now() < deadline, "{now}");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        rig.children.counts(),
        (1, 1),
        "the new owner started nothing"
    );
    next.shutdown();
}

/// D-G74, test mode: whoever listens on the seam's loopback port can serve a list, a digest, a `.sha256` and a
/// tarball that all agree; the job installs only the tarball seam.json pins, and asks for nothing past the list
/// otherwise.
#[test]
fn a_test_seam_installs_only_the_tarball_it_pins() {
    let mut rig = Rig::new("job-pin");
    let tarball = rig.offer(&json!({}));
    let origin = rig.github.origin.clone();
    rig.new_owner(Seam::pinned(origin.clone(), v("9.9.8"), [7u8; 32]));
    rig.check();
    let before = rig.github.seen().len();
    let done = rig.install();
    assert_eq!(
        (
            done["state"].clone(),
            done["job"]["error"]["code"].clone(),
            done["job"]["outcome"].clone()
        ),
        (json!("failed"), json!("seam"), json!("nothingChanged")),
        "{done}"
    );
    assert_eq!(
        rig.github.seen()[before..],
        [FEED.to_owned()],
        "the list only: no .sha256, no tarball"
    );
    assert_eq!(rig.children.counts(), (0, 0), "nothing ran");
    // the tarball the seam pins installs
    let mut hasher = super::verify::Hasher::new();
    hasher.update(&tarball);
    rig.new_owner(Seam::pinned(origin, v("9.9.8"), hasher.finish()));
    rig.check();
    assert_eq!(rig.install()["state"], "installed");
}

/// Cancel while the release's `--check` runs: it is killed at once, and a verdict it would have given blocks nothing.
#[test]
fn cancel_stops_the_compatibility_check_and_blocks_nothing() {
    let rig = Rig::new("job-cancel-check");
    rig.offer(&json!({"checkHang": 30, "check": {"ok": false, "code": "missingParts",
                                                  "params": {"packages": "libfoo1"}, "message": "x"}}));
    let answer = rig
        .reader
        .call("winpoint.install", &json!({"version": V}))
        .unwrap()
        .unwrap();
    let id = answer["jobId"].as_str().unwrap().to_owned();
    let deadline = Instant::now() + Duration::from_secs(20);
    while rig.children.counts().0 == 0 || status(&rig.reader)["state"] != "preparing" {
        assert!(Instant::now() < deadline, "no --check");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(status(&rig.reader)["job"]["canCancel"], true);
    let started = Instant::now();
    assert_eq!(
        rig.reader
            .call("winpoint.cancel", &json!({"jobId": id}))
            .unwrap()
            .unwrap(),
        json!({"ok":true})
    );
    let done = rig.wait(&id);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(done["state"], "available", "{done}");
    let record = rig.record();
    assert_eq!(
        (record["state"].clone(), record["outcome"].clone()),
        (json!("cancelled"), json!("nothingChanged"))
    );
    assert_eq!(
        rig.owner.shared().unwrap().read_state()["blocked"],
        Value::Null
    );
    assert!(!rig.work().join("bundle").exists());
    assert_eq!(rig.children.counts(), (1, 0));
}

/// An install that stopped after install.py swapped the new payload in (no status, or 4: the rollback was incomplete)
/// leaves the payload saying V. Where this host's version follows the payload (a test's own version, not the seam's
/// `running`), V still counts as not installed: the card keeps the job with Try again, and Try again finishes it. So
/// does a host that runs V's binaries already (install.py swapped those too, and a sign-out started them): it compares
/// with the version the job started from.
#[test]
fn an_unfinished_install_whose_payload_says_v_is_still_offered_and_retried() {
    let rig = Rig::new("job-unconfirmed");
    let own = |version: &str| {
        for service in [&rig.owner, &rig.reader] {
            service.shared().unwrap().hooks.lock().unwrap().own_version = Some(v(version));
        }
    };
    own("9.9.8");
    let version = rig.prefix.join("share/win11-gnome/VERSION");
    rig.offer(&json!({"noStatus": true, "exit": 0}));
    let cut = rig.install();
    assert_eq!(std::fs::read_to_string(&version).unwrap(), "9.9.9\n");
    assert_eq!(
        (
            cut["state"].clone(),
            cut["current"].clone(),
            cut["job"]["canRetry"].clone()
        ),
        (json!("interrupted"), json!("9.9.8"), json!(true)),
        "{cut}"
    );
    assert_eq!(cut["latest"]["version"], V, "V is still offered");
    assert_eq!(
        (rig.record()["from"].clone(), rig.record()["found"].clone()),
        (json!("9.9.8"), json!("9.9.8"))
    );
    // from here on WinPoint runs V's binaries (own == V): the job, its Try again and V's offer all stay
    own(V);
    let now = status(&rig.reader);
    assert_eq!(
        (
            now["state"].clone(),
            now["current"].clone(),
            now["job"]["canRetry"].clone(),
            now["latest"]["version"].clone()
        ),
        (json!("interrupted"), json!("9.9.8"), json!(true), json!(V)),
        "{now}"
    );
    // a Try again that the installer rolls back (to the unfinished payload it found: VERSION stays V) changed
    // nothing, and keeps all of it as well: its click found the payload saying V already
    rig.offer(&json!({"exit": 3}));
    let back = rig.install();
    assert_eq!(rig.record()["found"], V);
    assert_eq!(
        (
            back["state"].clone(),
            back["current"].clone(),
            back["job"]["outcome"].clone(),
            back["job"]["canRetry"].clone()
        ),
        (
            json!("failed"),
            json!("9.9.8"),
            json!("rolledBack"),
            json!(true)
        ),
        "{back}"
    );
    rig.offer(&json!({"exit": 4, "payload": true}));
    let failed = rig.install();
    assert_eq!(
        (
            failed["state"].clone(),
            failed["job"]["outcome"].clone(),
            failed["job"]["canRetry"].clone()
        ),
        (json!("failed"), json!("unknown"), json!(true)),
        "{failed}"
    );
    assert_eq!(
        failed["job"]["outcomeMessage"],
        "Try again to finish the installation."
    );
    // Try again (the same version, which the payload names already) runs install.py again, which finishes it this
    // time (the fake's release now installs; the real install.py rolls forward)
    rig.github
        .publish(&release_tarball(&rig.root.0, &json!({})), None, Some(None));
    let done = rig.install();
    assert_eq!(done["state"], "installed", "{done}");
    assert_eq!(done["current"], V, "confirmed now");
    assert_eq!(rig.children.counts(), (4, 4));
}

/// A rollback that put the payload back to U but could not put V's binaries back (install.py exits 4) leaves a host
/// that, once restarted, runs V over a payload that says U. Its Try again is still one of V (job.json `found`: this
/// host said V at the click), not `stale`, and it finishes the install.
#[test]
fn a_try_again_after_a_rollback_that_left_vs_binaries_is_not_stale() {
    let rig = Rig::new("job-binaries-left");
    let own = |version: &str| {
        for service in [&rig.owner, &rig.reader] {
            service.shared().unwrap().hooks.lock().unwrap().own_version = Some(v(version));
        }
    };
    own("9.9.8");
    let version = rig.prefix.join("share/win11-gnome/VERSION");
    rig.offer(&json!({"exit": 4}));
    let cut = rig.install();
    assert_eq!(std::fs::read_to_string(&version).unwrap(), "9.9.8\n");
    assert_eq!(
        (
            cut["state"].clone(),
            cut["job"]["outcome"].clone(),
            cut["job"]["canRetry"].clone()
        ),
        (json!("failed"), json!("unknown"), json!(true)),
        "{cut}"
    );
    // WinPoint restarts on V's binaries, which the rollback left: the job, its Try again and V's offer stay
    own(V);
    let now = status(&rig.reader);
    assert_eq!(
        (
            now["state"].clone(),
            now["current"].clone(),
            now["job"]["canRetry"].clone(),
            now["latest"]["version"].clone()
        ),
        (json!("failed"), json!("9.9.8"), json!(true), json!(V)),
        "{now}"
    );
    rig.github
        .publish(&release_tarball(&rig.root.0, &json!({})), None, Some(None));
    let done = rig.install();
    assert_eq!(rig.record()["found"], V);
    assert_eq!(
        (done["state"].clone(), done["current"].clone()),
        (json!("installed"), json!(V)),
        "{done}"
    );
    assert_eq!(rig.children.counts(), (2, 2));
}

/// The same rollback, but Try again comes while WinPoint still runs U and ends without installing (install.py rolls
/// it back to the payload it found): the chain stays one of V (`found` comes from the record it replaced), so once
/// WinPoint runs the V binaries left behind, V is still offered and Try again finishes it.
#[test]
fn a_try_again_chain_after_a_rollback_that_left_vs_binaries_survives_a_failed_retry() {
    let rig = Rig::new("job-binaries-retry");
    let own = |version: &str| {
        for service in [&rig.owner, &rig.reader] {
            service.shared().unwrap().hooks.lock().unwrap().own_version = Some(v(version));
        }
    };
    own("9.9.8");
    rig.offer(&json!({"exit": 4}));
    let cut = rig.install();
    assert_eq!(cut["job"]["outcome"], "unknown", "{cut}");
    rig.offer(&json!({"exit": 3}));
    let back = rig.install();
    assert_eq!(
        (
            back["state"].clone(),
            back["job"]["outcome"].clone(),
            rig.record()["found"].clone()
        ),
        (json!("failed"), json!("rolledBack"), json!(V)),
        "{back}"
    );
    own(V);
    let now = status(&rig.reader);
    assert_eq!(
        (now["current"].clone(), now["latest"]["version"].clone()),
        (json!("9.9.8"), json!(V)),
        "{now}"
    );
    rig.github
        .publish(&release_tarball(&rig.root.0, &json!({})), None, Some(None));
    let done = rig.install();
    assert_eq!(
        (done["state"].clone(), done["current"].clone()),
        (json!("installed"), json!(V)),
        "{done}"
    );
    assert_eq!(rig.children.counts(), (3, 3));
}

/// A job that changed nothing of the payload it found does not keep V from counting once V was installed by hand.
/// Here --check finds system files to change (`manual`), and the card says to install V with scripts/install.sh;
/// once the payload says V and WinPoint runs it (install.sh restarts it), the card is up to date, though job.json
/// still holds the failed job and state.json the verdict.
#[test]
fn a_version_installed_by_hand_after_a_job_that_changed_nothing_is_the_installed_version() {
    let rig = Rig::new("job-by-hand");
    let own = |version: &str| {
        for service in [&rig.owner, &rig.reader] {
            service.shared().unwrap().hooks.lock().unwrap().own_version = Some(v(version));
        }
    };
    own("9.9.8");
    let check = json!({"ok":true,"version":V,"extensionChanged":false,"systemFiles":["session"]});
    rig.offer(&json!({ "check": check }));
    let blocked = rig.install();
    assert_eq!(
        (
            blocked["state"].clone(),
            blocked["job"]["error"]["code"].clone(),
            blocked["latest"]["reason"].clone(),
            blocked["current"].clone()
        ),
        (
            json!("failed"),
            json!("manual"),
            json!("manual"),
            json!("9.9.8")
        ),
        "{blocked}"
    );
    // scripts/install.sh V
    std::fs::write(rig.prefix.join("share/win11-gnome/VERSION"), "9.9.9\n").unwrap();
    own(V);
    let now = status(&rig.reader);
    assert_eq!(
        (
            now["state"].clone(),
            now["current"].clone(),
            now["latest"].clone(),
            now["job"].clone()
        ),
        (json!("upToDate"), json!(V), Value::Null, Value::Null),
        "{now}"
    );
    assert_eq!(
        (rig.record()["state"].clone(), rig.record()["found"].clone()),
        (json!("failed"), json!("9.9.8"))
    );
    assert_eq!(rig.check()["state"], "upToDate");
    assert_eq!(rig.children.counts(), (1, 0));
}

/// SIGTERM once the installer has run 15 minutes (here 3 s), SIGKILL some minutes (here 2 s) after that SIGTERM for a
/// rollback that hangs. The clock is the installer's, so a new owner that takes over the watch between the two does
/// not start it again; one that takes over past both (an owner that died early in the install, the PC suspended
/// meanwhile) still sends SIGTERM first, which rolls install.py back, and SIGKILL only 2 s after its own SIGTERM.
#[test]
fn a_hung_installer_is_stopped_on_its_own_clock_also_after_a_new_owner_took_over() {
    let mut rig = Rig::new("job-timeout");
    let origin = rig.github.origin.clone();
    for (round, takeover) in [3500, 6000].into_iter().enumerate() {
        {
            let shared = rig.owner.shared().unwrap();
            let mut hooks = shared.hooks.lock().unwrap();
            hooks.install_timeout = Some(Duration::from_secs(3));
            hooks.kill_after = Some(Duration::from_secs(60));
        }
        let _ = std::fs::remove_file(rig.prefix.join("seen.json"));
        let _ = std::fs::remove_file(rig.prefix.join("term"));
        rig.offer(&json!({"hang": 60, "ignoreTerm": true}));
        rig.reader
            .call("winpoint.install", &json!({"version": V}))
            .unwrap()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !rig.prefix.join("seen.json").exists() {
            assert!(Instant::now() < deadline, "no installer");
            std::thread::sleep(Duration::from_millis(20));
        }
        let started = Instant::now();
        let installer = rig.record()["installer"].clone();
        assert!(store::owner_alive(&installer), "{installer}");
        // the owner goes before the 3 s; the installer goes on
        rig.owner.shutdown();
        let deadline = Instant::now() + Duration::from_secs(5);
        while rig.owner.shared().unwrap().job.lock().unwrap().is_some() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!rig.prefix.join("term").exists(), "no SIGTERM before 3 s");
        std::thread::sleep(Duration::from_millis(takeover).saturating_sub(started.elapsed()));
        rig.new_owner_with(Seam::for_test(origin.clone(), v("9.9.8")), |hooks| {
            hooks.install_timeout = Some(Duration::from_secs(3));
            hooks.kill_after = Some(Duration::from_secs(2));
        });
        let took = Instant::now();
        while !rig.prefix.join("term").exists() {
            assert!(
                store::owner_alive(&installer),
                "round {round}: SIGKILL without a SIGTERM first"
            );
            assert!(
                took.elapsed() < Duration::from_millis(2500),
                "round {round}: the new owner waited for a clock of its own"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let termed = Instant::now();
        // the SIGTERM changed nothing (a hung rollback): SIGKILL 2 s after it, then interrupted
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let now = status(&rig.owner);
            if now["state"] == "interrupted" {
                assert_eq!(
                    (
                        now["job"]["outcome"].clone(),
                        now["job"]["canRetry"].clone()
                    ),
                    (json!("unknown"), json!(true)),
                    "{now}"
                );
                break;
            }
            assert!(Instant::now() < deadline, "{now}");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            termed.elapsed() >= Duration::from_millis(1500),
            "round {round}: SIGKILL {:?} after the SIGTERM",
            termed.elapsed()
        );
        assert!(!store::owner_alive(&installer), "killed");
    }
    assert_eq!(
        rig.children.counts(),
        (2, 2),
        "the new owners started nothing"
    );
}

/// job.json says `installing` before the installer can exist. A start that reports an error after the installer
/// started (a transient unit's MainPID poll that timed out) is found by its argv and watched to its end; one that
/// failed before anything ran changed nothing (no log).
#[test]
fn an_installer_whose_start_failed_half_way_is_found_and_watched() {
    let rig = Rig::new("job-halfway");
    rig.children.start.store(STARTED_BUT_ERR, Ordering::SeqCst);
    rig.offer(&json!({"hang": 1}));
    let done = rig.install();
    assert_eq!(done["state"], "installed", "{done}");
    rig.children.start.store(NOT_STARTED, Ordering::SeqCst);
    rig.offer(&json!({}));
    let refused = rig.install();
    assert_eq!(
        (
            refused["state"].clone(),
            refused["job"]["error"]["code"].clone(),
            refused["job"]["outcome"].clone()
        ),
        (json!("failed"), json!("installer"), json!("nothingChanged")),
        "{refused}"
    );
    assert!(!rig.work().join("bundle").exists());
    assert_eq!(rig.children.counts(), (2, 2));
}

/// The last owner died while the installer started (job.json `installing`, no pid yet): the next owner, and every
/// reader meanwhile, find it by its frozen argv in /proc and watch it to its end. With none there, past the start's
/// grace, nothing changed unless its log says it ran. Nothing is started for this.
#[test]
fn a_new_owner_finds_an_installer_whose_pid_was_never_recorded() {
    let rig = Rig::new("job-nopid");
    let bundle = rig.work().join("bundle");
    std::fs::create_dir_all(bundle.join("scripts")).unwrap();
    let script = bundle.join("scripts/install.py");
    let version = rig.prefix.join("share/win11-gnome/VERSION");
    std::fs::write(
        &script,
        format!(
            "import sys, time\ntime.sleep(1.5)\nopen({:?}, 'w').write('9.9.9\\n')\n\
             open(sys.argv[sys.argv.index('--status') + 1], 'w').write('0\\n')\n",
            version.display().to_string()
        ),
    )
    .unwrap();
    let mut child = std::process::Command::new("/usr/bin/python3")
        .args(["-E", "-s", "-B"])
        .arg(&script)
        .arg(&bundle)
        .args(["--lang", "en", "--no-session", "--status"])
        .arg(rig.work().join("install.status"))
        .arg("--log")
        .arg(rig.work().join("install.log"))
        .spawn()
        .unwrap();
    let shared = rig.owner.shared().unwrap();
    let dead = json!({"pid": 999_999_999u32, "start": "1"});
    let old = store::now_ms() - 120_000;
    let mut record = json!({"schema":1,"jobId":"0123456789abcdef0123456789abcdef","version":V,"state":"installing",
                            "owner":dead,"installingAt":old,"restart":"restart","lang":"en",
                            "installer":{"pid":null,"start":null,"script":script.display().to_string()}});
    record["prefix"] = json!(rig.prefix.display().to_string());
    shared.write_job(&mut record);
    let deadline = Instant::now() + Duration::from_secs(5);
    while job::find_installer(&script.display().to_string()).is_none() {
        assert!(Instant::now() < deadline, "the stand-in did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    let cache = rig.root.0.join("cache");
    assert!(
        job::installing(&cache),
        "found by its argv: the power guard holds"
    );
    super::job::reattach(&shared, true);
    let deadline = Instant::now() + Duration::from_secs(20);
    while rig.record()["state"] != "installed" {
        assert!(Instant::now() < deadline, "{}", rig.record());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(rig.record()["installer"]["pid"], json!(child.id()));
    let _ = child.wait();
    // nobody runs it, and its start was long ago: no log, nothing changed; a log, it ran and stopped
    for (log, ends, outcome) in [
        (false, "failed", "nothingChanged"),
        (true, "interrupted", "unknown"),
    ] {
        let _ = std::fs::remove_file(rig.work().join("install.status"));
        let _ = std::fs::remove_file(rig.work().join("install.log"));
        if log {
            std::fs::write(rig.work().join("install.log"), "started\n").unwrap();
        }
        record["state"] = json!("installing");
        record["installer"] =
            json!({"pid":null,"start":null,"script":"/nowhere/scripts/install.py"});
        shared.write_job(&mut record);
        assert!(
            !job::installing(&cache),
            "nothing to find, long past its start"
        );
        super::job::reattach(&shared, true);
        let deadline = Instant::now() + Duration::from_secs(10);
        while rig.record()["state"] == "installing" {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        }
        let after = rig.record();
        assert_eq!(
            (after["state"].clone(), after["outcome"].clone()),
            (json!(ends), json!(outcome)),
            "log {log}: {after}"
        );
    }
    assert_eq!(rig.children.counts(), (0, 0), "nothing started");
}

/// `installed` waits for a restart that runs V; once another install took its place (the payload names another
/// version: a build installed by hand) the card no longer waits, and the next owner clears the record.
#[test]
fn an_installed_job_another_install_replaced_is_cleared() {
    let rig = Rig::new("job-replaced");
    rig.offer(&json!({}));
    assert_eq!(rig.install()["state"], "installed");
    std::fs::write(rig.prefix.join("share/win11-gnome/VERSION"), "9.10.0-dev\n").unwrap();
    assert_ne!(
        status(&rig.reader)["state"],
        "installed",
        "no restart to wait for"
    );
    rig.owner.shutdown();
    let next = WinPoint::with(
        rig.panel.clone(),
        Ok(Seam::for_test(rig.github.origin.clone(), v("9.9.8"))),
        notify::Target::Off,
    );
    {
        let shared = next.shared().unwrap();
        let mut hooks = shared.hooks.lock().unwrap();
        hooks.prefix = Some(rig.prefix.clone());
        hooks.own_version = Some(v("9.10.0-dev"));
    }
    assert!(next.take_over());
    assert!(!rig.root.0.join("cache/winpoint/job.json").exists());
    assert!(!rig.root.0.join("cache/winpoint/work").exists());
    next.shutdown();
}
