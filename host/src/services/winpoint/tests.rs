//! Unit tests of WinPoint's update check (D-G74a). No test contacts GitHub: the HTTP client talks to
//! std `TcpListener`s on 127.0.0.1, the TLS test to a self-signed server on localhost, and the notification test to
//! a fake notification server on the sandbox's private session bus (tests/rust-sandbox/run-tests.sh), never the
//! live one.
use super::{
    Mode, View, WinPoint,
    check::{self, Failure, Fetched, Trigger},
    check_args, compose,
    feed::{self, Origin},
    http::{self, Client, Framing, HttpError, Limits, Policy},
    notify, schedule,
    seam::{self, Seam},
    version::Version,
};
use crate::{
    i18n::Lang,
    services::{ServiceContext, update::net::Net},
};
use gio::prelude::*;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(super) const RELEASES: &str =
    include_str!("../../../../tests/update/fixtures/winpoint/releases.json");
const SELF_SIGNED: &str = include_str!("../../../../tests/update/fixtures/winpoint/selfsigned.pem");

pub(super) fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}
fn url(s: &str) -> url::Url {
    url::Url::parse(s).unwrap()
}

// ------------------------------------------------------------------------------------------------ fixtures

/// A temporary directory removed on drop.
pub(super) struct Temp(pub PathBuf);
impl Temp {
    pub(super) fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("wp-{tag}-{}", crate::events::nonce()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(dir)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A host's context: its own config (settings.json with `lang` and `extra` update keys), the shared cache and
/// event folder of `root` (the same user's other hosts).
pub(super) fn host(
    root: &Path,
    kind: &str,
    dry: bool,
    lang: &str,
    update: Value,
) -> Arc<ServiceContext> {
    let config = root.join(format!("config-{kind}-{}", crate::events::nonce()));
    std::fs::create_dir_all(&config).unwrap();
    let mut settings: Value =
        serde_json::from_str(include_str!("../../settings-defaults.json")).unwrap();
    settings["language"] = json!(lang);
    for (key, value) in update.as_object().cloned().unwrap_or_default() {
        settings["update"][key] = value;
    }
    std::fs::write(
        config.join("settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let events = crate::events::Events::new(&root.join("events")).unwrap();
    ServiceContext::new(
        events,
        config,
        root.join("cache"),
        root.to_path_buf(),
        kind.into(),
        dry,
    )
    .unwrap()
}

/// The feed fixture with its GitHub URLs moved to `origin` (what the fake server of a test serves).
pub(super) fn releases_at(origin: &str) -> String {
    RELEASES.replace("https://github.com/", origin)
}

/// A reader that gives one byte per read: the parsers must not depend on how the stream is cut.
struct OneByte<'a>(&'a [u8]);
impl Read for OneByte<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.0.is_empty() || buf.is_empty() {
            return Ok(0);
        }
        buf[0] = self.0[0];
        self.0 = &self.0[1..];
        Ok(1)
    }
}
fn one(bytes: &[u8]) -> BufReader<OneByte<'_>> {
    BufReader::with_capacity(1, OneByte(bytes))
}

// ------------------------------------------------------------------------------------------------ http parsers

#[test]
fn read_head_parses_a_head_and_keeps_its_limits() {
    let (status, headers) = http::read_head(&mut one(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nX-A:  b \t\r\n\r\nhi",
    ))
    .unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        headers,
        [
            ("Content-Length".into(), "2".into()),
            ("X-A".into(), "b".into())
        ]
    );
    // 1xx answers are skipped; a status line without a reason phrase is fine
    let (status, _) = http::read_head(&mut one(
        b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.0 204\r\n\r\n",
    ))
    .unwrap();
    assert_eq!(status, 204);
    let protocol = |bytes: &[u8]| match http::read_head(&mut one(bytes)) {
        Err(HttpError::Protocol(what)) => what,
        other => panic!("{:?}: {other:?}", String::from_utf8_lossy(bytes)),
    };
    assert_eq!(
        protocol(b"HTTP/1.1 200 OK\r\nA: b\r\n c\r\n\r\n"),
        "obs-fold"
    );
    assert_eq!(protocol(b"HTTP/1.1 200 OK\r\nA: b\n\r\n"), "bare LF");
    assert_eq!(protocol(b""), "no answer");
    assert_eq!(protocol(b"HTTP/1.1 200 OK\r\nA: b\r\n"), "truncated");
    for bad in [
        &b"HTTP/2 200 OK\r\n\r\n"[..],
        b"HTTP/1.1 20 OK\r\n\r\n",
        b"ICY 200 OK\r\n\r\n",
        b"HTTP/1.1 999 X\r\n\r\n",
        b"HTTP/1.1 200OK\r\n\r\n",
        b"HTTP/1.x 200 OK\r\n\r\n",
    ] {
        assert_eq!(protocol(bad), "status line");
    }
    assert_eq!(
        protocol(b"HTTP/1.1 200 OK\r\nBad Name: x\r\n\r\n"),
        "header name"
    );
    assert_eq!(protocol(b"HTTP/1.1 200 OK\r\n: x\r\n\r\n"), "header name");
    assert_eq!(protocol(b"HTTP/1.1 200 OK\r\nnocolon\r\n\r\n"), "header");
    let long = format!("HTTP/1.1 200 OK\r\nA: {}\r\n\r\n", "x".repeat(8 << 10));
    assert_eq!(protocol(long.as_bytes()), "line too long");
    let mut big = String::from("HTTP/1.1 200 OK\r\n");
    for i in 0..70 {
        big.push_str(&format!("H{i}: {}\r\n", "y".repeat(1000)));
    }
    assert_eq!(protocol(format!("{big}\r\n").as_bytes()), "head too large");
    let mut many = String::from("HTTP/1.1 200 OK\r\n");
    for i in 0..129 {
        many.push_str(&format!("H{i}: v\r\n"));
    }
    assert_eq!(
        protocol(format!("{many}\r\n").as_bytes()),
        "too many headers"
    );
    let mut ok = String::from("HTTP/1.1 200 OK\r\n");
    for i in 0..128 {
        ok.push_str(&format!("H{i}: v\r\n"));
    }
    assert_eq!(
        http::read_head(&mut one(format!("{ok}\r\n").as_bytes()))
            .unwrap()
            .1
            .len(),
        128
    );
}

#[test]
fn framing_is_the_strict_subset_and_refuses_a_body_without_a_length() {
    let h = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        http::framing(200, &h(&[("Content-Length", "5")])),
        Ok(Framing::Length(5))
    );
    assert_eq!(
        http::framing(200, &h(&[("content-length", "5, 5")])),
        Ok(Framing::Length(5))
    );
    assert_eq!(
        http::framing(200, &h(&[("Content-Length", "5"), ("Content-Length", "5")])),
        Ok(Framing::Length(5))
    );
    assert_eq!(
        http::framing(200, &h(&[("Transfer-Encoding", "chunked")])),
        Ok(Framing::Chunked)
    );
    assert_eq!(
        http::framing(200, &h(&[("transfer-encoding", "Chunked")])),
        Ok(Framing::Chunked)
    );
    for (status, headers) in [
        (204, h(&[("Content-Length", "5")])),
        (304, h(&[])),
        (101, h(&[])),
    ] {
        assert_eq!(http::framing(status, &headers), Ok(Framing::Empty));
    }
    let refused = [
        (h(&[]), "no length"),
        (
            h(&[("Content-Length", "5"), ("Content-Length", "6")]),
            "two lengths",
        ),
        (h(&[("Content-Length", "5, 6")]), "two lengths"),
        (h(&[("Content-Length", "-1")]), "content length"),
        (h(&[("Content-Length", "abc")]), "content length"),
        (h(&[("Content-Length", "")]), "content length"),
        (
            h(&[("Content-Length", "99999999999999999999")]),
            "content length",
        ),
        (
            h(&[("Transfer-Encoding", "chunked"), ("Content-Length", "5")]),
            "length and chunked",
        ),
        (
            h(&[("Transfer-Encoding", "gzip, chunked")]),
            "transfer coding",
        ),
        (
            h(&[
                ("Transfer-Encoding", "chunked"),
                ("Transfer-Encoding", "chunked"),
            ]),
            "transfer coding",
        ),
    ];
    for (headers, what) in refused {
        assert_eq!(
            http::framing(200, &headers),
            Err(HttpError::Protocol(what)),
            "{headers:?}"
        );
    }
}

fn chunked(bytes: &[u8], max: u64) -> Result<Vec<u8>, HttpError> {
    let mut sink = Vec::new();
    let n = http::copy_body(&mut one(bytes), Framing::Chunked, max, &mut sink)?;
    assert_eq!(n, sink.len() as u64);
    Ok(sink)
}

#[test]
fn the_chunk_decoder_takes_one_byte_at_a_time_and_refuses_every_malformed_chunk() {
    assert_eq!(
        chunked(
            b"4;ext=1\r\nWiki\r\n5\r\npedia\r\n0\r\nTrailer: x\r\n\r\n",
            100
        )
        .unwrap(),
        b"Wikipedia"
    );
    assert_eq!(
        chunked(b"A\r\n0123456789\r\n0\r\n\r\n", 100).unwrap(),
        b"0123456789"
    );
    assert_eq!(chunked(b"0\r\n\r\n", 0).unwrap(), b"");
    assert_eq!(
        chunked(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n", 9).unwrap(),
        b"Wikipedia"
    );
    let protocol = |bytes: &[u8]| match chunked(bytes, 1 << 20) {
        Err(HttpError::Protocol(what)) => what,
        other => panic!("{:?}: {other:?}", String::from_utf8_lossy(bytes)),
    };
    assert_eq!(protocol(b"G\r\nx\r\n0\r\n\r\n"), "chunk size");
    assert_eq!(protocol(b"\r\n"), "chunk size");
    assert_eq!(protocol(b";ext\r\n"), "chunk size");
    assert_eq!(
        protocol(b"00000000000000001\r\nx\r\n0\r\n\r\n"),
        "chunk size"
    );
    assert_eq!(protocol(b"4\r\nWikiX\r\n0\r\n\r\n"), "line too long");
    assert_eq!(protocol(b"4\r\nWiki\n0\r\n\r\n"), "bare LF");
    assert_eq!(protocol(b"4\r\nWi"), "truncated");
    assert_eq!(protocol(b"4\r\nWiki\r\n"), "truncated");
    assert_eq!(protocol(b"4\r\nWiki\r\n0\r\n"), "truncated");
    let trailers = format!("1\r\nx\r\n0\r\n{}\r\n", "T: v\r\n".repeat(2000));
    assert_eq!(protocol(trailers.as_bytes()), "trailers too large");
    // max applies across all chunks, also when a size would overflow
    assert_eq!(
        chunked(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n", 8),
        Err(HttpError::TooLarge)
    );
    assert_eq!(
        chunked(b"FFFFFFFFFFFFFFFF\r\n", 1 << 20),
        Err(HttpError::TooLarge)
    );
    assert_eq!(
        chunked(b"1\r\na\r\nFFFFFFFFFFFFFFFF\r\n", u64::MAX),
        Err(HttpError::TooLarge)
    );
}

#[test]
fn copy_body_with_a_length_stops_at_max_and_at_a_short_stream() {
    let body = |bytes: &[u8], framing, max| {
        let mut sink = Vec::new();
        http::copy_body(&mut one(bytes), framing, max, &mut sink).map(|_| sink)
    };
    assert_eq!(body(b"hello", Framing::Length(5), 5).unwrap(), b"hello");
    assert_eq!(
        body(b"hello world", Framing::Length(5), 5).unwrap(),
        b"hello",
        "the rest is not read"
    );
    assert_eq!(
        body(b"hello!", Framing::Length(6), 5),
        Err(HttpError::TooLarge),
        "max + 1"
    );
    assert_eq!(
        body(b"hel", Framing::Length(5), 5),
        Err(HttpError::Protocol("truncated"))
    );
    assert_eq!(body(b"ignored", Framing::Empty, 0).unwrap(), b"");
}

#[test]
fn redirects_stay_on_https_in_production_and_on_the_origin_in_a_test() {
    let base = url("https://api.github.com/repos/roboburr/WinPoint/releases?per_page=10");
    let prod = Policy::Production;
    assert_eq!(
        http::redirect_target(&base, "/repositories/1/releases?per_page=10", &prod)
            .unwrap()
            .as_str(),
        "https://api.github.com/repositories/1/releases?per_page=10"
    );
    assert_eq!(
        http::redirect_target(
            &base,
            "https://release-assets.githubusercontent.com/a?b=c#frag",
            &prod
        )
        .unwrap()
        .as_str(),
        "https://release-assets.githubusercontent.com/a?b=c",
        "any https host (GitHub does not document its download host), no fragment"
    );
    assert_eq!(
        http::redirect_target(&base, "//objects.example/x", &prod)
            .unwrap()
            .as_str(),
        "https://objects.example/x"
    );
    for bad in [
        "http://github.com/",
        "ftp://github.com/x",
        "https://user:pw@github.com/",
        "file:///etc/passwd",
    ] {
        assert_eq!(
            http::redirect_target(&base, bad, &prod),
            Err(HttpError::Policy),
            "{bad}"
        );
    }
    let origin = url("http://127.0.0.1:47811/");
    let test = Policy::Test {
        origin: origin.clone(),
    };
    assert_eq!(
        http::redirect_target(&origin.join("a/b").unwrap(), "../cdn?id=1", &test)
            .unwrap()
            .as_str(),
        "http://127.0.0.1:47811/cdn?id=1"
    );
    for bad in [
        "http://127.0.0.1:47812/",
        "https://127.0.0.1:47811/",
        "http://localhost:47811/",
        "https://github.com/",
    ] {
        assert_eq!(
            http::redirect_target(&origin, bad, &test),
            Err(HttpError::Policy),
            "{bad}"
        );
    }
}

#[test]
fn http_errors_have_fixed_texts_and_keep_the_raw_detail() {
    let keys = [
        (HttpError::NoTls, "update.winpoint.err.noTls"),
        (HttpError::Dns, "update.winpoint.err.dns"),
        (HttpError::Connect, "update.winpoint.err.connect"),
        (HttpError::Tls("x".into()), "update.winpoint.err.tls"),
        (HttpError::Policy, "update.winpoint.err.tls"),
        (HttpError::Proxy, "update.winpoint.err.proxy"),
        (HttpError::Timeout, "update.winpoint.err.timeout"),
        (HttpError::Cancelled, "update.winpoint.err.cancelled"),
        (HttpError::Protocol("x"), "update.winpoint.err.feed"),
        (HttpError::TooLarge, "update.winpoint.err.feed"),
        (HttpError::Status(500), "update.winpoint.err.status"),
        (HttpError::Io("x".into()), "update.winpoint.err.connect"),
    ];
    for (error, key) in keys {
        assert_eq!(error.key(), key, "{error:?}");
        assert!(!crate::i18n::tr(Lang::En, key).is_empty());
    }
    let refused = glib::Error::new(gio::IOErrorEnum::ConnectionRefused, "Verbinding geweigerd");
    assert_eq!(http::from_glib(&refused), HttpError::Connect);
    let timed = glib::Error::new(gio::IOErrorEnum::TimedOut, "x");
    assert_eq!(http::from_glib(&timed), HttpError::Timeout);
    let proxy = glib::Error::new(gio::IOErrorEnum::ProxyNeedAuth, "x");
    assert_eq!(http::from_glib(&proxy), HttpError::Proxy);
    let dns = glib::Error::new(gio::ResolverError::NotFound, "x");
    assert_eq!(http::from_glib(&dns), HttpError::Dns);
    let tls = glib::Error::new(
        gio::TlsError::BadCertificate,
        "Onaanvaardbaar TLS-certificaat",
    );
    assert_eq!(
        http::from_glib(&tls),
        HttpError::Tls("Onaanvaardbaar TLS-certificaat".into())
    );
    assert_eq!(
        HttpError::Tls("raw".into()).detail(),
        "raw",
        "the raw message only goes into the detail"
    );
}

// ------------------------------------------------------------------------------------------------ http client

/// A loopback server that serves `count` connections, one after the other: it records each request head and
/// hands the stream to `answer(index, head, stream)`.
pub(super) fn serve(
    count: usize,
    answer: impl Fn(usize, &str, &mut TcpStream) + Send + 'static,
) -> (
    url::Url,
    Arc<Mutex<Vec<String>>>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = url(&format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    ));
    let heads = Arc::new(Mutex::new(Vec::new()));
    let seen = heads.clone();
    let thread = std::thread::spawn(move || {
        for index in 0..count {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            seen.lock().unwrap().push(head.clone());
            answer(index, &head, &mut stream);
        }
    });
    (origin, heads, thread)
}
fn limits(stall: u64, deadline: Option<u64>) -> Limits {
    Limits {
        max_body: 1 << 20,
        stall: Duration::from_secs(stall),
        deadline: deadline.map(Duration::from_secs),
    }
}
fn get(
    client: &Client,
    target: &url::Url,
    limits: &Limits,
) -> Result<(http::Response, Vec<u8>), HttpError> {
    let mut body = Vec::new();
    client
        .get(
            target,
            check::ACCEPT,
            limits,
            &mut body,
            &AtomicBool::new(false),
        )
        .map(|r| (r, body))
}

#[test]
fn a_get_sends_the_github_headers_and_reads_a_body_with_a_length() {
    let (origin, heads, thread) = serve(1, |_, _, s| {
        let _ = s.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Ratelimit-Remaining: 59\r\n\r\nhello",
        );
    });
    let client = Client::for_test(origin.clone());
    let target = Origin::test(&origin).feed_url();
    let (response, body) = get(&client, &target, &limits(5, Some(10))).unwrap();
    thread.join().unwrap();
    assert_eq!((response.status, body.as_slice()), (200, &b"hello"[..]));
    assert_eq!(response.header("x-ratelimit-remaining"), Some("59"));
    assert_eq!(response.url, target);
    let head = heads.lock().unwrap()[0].clone();
    let port = origin.port().unwrap();
    for line in [
        "GET /repos/roboburr/WinPoint/releases?per_page=10 HTTP/1.1\r\n".to_owned(),
        format!("Host: 127.0.0.1:{port}\r\n"),
        format!(
            "User-Agent: WinPoint/{} (+https://github.com/roboburr/WinPoint)\r\n",
            crate::VERSION
        ),
        "Accept: application/vnd.github+json\r\n".into(),
        "Accept-Encoding: identity\r\n".into(),
        "Connection: close\r\n".into(),
        "X-GitHub-Api-Version: 2022-11-28\r\n".into(),
    ] {
        assert!(head.contains(&line), "{line:?} in {head:?}");
    }
}

#[test]
fn redirects_are_followed_five_times_and_no_more() {
    let hop = |redirects: usize| {
        move |i: usize, _: &str, s: &mut TcpStream| {
            if i < redirects {
                let location = format!("/hop{}?n={}", i + 1, i + 1);
                let _ = s.write_all(
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
                    )
                    .as_bytes(),
                );
            } else {
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nok\r\n0\r\n\r\n",
                );
            }
        }
    };
    let (origin, heads, thread) = serve(6, hop(5));
    let (response, body) = get(
        &Client::for_test(origin.clone()),
        &origin,
        &limits(5, Some(10)),
    )
    .unwrap();
    thread.join().unwrap();
    assert_eq!((response.status, body.as_slice()), (200, &b"ok"[..]));
    assert_eq!(response.url, origin.join("/hop5?n=5").unwrap());
    assert!(heads.lock().unwrap()[5].starts_with("GET /hop5?n=5 HTTP/1.1\r\n"));
    let (origin, heads, thread) = serve(7, hop(6));
    let error = get(
        &Client::for_test(origin.clone()),
        &origin,
        &limits(5, Some(10)),
    )
    .unwrap_err();
    assert_eq!(error, HttpError::Protocol("too many redirects"));
    assert_eq!(heads.lock().unwrap().len(), 6, "no seventh request");
    drop(TcpStream::connect(origin.socket_addrs(|| None).unwrap()[0]));
    thread.join().unwrap();
    // a redirect off the test origin is refused before any request goes there
    let (origin, _, thread) = serve(1, |_, _, s| {
        let _ = s.write_all(
            b"HTTP/1.1 301 Moved\r\nLocation: https://github.com/\r\nContent-Length: 0\r\n\r\n",
        );
    });
    assert_eq!(
        get(
            &Client::for_test(origin.clone()),
            &origin,
            &limits(5, Some(10))
        )
        .unwrap_err(),
        HttpError::Policy
    );
    thread.join().unwrap();
}

#[test]
fn a_non_2xx_answer_is_returned_without_its_body_and_a_2xx_without_a_length_is_refused() {
    let (origin, _, thread) = serve(2, |i, _, s| {
        if i == 0 {
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\n\r\n{\"message\":\"Not Found\"}");
        } else {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n[]");
        }
    });
    let client = Client::for_test(origin.clone());
    let (response, body) = get(&client, &origin, &limits(5, Some(10))).unwrap();
    assert_eq!((response.status, body.len()), (404, 0));
    assert_eq!(
        get(&client, &origin, &limits(5, Some(10))).unwrap_err(),
        HttpError::Protocol("no length")
    );
    thread.join().unwrap();
}

#[test]
fn a_server_that_never_answers_ends_in_a_timeout_at_the_deadline() {
    let (origin, _, thread) = serve(1, |_, _, _| std::thread::sleep(Duration::from_millis(2500)));
    let started = Instant::now();
    let error = get(
        &Client::for_test(origin.clone()),
        &origin,
        &limits(30, Some(1)),
    )
    .unwrap_err();
    let took = started.elapsed();
    assert_eq!(error, HttpError::Timeout);
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_secs(2),
        "{took:?}"
    );
    thread.join().unwrap();
}

#[test]
fn stop_from_another_thread_ends_a_blocked_read_within_a_second() {
    let (origin, _, thread) = serve(1, |_, _, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial");
        std::thread::sleep(Duration::from_millis(2500));
    });
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        flag.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    let mut body = Vec::new();
    let error = Client::for_test(origin.clone())
        .get(
            &origin,
            "application/json",
            &limits(30, None),
            &mut body,
            &stop,
        )
        .unwrap_err();
    let took = started.elapsed();
    assert_eq!(error, HttpError::Cancelled);
    assert!(took < Duration::from_millis(1300), "{took:?}");
    assert_eq!(body, b"partial");
    thread.join().unwrap();
}

#[test]
fn a_stalled_body_ends_in_a_timeout_at_the_stall_limit() {
    let (origin, _, thread) = serve(1, |_, _, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nsome");
        std::thread::sleep(Duration::from_millis(2500));
    });
    let started = Instant::now();
    let error = get(&Client::for_test(origin.clone()), &origin, &limits(1, None)).unwrap_err();
    let took = started.elapsed();
    assert_eq!(error, HttpError::Timeout);
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_millis(2200),
        "{took:?}"
    );
    thread.join().unwrap();
}

#[test]
fn the_policy_refuses_a_url_before_connecting() {
    let client = Client::production();
    assert!(matches!(client.policy(), Policy::Production));
    let limits = limits(1, Some(1));
    assert_eq!(
        get(&client, &url("http://api.github.com/"), &limits).unwrap_err(),
        HttpError::Policy
    );
    let test = Client::for_test(url("http://127.0.0.1:47811/"));
    assert_eq!(
        get(&test, &url("https://api.github.com/"), &limits).unwrap_err(),
        HttpError::Policy
    );
    assert_eq!(
        get(&test, &url("http://127.0.0.1:47812/"), &limits).unwrap_err(),
        HttpError::Policy
    );
}

#[test]
fn a_closed_port_is_a_connect_error() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let origin = url(&format!("http://127.0.0.1:{port}/"));
    assert_eq!(
        get(
            &Client::for_test(origin.clone()),
            &origin,
            &limits(5, Some(5))
        )
        .unwrap_err(),
        HttpError::Connect
    );
}

// ------------------------------------------------------------------------------------------------ TLS

/// Accept one connection within 10 s (a client that never connects must not hang the test).
fn accept_within(listener: &TcpListener) -> Option<TcpStream> {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                return Some(stream);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(_) => return None,
        }
    }
    None
}

#[test]
fn production_refuses_a_certificate_that_no_trusted_authority_signed() {
    if !gio::TlsBackend::default().supports_tls() {
        eprintln!("skip: GIO has no TLS backend here (glib-networking)");
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let Some(stream) = accept_within(&listener) else {
            return false;
        };
        let certificate =
            gio::TlsCertificate::from_pem(SELF_SIGNED).expect("the fixture's certificate and key");
        let socket = unsafe { gio::Socket::from_fd(stream) }.unwrap();
        let connection = socket.connection_factory_create_connection();
        let tls = gio::TlsServerConnection::new(&connection, Some(&certificate)).unwrap();
        // the client aborts the handshake once it has seen the certificate
        tls.handshake(gio::Cancellable::NONE).is_err()
    });
    let error = get(
        &Client::production(),
        &url(&format!("https://localhost:{port}/")),
        &limits(5, Some(10)),
    )
    .unwrap_err();
    assert!(matches!(error, HttpError::Tls(_)), "{error:?}");
    assert_eq!(error.key(), "update.winpoint.err.tls");
    assert!(
        server.join().unwrap(),
        "the server saw the client refuse the handshake"
    );
}

/// The module files of the updater (not this test file's own search strings, which are built below).
fn sources() -> Vec<(PathBuf, String)> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services");
    let mut files = vec![base.join("winpoint.rs")];
    for entry in std::fs::read_dir(base.join("winpoint")).unwrap() {
        files.push(entry.unwrap().path());
    }
    files
        .into_iter()
        .map(|p| (p.clone(), std::fs::read_to_string(&p).unwrap()))
        .collect()
}

#[test]
fn nothing_under_services_winpoint_weakens_tls() {
    let banned = [
        ["accept", "-certificate"].concat(),
        ["accept", "_certificate"].concat(),
        ["tls_", "validation_flags"].concat(),
        ["set_", "server_identity"].concat(),
        ["Tls", "CertificateFlags"].concat(),
    ];
    let files = sources();
    assert!(files.len() >= 11, "{} files", files.len());
    for (path, text) in files {
        for word in &banned {
            assert!(
                !text.contains(word.as_str()),
                "{} contains {word}",
                path.display()
            );
        }
    }
}

/// The text of `fn <name>(` up to its closing brace (the first such function after `from`).
fn body<'a>(text: &'a str, name: &str, from: usize) -> &'a str {
    let head = format!("fn {name}(");
    let start = from
        + text[from..]
            .find(&head)
            .unwrap_or_else(|| panic!("no {head}"));
    let open = start + text[start..].find('{').unwrap();
    let mut depth = 0usize;
    for (at, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[start..open + at + 1];
                }
            }
            _ => {}
        }
    }
    panic!("{head} is not closed")
}
fn count(text: &str, word: &str) -> usize {
    text.matches(word).count()
}

/// D-G73 and D-G74: WinPoint starts one Python program of its own, the release bundle's scripts/install.py, and only
/// from `winpoint.install` (the user's click on Download and install or Try again): never from a check, a timer, a
/// notification, a status read or a reattach. The program path is written once; its two starts are in job.rs
/// `run_check` and `run_install`, which only `install_steps` calls, which only the job thread of `job::start` runs,
/// which only `WinPoint::install` calls.
#[test]
fn the_one_python_program_starts_only_from_the_users_click() {
    let python = ["/usr/bin/", "python"].concat();
    let command = ["Command", "::new("].concat();
    let scope = ["process", "_scope"].concat();
    let spawn_pid = ["spawn", "_pid"].concat();
    let code: Vec<(String, String)> = sources()
        .into_iter()
        .filter(|(path, _)| {
            let name = path.file_name().unwrap().to_string_lossy();
            name != "tests.rs" && name != "e2e_tests.rs"
        })
        .map(|(path, text)| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                text,
            )
        })
        .collect();
    let text_of = |name: &str| {
        code.iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.as_str())
            .unwrap()
    };
    let total = |word: &str| code.iter().map(|(_, t)| count(t, word)).sum::<usize>();
    let job = text_of("job.rs");
    let service = text_of("winpoint.rs");
    assert_eq!(total(&python), 1, "the interpreter's path, once");
    assert_eq!(
        count(job, &[&python, "3\";"].concat()),
        1,
        "only const PYTHON in job.rs"
    );
    for (name, text) in &code {
        if name != "job.rs" {
            for word in [&command, &scope, &spawn_pid, "PYTHON"] {
                assert_eq!(count(text, word), 0, "{name} contains {word}");
            }
        }
    }
    // the two starts of the interpreter
    assert_eq!(count(job, &command), 2);
    let check = body(job, "run_check", 0);
    let install = body(job, "run_install", 0);
    for start in [check, install] {
        assert_eq!(count(start, &[&command, "PYTHON)"].concat()), 1, "{start}");
    }
    assert_eq!(count(job, &spawn_pid), 1);
    let at = job.find("impl Spawner for Scope").unwrap();
    assert!(body(job, "spawn", at).contains(&spawn_pid));
    // who calls them: install_steps only, run by run_job only, started by job::start only, called by install only
    let steps = body(job, "install_steps", 0);
    for (call, inside, calls) in [
        ("run_check(", steps, 1),
        ("run_install(", steps, 1),
        ("install_steps(", body(job, "run_job", 0), 1),
        ("run_job(", body(job, "start", 0), 1),
    ] {
        assert_eq!(
            count(job, call),
            calls + 1,
            "{call}: its definition and its call"
        );
        assert_eq!(count(inside, call), calls, "{call}");
    }
    assert_eq!(count(service, "job::start("), 1);
    assert_eq!(count(body(service, "install", 0), "job::start("), 1);
    // and never from a check, a timer, a status read, a notification or a reattach
    let never = [
        "job::start(",
        "run_job(",
        "install_steps(",
        "run_check(",
        "run_install(",
        ".spawn(command",
        ".output(command",
    ];
    for (name, inside) in [
        ("reattach", body(job, "reattach", 0)),
        ("tick", body(service, "tick", 0)),
        ("check_now", body(service, "check_now", 0)),
        ("status", body(service, "status", 0)),
        ("start", body(service, "start", 0)),
        ("take_over", body(service, "take_over", 0)),
        ("notify", body(service, "notify", 0)),
        ("notify_installed", body(service, "notify_installed", 0)),
    ] {
        for word in never {
            assert!(!inside.contains(word), "{name} contains {word}");
        }
    }
    for name in [
        "check.rs",
        "schedule.rs",
        "notify.rs",
        "feed.rs",
        "http.rs",
        "seam.rs",
        "verify.rs",
    ] {
        for word in never {
            assert!(!text_of(name).contains(word), "{name} contains {word}");
        }
    }
    // no automatic installation and no setting for it
    for word in ["autoInstall", "auto_install", "winpointAutoInstall"] {
        assert_eq!(total(word), 0, "{word}");
    }
}

// ------------------------------------------------------------------------------------------------ feed

#[test]
fn the_feed_keeps_stable_published_releases_and_their_complete_asset_sets() {
    let github = Origin::github();
    let all = feed::parse_releases(RELEASES.as_bytes(), &github, "x86_64").unwrap();
    let versions: Vec<String> = all.iter().map(|r| r.version.to_string()).collect();
    // v2.0.0 (a fork's page), v1.0.0 (draft), v0.9.9 (prerelease), v0.9.8-rc.1 and release-1 are skipped
    assert_eq!(
        versions,
        [
            "0.9.7", "0.9.6", "0.9.5", "0.9.4", "0.9.3", "0.9.2", "0.9.1", "0.9.0"
        ]
    );
    let by = |v: &str| all.iter().find(|r| r.version.to_string() == v).unwrap();
    for incomplete in ["0.9.7", "0.9.6", "0.9.5", "0.9.4", "0.9.3"] {
        // aarch64 only, size 0, state open, a foreign download URL, over 256 MiB
        assert!(by(incomplete).assets.is_none(), "{incomplete}");
    }
    let r092 = by("0.9.2").assets.clone().unwrap();
    assert_eq!(r092.tarball.name, "winpoint-0.9.2-linux-x86_64.tar.gz");
    assert_eq!(
        r092.sha256.name,
        "winpoint-0.9.2-linux-x86_64.tar.gz.sha256"
    );
    assert_eq!(
        r092.tarball.digest, None,
        "the reason noDigest (no download by WinPoint)"
    );
    let r091 = by("0.9.1").assets.clone().unwrap();
    assert!(r091.tarball.digest.unwrap().starts_with("sha256:"));
    // an asset of another name (an old .sha256.sig of the fixture) is ignored: there is no signature (D-G74)
    assert!(
        !by("0.9.1").json().to_string().contains(".sig"),
        "{}",
        by("0.9.1").json()
    );
    assert_eq!(r091.tarball.size, 71234567);
    assert_eq!(
        r091.tarball.url.as_str(),
        "https://github.com/roboburr/WinPoint/releases/download/v0.9.1/winpoint-0.9.1-linux-x86_64.tar.gz"
    );
    assert_eq!(
        by("0.9.0").assets.as_ref().unwrap().tarball.digest,
        None,
        "no digest field at all"
    );
    assert_eq!(
        by("0.9.7").name,
        "WinPoint 0.9.7",
        "an empty name falls back"
    );
    assert_eq!(by("0.9.1").name, "WinPoint 0.9.1");
    assert_eq!(
        by("0.9.1").notes_url,
        "https://github.com/roboburr/WinPoint/releases/tag/v0.9.1"
    );
    assert_eq!(
        by("0.9.1").published_at.as_deref(),
        Some("2026-11-21T10:00:00Z")
    );
    let arm = feed::parse_releases(RELEASES.as_bytes(), &github, "aarch64").unwrap();
    let arm_complete: Vec<String> = arm
        .iter()
        .filter(|r| r.assets.is_some())
        .map(|r| r.version.to_string())
        .collect();
    assert_eq!(arm_complete, ["0.9.7", "0.9.1"]);
    // the round trip through state.json
    for release in &all {
        assert_eq!(
            feed::Release::from_json(&release.json()).as_ref(),
            Some(release)
        );
    }
}

#[test]
fn choose_offers_the_highest_newer_release_for_this_pc_and_never_a_downgrade() {
    let github = Origin::github();
    let x86 = feed::parse_releases(RELEASES.as_bytes(), &github, "x86_64").unwrap();
    let arm = feed::parse_releases(RELEASES.as_bytes(), &github, "aarch64").unwrap();
    let chosen = |all: &[feed::Release], current: &str| {
        feed::choose(all, &v(current)).map(|r| (r.version.to_string(), r.assets.is_some()))
    };
    assert_eq!(
        chosen(&x86, "0.9.0"),
        Some(("0.9.2".into(), true)),
        "0.9.3 to 0.9.7 have no x86_64 set"
    );
    assert_eq!(chosen(&x86, "0.8.0"), Some(("0.9.2".into(), true)));
    assert_eq!(chosen(&x86, "0.9.1"), Some(("0.9.2".into(), true)));
    assert_eq!(chosen(&arm, "0.9.0"), Some(("0.9.7".into(), true)));
    assert_eq!(
        chosen(&x86, "0.9.2"),
        Some(("0.9.7".into(), false)),
        "noAsset: the highest newer one"
    );
    assert_eq!(chosen(&x86, "0.9.7"), None);
    assert_eq!(chosen(&x86, "0.10.0-dev"), None);
    assert_eq!(chosen(&x86, "9.0.0"), None, "never a downgrade");
    assert_eq!(chosen(&[], "0.9.0"), None);
    // a development build before 0.9.2 still gets it
    assert_eq!(chosen(&x86, "0.9.2-dev"), Some(("0.9.2".into(), true)));
}

#[test]
fn the_feed_has_a_size_limit_and_must_be_a_json_array() {
    let github = Origin::github();
    let big = vec![b' '; feed::MAX_FEED as usize + 1];
    assert_eq!(
        feed::parse_releases(&big, &github, "x86_64"),
        Err(feed::FeedError::TooLarge)
    );
    assert_eq!(
        feed::parse_releases(b"<html>", &github, "x86_64"),
        Err(feed::FeedError::NotJson)
    );
    assert_eq!(
        feed::parse_releases(b"{\"message\":\"x\"}", &github, "x86_64"),
        Err(feed::FeedError::NotArray)
    );
    assert_eq!(feed::parse_releases(b"[]", &github, "x86_64"), Ok(vec![]));
    assert_eq!(
        feed::parse_releases(b"[1, null, {}]", &github, "x86_64"),
        Ok(vec![])
    );
}

#[test]
fn a_test_origin_serves_its_own_release_pages_and_github_urls_are_foreign_there() {
    let origin = url("http://127.0.0.1:47811/");
    let local = releases_at(origin.as_str());
    let test = Origin::test(&origin);
    assert_eq!(
        test.feed_url().as_str(),
        "http://127.0.0.1:47811/repos/roboburr/WinPoint/releases?per_page=10"
    );
    let all = feed::parse_releases(local.as_bytes(), &test, "x86_64").unwrap();
    assert_eq!(all.len(), 8);
    assert_eq!(
        all[6].notes_url,
        "http://127.0.0.1:47811/roboburr/WinPoint/releases/tag/v0.9.1"
    );
    assert!(
        feed::parse_releases(RELEASES.as_bytes(), &test, "x86_64")
            .unwrap()
            .is_empty()
    );
    assert!(
        feed::parse_releases(local.as_bytes(), &Origin::github(), "x86_64")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        Origin::github().feed_url().as_str(),
        "https://api.github.com/repos/roboburr/WinPoint/releases?per_page=10"
    );
}

/// `html_url` is parsed first and must be exactly the release's own page in its normal form, which is what is stored
/// and opened: dot segments (plain, `%2e%2e`, or with backslashes, which count as slashes in an https URL) that
/// climb out of the repository's release pages are refused, so are a query, a fragment, another tag's page or host.
#[test]
fn the_release_page_is_the_releases_own_in_its_normal_form() {
    let github = Origin::github();
    let page = "https://github.com/roboburr/WinPoint/releases/tag/v0.9.1";
    let list: Value = serde_json::from_str(RELEASES).unwrap();
    let entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["tag_name"] == "v0.9.1")
        .unwrap()
        .clone();
    let with = |html: &str| {
        let mut one = entry.clone();
        one["html_url"] = json!(html);
        let body = serde_json::to_vec(&json!([one])).unwrap();
        feed::parse_releases(&body, &github, "x86_64")
            .unwrap()
            .first()
            .map(|r| r.notes_url.clone())
    };
    assert_eq!(with(page).as_deref(), Some(page));
    // harmless spellings of the same page are stored as the page itself
    for same in [
        "https://github.com/roboburr/WinPoint/releases/x/../tag/v0.9.1",
        "https://GitHub.com:443/roboburr/WinPoint/releases/tag/v0.9.1",
        "https://github.com/roboburr/WinPoint/releases/%2e/tag/v0.9.1",
    ] {
        assert_eq!(with(same).as_deref(), Some(page), "{same}");
    }
    for escape in [
        "https://github.com/roboburr/WinPoint/releases/../../../attacker/WinPoint/releases/tag/v0.9.1",
        "https://github.com/roboburr/WinPoint/releases/%2e%2e/%2e%2e/%2e%2e/attacker/WinPoint/releases/tag/v0.9.1",
        "https://github.com/roboburr/WinPoint/releases/%2E%2E/%2E%2E/%2E%2E/attacker/WinPoint/releases/tag/v0.9.1",
        r"https://github.com/roboburr/WinPoint/releases/..\..\..\attacker\WinPoint\releases\tag\v0.9.1",
        "https://github.com/roboburr/WinPoint/releases/tag/v0.9.1/../../../../../attacker/WinPoint/releases/tag/v0.9.1",
        "https://github.com/roboburr/WinPoint/releases/tag/v0.9.1?next=https://example.org/",
        "https://github.com/roboburr/WinPoint/releases/tag/v0.9.1#x",
        "https://github.com/roboburr/WinPoint/releases/tag/v0.9.2",
        "https://github.com/roboburr/WinPoint/releases/",
        "https://github.com/roboburr/WinPoint/releases/download/v0.9.1/winpoint-0.9.1-linux-x86_64.tar.gz",
        "http://github.com/roboburr/WinPoint/releases/tag/v0.9.1",
        "https://github.com.example.org/roboburr/WinPoint/releases/tag/v0.9.1",
        "https://someone@github.com/roboburr/WinPoint/releases/tag/v0.9.1",
        "not a url",
    ] {
        assert_eq!(with(escape), None, "{escape}");
    }
    // what the url crate makes of the escapes (the strings a raw prefix test took)
    for escape in [
        "https://github.com/roboburr/WinPoint/releases/../../../attacker/WinPoint/releases/tag/v0.9.1",
        "https://github.com/roboburr/WinPoint/releases/%2e%2e/%2e%2e/%2e%2e/attacker/WinPoint/releases/tag/v0.9.1",
        r"https://github.com/roboburr/WinPoint/releases/..\..\..\attacker\WinPoint\releases\tag\v0.9.1",
    ] {
        assert!(escape.starts_with("https://github.com/roboburr/WinPoint/releases/"));
        assert_eq!(
            url(escape).as_str(),
            "https://github.com/attacker/WinPoint/releases/tag/v0.9.1"
        );
    }
    let test = Origin::test(&url("http://127.0.0.1:47811/"));
    assert_eq!(
        feed::release_page(
            "http://127.0.0.1:47811/roboburr/WinPoint/releases/tag/v1.0.0",
            &test,
            "v1.0.0"
        )
        .as_deref(),
        Some("http://127.0.0.1:47811/roboburr/WinPoint/releases/tag/v1.0.0")
    );
    assert_eq!(
        feed::release_page(page, &test, "v0.9.1"),
        None,
        "GitHub's page is foreign to a test origin"
    );
}

#[test]
fn asset_names_are_frozen() {
    assert_eq!(
        feed::tarball_name(&v("0.9.0"), "x86_64"),
        "winpoint-0.9.0-linux-x86_64.tar.gz"
    );
    assert!(!feed::machine().is_empty());
}

// ------------------------------------------------------------------------------------------------ schedule

#[test]
fn an_automatic_check_is_due_once_a_day_and_never_before_github_allows_it() {
    let now = 1_763_130_000_000u64;
    let h = schedule::HOUR_MS;
    let cases = [
        (json!({}), true, false, true, "never checked"),
        (json!({}), false, false, false, "automatic checks off"),
        (json!({}), true, true, false, "a job runs"),
        (
            json!({"checkedAt":now - 2 * h,"nextCheckAt":now + 20 * h}),
            true,
            false,
            false,
            "not yet",
        ),
        (
            json!({"checkedAt":now - 25 * h,"nextCheckAt":now - h}),
            true,
            false,
            true,
            "a day later",
        ),
        (
            json!({"nextCheckAt":now - h,"retryAfter":now + h}),
            true,
            false,
            false,
            "GitHub's wait",
        ),
        (
            json!({"nextCheckAt":now - h,"retryAfter":now - 1}),
            true,
            false,
            true,
            "the wait is over",
        ),
        (
            json!({"checkedAt":now + 2 * h,"nextCheckAt":now + 26 * h}),
            true,
            false,
            true,
            "the clock jumped back",
        ),
        (
            json!({"checkedAt":now + h,"nextCheckAt":now + 20 * h}),
            true,
            false,
            false,
            "within the hour's slack",
        ),
        (
            json!({"nextCheckAt":now + 30 * h}),
            true,
            false,
            true,
            "further away than any schedule sets",
        ),
        (
            json!({"retryAfter":now + 30 * h}),
            true,
            false,
            true,
            "a wait no answer sets",
        ),
        (
            json!({"checkedAt":now + 2 * h}),
            false,
            false,
            false,
            "a jumped clock, but off",
        ),
    ];
    for (state, auto, job, due, why) in cases {
        assert_eq!(schedule::due(now, &state, auto, job), due, "{why}");
    }
    assert_eq!(schedule::first_delay_s(false, 0), 180);
    assert_eq!(
        schedule::first_delay_s(false, u32::MAX),
        599,
        "3 to 10 minutes after the start"
    );
    assert_eq!(schedule::first_delay_s(true, 300), 5, "a test seam's run");
    assert_eq!(schedule::after_ok(now, 0), now + schedule::DAY_MS);
    assert_eq!(
        schedule::after_ok(now, u64::MAX),
        now + schedule::DAY_MS + schedule::OK_JITTER_MS - 1
    );
    assert_eq!(schedule::backoff_ms(1), 6 * h);
    assert_eq!(schedule::backoff_ms(2), 12 * h);
    assert_eq!(schedule::backoff_ms(3), 24 * h);
    assert_eq!(schedule::backoff_ms(40), 24 * h);
    for _ in 0..200 {
        let jitter = schedule::jitter_ms(schedule::OK_JITTER_MS);
        assert!(jitter < schedule::OK_JITTER_MS && jitter % 1000 == 0);
    }
}

// ------------------------------------------------------------------------------------------------ check

fn response(status: u16, headers: &[(&str, &str)]) -> http::Response {
    http::Response {
        status,
        url: url("https://api.github.com/repos/roboburr/WinPoint/releases?per_page=10"),
        headers: headers
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect(),
    }
}

#[test]
fn each_answer_of_the_list_request_is_classified() {
    let now = 1_763_130_000_000u64;
    let github = Origin::github();
    let classify =
        |r: http::Response, body: &[u8]| check::classify(&r, body, &github, "x86_64", now);
    assert!(
        matches!(classify(response(200, &[]), RELEASES.as_bytes()), Fetched::Releases(all) if all.len() == 8)
    );
    assert!(matches!(
        classify(response(200, &[]), b"<html>"),
        Fetched::Failed(Failure { code: "feed", .. })
    ));
    assert_eq!(classify(response(404, &[]), b""), Fetched::NotFound);
    let reset = (now / 1000 + 600).to_string();
    assert_eq!(
        classify(
            response(
                403,
                &[
                    ("X-RateLimit-Remaining", "0"),
                    ("X-RateLimit-Reset", &reset)
                ]
            ),
            b""
        ),
        Fetched::RateLimited {
            until_ms: now + schedule::HOUR_MS
        },
        "at least an hour"
    );
    let late = (now / 1000 + 7200).to_string();
    assert_eq!(
        classify(
            response(
                403,
                &[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", &late)]
            ),
            b""
        ),
        Fetched::RateLimited {
            until_ms: (now / 1000 + 7200) * 1000
        }
    );
    assert_eq!(
        classify(response(429, &[("Retry-After", "10800")]), b""),
        Fetched::RateLimited {
            until_ms: now + 10_800_000
        }
    );
    let Fetched::Failed(forbidden) =
        classify(response(403, &[("X-RateLimit-Remaining", "12")]), b"")
    else {
        panic!("a 403 without a rate limit is an error")
    };
    assert_eq!(
        (forbidden.code, forbidden.key, forbidden.params.clone()),
        ("status", "update.winpoint.err.status", json!({"code":403}))
    );
    assert!(
        forbidden.detail.contains("api.github.com"),
        "{}",
        forbidden.detail
    );
    assert!(matches!(
        classify(response(500, &[]), b""),
        Fetched::Failed(Failure { code: "status", .. })
    ));
}

#[test]
fn the_state_follows_each_answer() {
    let now = 1_763_130_000_000u64;
    let h = schedule::HOUR_MS;
    let current = v("0.9.0");
    let all = feed::parse_releases(RELEASES.as_bytes(), &Origin::github(), "x86_64").unwrap();
    let ok = check::next_state(
        &json!({}),
        &Fetched::Releases(all.clone()),
        now,
        &current,
        1000,
    );
    assert_eq!(ok["result"], "ok");
    assert_eq!(ok["latest"]["version"], "0.9.2");
    assert_eq!(ok["latest"]["assets"]["tarball"]["size"], 71234567);
    assert_eq!(
        (
            ok["everOk"].clone(),
            ok["failures"].clone(),
            ok["error"].clone()
        ),
        (json!(true), json!(0), Value::Null)
    );
    assert_eq!(
        (ok["checkedAt"].as_u64(), ok["okAt"].as_u64()),
        (Some(now), Some(now))
    );
    assert_eq!(ok["nextCheckAt"], now + schedule::DAY_MS + 1000);
    assert_eq!(
        check::next_state(&ok, &Fetched::Releases(all.clone()), now, &v("0.9.7"), 0)["latest"],
        Value::Null
    );
    // 404 before any 200: no release yet (up to date); after one: unreachable, the last release stays
    let none = check::next_state(&json!({}), &Fetched::NotFound, now, &current, 0);
    assert_eq!(
        (
            none["result"].clone(),
            none["latest"].clone(),
            none["error"].clone()
        ),
        (json!("noReleases"), Value::Null, Value::Null)
    );
    assert_eq!(none["everOk"], false);
    let gone = check::next_state(&ok, &Fetched::NotFound, now + h, &current, 0);
    assert_eq!(gone["result"], "unreachable");
    assert_eq!(gone["error"]["key"], "update.winpoint.err.unreachable");
    assert_eq!(gone["latest"], ok["latest"]);
    assert_eq!(gone["okAt"], ok["okAt"]);
    assert_eq!(gone["nextCheckAt"], now + h + schedule::DAY_MS);
    // the rate limit
    let limited = check::next_state(
        &ok,
        &Fetched::RateLimited {
            until_ms: now + 2 * h,
        },
        now,
        &current,
        0,
    );
    assert_eq!(
        (limited["result"].clone(), limited["retryAfter"].clone()),
        (json!("rateLimited"), json!(now + 2 * h))
    );
    assert_eq!(limited["nextCheckAt"], now + 2 * h);
    assert_eq!(limited["latest"], ok["latest"]);
    // errors back off 6 h, 12 h, 24 h; a success clears them
    let failure = Fetched::Failed(Failure::http(&HttpError::Timeout));
    let mut state = ok.clone();
    for (n, wait) in [(1, 6 * h), (2, 12 * h), (3, 24 * h), (4, 24 * h)] {
        state = check::next_state(&state, &failure, now, &current, 0);
        assert_eq!(
            (state["failures"].as_u64(), state["nextCheckAt"].as_u64()),
            (Some(n), Some(now + wait))
        );
        assert_eq!(state["error"]["key"], "update.winpoint.err.timeout");
        assert_eq!(state["latest"], ok["latest"]);
    }
    let again = check::next_state(&state, &Fetched::Releases(all), now, &current, 0);
    assert_eq!(
        (
            again["failures"].clone(),
            again["error"].clone(),
            again["result"].clone()
        ),
        (json!(0), Value::Null, json!("ok"))
    );
    // offline: no request, so neither checkedAt nor the schedule move
    let offline = check::next_state(&ok, &Fetched::Offline, now + h, &current, 0);
    assert_eq!(offline["result"], "offline");
    assert_eq!(offline["error"]["key"], "update.winpoint.err.offline");
    assert_eq!(
        (offline["checkedAt"].clone(), offline["nextCheckAt"].clone()),
        (ok["checkedAt"].clone(), ok["nextCheckAt"].clone())
    );
}

#[test]
fn only_an_automatic_check_notifies_and_only_once_per_version() {
    let version = v("0.9.2");
    let fresh = json!({"notified":null});
    assert!(check::notify_now(&fresh, &version, Trigger::Auto, true));
    assert!(
        !check::notify_now(&fresh, &version, Trigger::Manual, true),
        "a manual check only records it"
    );
    assert!(
        !check::notify_now(&fresh, &version, Trigger::Auto, false),
        "automatic checks off"
    );
    assert!(!check::notify_now(
        &json!({"notified":"0.9.2"}),
        &version,
        Trigger::Auto,
        true
    ));
    assert!(
        check::notify_now(&json!({"notified":"0.9.1"}), &version, Trigger::Auto, true),
        "a newer one again"
    );
    assert_eq!(check::newer_latest(&json!({}), &v("0.9.0")), None);
}

// ------------------------------------------------------------------------------------------------ the service

pub(super) fn view<'a>(state: &'a Value, current: &'a Version) -> View<'a> {
    View {
        mode: "installed",
        owner_running: Some(true),
        checking: false,
        fault: None,
        state,
        job: None,
        current,
        installed: Some("0.9.0".into()),
        arch: "x86_64",
        auto: true,
        dry: false,
        python: true,
        exe_dir: "/home/u/.local/bin".into(),
        lang: Lang::En,
    }
}

#[test]
fn the_status_state_follows_its_precedence() {
    let current = v("0.9.0");
    let all = feed::parse_releases(RELEASES.as_bytes(), &Origin::github(), "x86_64").unwrap();
    let ok = check::next_state(
        &json!({"rev":4}),
        &Fetched::Releases(all),
        1_000,
        &current,
        0,
    );
    let state_of = |state: &Value, f: &dyn Fn(&mut View)| {
        let mut w = view(state, &current);
        f(&mut w);
        compose(&w)
    };
    let available = state_of(&ok, &|_| {});
    assert_eq!(available["state"], "available");
    assert_eq!(available["latest"]["version"], "0.9.2");
    assert_eq!(
        available["latest"]["notesUrl"],
        "https://github.com/roboburr/WinPoint/releases/tag/v0.9.2"
    );
    assert_eq!(
        available["latest"]["asset"],
        "winpoint-0.9.2-linux-x86_64.tar.gz"
    );
    assert_eq!(available["latest"]["size"], 71234567);
    // 0.9.2's tarball has no digest from GitHub: offered with its links, never downloaded by WinPoint
    assert_eq!(
        (
            available["latest"]["reason"].clone(),
            available["latest"]["canInstall"].clone(),
            available["error"].clone()
        ),
        (json!("noDigest"), json!(false), Value::Null)
    );
    assert_eq!(
        available["latest"]["message"],
        "This release has no checksum from GitHub; download it from the link"
    );
    assert_eq!(
        (available["rev"].clone(), available["checkedAt"].clone()),
        (json!(4), json!(1_000))
    );
    assert_eq!(
        (available["version"].clone(), available["current"].clone()),
        (json!(crate::VERSION), json!("0.9.0"))
    );
    assert_eq!(available["job"], Value::Null);
    assert_eq!(state_of(&ok, &|w| w.checking = true)["state"], "checking");
    // a state folder this host cannot use: a failed check with the folder's error, over checking; the release found
    // before stays below, and whether the owner runs is not known
    let fault = || {
        let error = io::Error::from(io::ErrorKind::PermissionDenied);
        Some(
            Failure::cache(
                Path::new("/c/win11-gnome/winpoint"),
                Some("check.lock"),
                &error,
            )
            .json(),
        )
    };
    let cache = state_of(&ok, &|w| {
        w.fault = fault();
        w.checking = true;
        w.owner_running = None;
    });
    assert_eq!(
        (cache["state"].clone(), cache["error"]["code"].clone()),
        (json!("checkFailed"), json!("cache"))
    );
    assert_eq!(cache["error"]["key"], "update.winpoint.err.cache");
    assert_eq!(
        cache["error"]["message"],
        "WinPoint can’t use its folder /c/win11-gnome/winpoint. The folder and the files in it must be yours and \
         accessible only to you."
    );
    assert!(
        cache["error"]["detail"]
            .as_str()
            .unwrap()
            .starts_with("/c/win11-gnome/winpoint/check.lock: "),
        "{}",
        cache["error"]
    );
    assert_eq!(cache["latest"]["version"], "0.9.2");
    assert_eq!(cache["ownerRunning"], Value::Null);
    assert_eq!(
        state_of(&ok, &|w| {
            w.fault = fault();
            w.mode = "unsupported";
        })["error"]["code"],
        "test_seam",
        "a refused seam stays first"
    );
    assert_eq!(
        state_of(&ok, &|w| w.mode = "unsupported")["state"],
        "unsupported"
    );
    assert_eq!(
        state_of(&ok, &|w| w.mode = "unsupported")["error"]["code"],
        "test_seam"
    );
    // the stored release is no longer newer (installed by hand meanwhile): up to date, no latest
    let newer = v("0.9.2");
    let mut w = view(&ok, &newer);
    w.installed = Some("0.9.2".into());
    assert_eq!(
        (compose(&w)["state"].clone(), compose(&w)["latest"].clone()),
        (json!("upToDate"), Value::Null)
    );
    // a failed last check wins over the release it found before, which stays below
    for (fetched, state, message) in [
        (Fetched::Offline, "offline", "No internet connection"),
        (
            Fetched::NotFound,
            "unreachable",
            "WinPoint’s releases can’t be found on GitHub",
        ),
        (
            Fetched::Failed(Failure::http(&HttpError::Status(502))),
            "checkFailed",
            "GitHub returned an error (502)",
        ),
        (
            Fetched::RateLimited {
                until_ms: 3_600_000 * 30,
            },
            "rateLimited",
            "GitHub asks WinPoint to wait until ",
        ),
    ] {
        let s = check::next_state(&ok, &fetched, 2_000, &current, 0);
        let out = state_of(&s, &|_| {});
        assert_eq!(out["state"], state, "{fetched:?}");
        assert!(
            out["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with(message),
            "{}",
            out["error"]
        );
        assert_eq!(out["latest"]["version"], "0.9.2");
    }
    let limited = check::next_state(
        &ok,
        &Fetched::RateLimited {
            until_ms: 3_600_000 * 30,
        },
        2_000,
        &current,
        0,
    );
    let time = &state_of(&limited, &|_| {})["error"]["params"]["time"];
    assert!(
        time.as_str()
            .is_some_and(|t| t.len() == 5 && t.as_bytes()[2] == b':'),
        "{time}"
    );
    let none = check::next_state(&json!({}), &Fetched::NotFound, 1, &current, 0);
    assert_eq!(
        state_of(&none, &|_| {})["state"],
        "upToDate",
        "no release yet"
    );
    assert_eq!(state_of(&json!({}), &|_| {})["state"], "unknown");
    assert_eq!(state_of(&json!({}), &|_| {})["checkedAt"], Value::Null);
    // no download for this PC yet
    let arm_only = feed::parse_releases(RELEASES.as_bytes(), &Origin::github(), "x86_64").unwrap();
    let s = check::next_state(&json!({}), &Fetched::Releases(arm_only), 1, &v("0.9.2"), 0);
    let out = compose(&view(&s, &v("0.9.2")));
    assert_eq!(
        (out["state"].clone(), out["latest"]["reason"].clone()),
        (json!("available"), json!("noAsset"))
    );
    assert_eq!(
        out["latest"]["message"],
        "There’s no download for this PC (x86_64) yet"
    );
    assert_eq!(
        (
            out["latest"]["asset"].clone(),
            out["latest"]["size"].clone()
        ),
        (Value::Null, Value::Null)
    );
    let mut nl = view(&s, &current);
    nl.lang = Lang::Nl;
    let out = compose(&View {
        current: &v("0.9.2"),
        ..nl
    });
    assert_eq!(
        out["latest"]["message"],
        "Er is nog geen download voor deze pc (x86_64)"
    );
}

#[test]
fn every_method_takes_only_its_arguments() {
    for (method, args, ok) in [
        ("winpoint.status", json!({}), true),
        ("winpoint.status", json!({"x":1}), false),
        ("winpoint.status", json!([]), false),
        ("winpoint.status", Value::Null, false),
        ("winpoint.check", json!({}), true),
        ("winpoint.check", json!({"force":true}), false),
        ("winpoint.check", json!("x"), false),
        ("winpoint.install", json!({}), false),
        ("winpoint.install", json!({"version":"0.9.1"}), true),
        (
            "winpoint.install",
            json!({"version":"0.9.1","metered":true}),
            true,
        ),
        (
            "winpoint.install",
            json!({"version":"0.9.1","metered":false}),
            true,
        ),
        (
            "winpoint.install",
            json!({"version":"0.9.1","metered":1}),
            false,
        ),
        (
            "winpoint.install",
            json!({"version":"0.9.1","force":true}),
            false,
        ),
        ("winpoint.install", json!({"version":"v0.9.1"}), false),
        ("winpoint.install", json!({"version":"0.09.1"}), false),
        ("winpoint.install", json!({"version":9}), false),
        (
            "winpoint.install",
            json!({"version": format!("1.0.0-{}", "a".repeat(30))}),
            false,
        ),
        ("winpoint.install", json!([]), false),
        (
            "winpoint.cancel",
            json!({"jobId":"0123456789abcdef0123456789abcdef"}),
            true,
        ),
        ("winpoint.cancel", json!({"jobId":"x"}), false),
        ("winpoint.cancel", json!({}), false),
        (
            "winpoint.cancel",
            json!({"jobId":"0123456789abcdef0123456789abcdef","x":1}),
            false,
        ),
        ("winpoint.progress", json!({}), false),
        ("winpoint.nope", json!({}), false),
    ] {
        assert_eq!(check_args(method, &args), ok, "{method} {args}");
    }
    let root = Temp::new("args");
    let ctx = host(&root.0, "settings", false, "en", json!({}));
    let service = WinPoint::with(ctx.clone(), Ok(Seam::Production), notify::Target::Off);
    for (method, args) in [
        ("winpoint.install", json!({})),
        ("winpoint.install", json!({"version":"0.9.1","x":1})),
        ("winpoint.cancel", json!({"jobId":1})),
    ] {
        let error = service.call(method, &args).unwrap().unwrap_err();
        assert_eq!(error.code, "bad_args", "{method} {args}");
    }
    assert!(
        service.call("winpoint.progress", &json!({})).is_none(),
        "an event only"
    );
    let error = service
        .call("winpoint.check", &json!({"x":1}))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "bad_args");
    ctx.events.shutdown();
}

#[test]
fn a_dry_run_answers_from_the_files_and_never_checks() {
    let root = Temp::new("dry");
    let ctx = host(&root.0, "panel", true, "nl", json!({}));
    let service = WinPoint::with(ctx.clone(), Ok(Seam::Production), notify::Target::Session);
    assert_eq!(service.mode(), Mode::Dry);
    service.start();
    assert_eq!(service.mode(), Mode::Dry, "no owner lock, no timers");
    let status = service
        .call("winpoint.status", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            status["mode"].clone(),
            status["state"].clone(),
            status["dryRun"].clone()
        ),
        (json!("dry"), json!("unknown"), json!(true))
    );
    let check = service.call("winpoint.check", &json!({})).unwrap().unwrap();
    assert_eq!(check, json!({"started":false,"dryRun":true}));
    let install = service
        .call("winpoint.install", &json!({"version":"0.9.1"}))
        .unwrap()
        .unwrap();
    assert_eq!(install, json!({"jobId":null,"dryRun":true}));
    let cancel = service
        .call(
            "winpoint.cancel",
            &json!({"jobId":"0123456789abcdef0123456789abcdef"}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(cancel, json!({"ok":true,"dryRun":true}));
    assert!(!root.0.join("cache/winpoint/job.json").exists());
    assert!(!root.0.join("cache/winpoint/work").exists());
    assert!(!root.0.join("cache/winpoint/state.json").exists());
    assert!(!root.0.join("cache/winpoint/owner.lock").exists());
    ctx.events.shutdown();
}

#[test]
fn a_failed_seam_makes_the_service_unsupported_without_any_network() {
    let root = Temp::new("seam");
    let ctx = host(&root.0, "panel", false, "en", json!({}));
    let service = WinPoint::with(ctx.clone(), Err("bad seam".into()), notify::Target::Off);
    assert_eq!(service.mode(), Mode::Unsupported);
    service.start();
    let status = service
        .call("winpoint.status", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(
        (status["mode"].clone(), status["state"].clone()),
        (json!("unsupported"), json!("unsupported"))
    );
    assert_eq!(status["error"]["code"], "test_seam");
    let error = service
        .call("winpoint.check", &json!({}))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "test_seam");
    assert!(
        !root.0.join("cache/winpoint").exists(),
        "not even the state folder"
    );
    ctx.events.shutdown();
}

#[test]
fn one_owner_and_a_reader_that_forwards_its_check_or_says_no_owner() {
    let root = Temp::new("fwd");
    let seam = Seam::for_test(url("http://127.0.0.1:47811/"), v("0.9.0"));
    let panel = host(&root.0, "panel", false, "en", json!({}));
    let settings = host(&root.0, "settings", false, "en", json!({}));
    let owner = WinPoint::with(panel.clone(), Ok(seam.clone()), notify::Target::Off);
    let reader = WinPoint::with(settings.clone(), Ok(seam.clone()), notify::Target::Off);
    assert!(owner.take_over());
    assert_eq!(owner.mode(), Mode::Owner);
    assert!(!reader.take_over(), "one owner");
    assert_eq!(reader.mode(), Mode::Reader);
    assert_eq!(
        reader.call("winpoint.status", &json!({})).unwrap().unwrap()["ownerRunning"],
        true
    );
    // a check that just ran: the owner answers `recent` without a request (nothing listens on the origin)
    let now = crate::services::update::store::now_ms();
    owner
        .shared()
        .unwrap()
        .write_state(json!({"checkedAt": now, "result": "noReleases"}));
    let answer = reader.call("winpoint.check", &json!({})).unwrap().unwrap();
    assert_eq!(answer, json!({"started":false,"skipped":"recent"}));
    assert_eq!(
        owner.call("winpoint.check", &json!({})).unwrap().unwrap(),
        answer
    );
    let status = reader.call("winpoint.status", &json!({})).unwrap().unwrap();
    assert_eq!(
        (status["state"].clone(), status["checkedAt"].clone()),
        (json!("upToDate"), json!(now))
    );
    // the owner's error reaches the reader as its own
    owner
        .shared()
        .unwrap()
        .write_state(json!({"retryAfter": now + 3_600_000, "checkedAt": now - 60_000}));
    let limited = reader.call("winpoint.check", &json!({})).unwrap().unwrap();
    assert_eq!(limited["skipped"], "rateLimited");
    owner.shutdown();
    assert_eq!(
        reader.call("winpoint.status", &json!({})).unwrap().unwrap()["ownerRunning"],
        false
    );
    let started = Instant::now();
    let error = reader
        .call("winpoint.check", &json!({}))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "noOwner");
    assert_eq!(
        error.message,
        "WinPoint can only check for updates while its taskbar is running"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(2900),
        "{:?}",
        started.elapsed()
    );
    // the lock is free again: the next panel host takes over
    assert!(reader.take_over());
    for ctx in [panel, settings] {
        ctx.events.shutdown();
    }
}

/// A state folder or lock file that is not this user's alone (here: 0755, then a 0644 check.lock) is no seam failure:
/// the status says which (`cache`, with the folder), a check says it at once instead of `noOwner` after 3 s or a
/// silent no-op, an automatic check stores it, and the service recovers once the mode is right (no restart).
#[test]
fn a_state_folder_that_is_not_private_is_said_and_retried_not_a_seam_failure() {
    let root = Temp::new("cachedir");
    let dir = root.0.join("cache/winpoint");
    std::fs::create_dir_all(&dir).unwrap();
    let mode = |path: &Path, mode: u32| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    mode(&dir, 0o755);
    let seam = Seam::for_test(url("http://127.0.0.1:47811/"), v("0.9.0"));
    let panel = host(&root.0, "panel", false, "en", json!({}));
    let settings = host(&root.0, "settings", false, "nl", json!({}));
    let owner = WinPoint::with(panel.clone(), Ok(seam.clone()), notify::Target::Off);
    let reader = WinPoint::with(settings.clone(), Ok(seam), notify::Target::Off);
    assert_eq!(owner.mode(), Mode::Reader, "not unsupported");
    assert!(owner.shared().is_none());
    assert!(!owner.take_over(), "no owner without the folder");
    let status = owner.call("winpoint.status", &json!({})).unwrap().unwrap();
    assert_eq!(
        (
            status["mode"].clone(),
            status["state"].clone(),
            status["error"]["code"].clone(),
            status["ownerRunning"].clone()
        ),
        (
            json!("dev"),
            json!("checkFailed"),
            json!("cache"),
            Value::Null
        )
    );
    assert_ne!(status["error"]["key"], "update.err.testSeam");
    assert_eq!(
        status["error"]["message"],
        format!(
            "WinPoint can’t use its folder {}. The folder and the files in it must be yours and accessible only to you.",
            dir.display()
        )
    );
    assert_eq!(status["current"], "0.9.0");
    let started = Instant::now();
    let error = reader
        .call("winpoint.check", &json!({}))
        .unwrap()
        .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.key.as_deref()),
        ("cache", Some("update.winpoint.err.cache"))
    );
    assert!(
        error.message.starts_with("WinPoint kan zijn map "),
        "{}",
        error.message
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "at once, not noOwner after 3 s"
    );
    // fixed: every host uses the folder again, and the panel host becomes the owner
    mode(&dir, 0o700);
    assert!(owner.take_over());
    assert_eq!(
        reader.call("winpoint.status", &json!({})).unwrap().unwrap()["state"],
        "unknown"
    );
    // a check.lock of another mode: said by the status and by a check, and stored by a check that runs anyway
    let lock = dir.join("check.lock");
    std::fs::write(&lock, "").unwrap();
    mode(&lock, 0o644);
    let status = reader.call("winpoint.status", &json!({})).unwrap().unwrap();
    assert_eq!(
        (status["state"].clone(), status["error"]["code"].clone()),
        (json!("checkFailed"), json!("cache"))
    );
    assert!(
        status["error"]["detail"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{}: ", lock.display())),
        "{}",
        status["error"]
    );
    assert_eq!(status["ownerRunning"], true);
    let error = owner
        .call("winpoint.check", &json!({}))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "cache");
    assert!(
        error.data.as_ref().unwrap()["detail"]
            .as_str()
            .unwrap()
            .contains("check.lock"),
        "{:?}",
        error.data
    );
    let shared = owner.shared().unwrap();
    check::run(&shared, Trigger::Manual);
    let state = shared.read_state();
    assert_eq!(
        (state["result"].clone(), state["error"]["code"].clone()),
        (json!("error"), json!("cache")),
        "the check that found it says so (it used to return without a trace)"
    );
    mode(&lock, 0o600);
    let status = reader.call("winpoint.status", &json!({})).unwrap().unwrap();
    assert_eq!(
        (status["state"].clone(), status["error"]["code"].clone()),
        (json!("checkFailed"), json!("cache")),
        "the last check's result, until the next check"
    );
    owner.shutdown();
    for ctx in [panel, settings] {
        ctx.events.shutdown();
    }
}

#[test]
fn the_offline_answer_of_a_manual_check_is_the_last_result() {
    let root = Temp::new("offline");
    let ctx = host(&root.0, "panel", false, "nl", json!({}));
    let service = WinPoint::with(
        ctx.clone(),
        Ok(Seam::for_test(url("http://127.0.0.1:47811/"), v("0.9.0"))),
        notify::Target::Off,
    );
    assert!(service.take_over());
    *service.shared().unwrap().net.lock().unwrap() = Some(Net::from_nm(Some(1), None));
    let answer = service.call("winpoint.check", &json!({})).unwrap().unwrap();
    assert_eq!(answer, json!({"started":false,"skipped":"offline"}));
    let status = service
        .call("winpoint.status", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(status["state"], "offline");
    assert_eq!(status["error"]["message"], "Geen internetverbinding");
    assert_eq!(status["checkedAt"], Value::Null, "no request was made");
    service.shutdown();
    ctx.events.shutdown();
}

// ------------------------------------------------------------------------------------------------ seam

#[test]
fn the_seam_file_is_exactly_a_loopback_origin_a_version_and_the_tarballs_digest() {
    let hex = "ab".repeat(32);
    let seam = |origin: &str, running: &str, digest: &str| {
        format!(r#"{{"origin":"{origin}","running":"{running}","digest":"{digest}"}}"#).into_bytes()
    };
    let good = seam("http://127.0.0.1:47811/", "0.8.9", &format!("sha256:{hex}"));
    assert_eq!(
        seam::parse_seam(&good).unwrap(),
        (url("http://127.0.0.1:47811/"), v("0.8.9"), [0xab; 32])
    );
    let digest = format!("sha256:{hex}");
    let mut bad: Vec<Vec<u8>> = [
        "http://127.0.0.1:80/",
        "http://127.0.0.1:65536/",
        "http://127.0.0.1:047811/",
        "http://localhost:47811/",
        "https://127.0.0.1:47811/",
        "http://127.0.0.1:47811",
        "http://127.0.0.1:47811/x",
        "http://10.0.0.1:47811/",
    ]
    .iter()
    .map(|origin| seam(origin, "0.8.9", &digest))
    .collect();
    bad.push(seam("http://127.0.0.1:47811/", "x", &digest));
    // the digest is GitHub's own form of the run's one tarball: lowercase, sha256, 64 digits
    for wrong in [
        hex.clone(),
        format!("sha256:{}", hex.to_uppercase()),
        format!("sha256:{}", &hex[..62]),
        format!("sha512:{hex}"),
        String::new(),
    ] {
        bad.push(seam("http://127.0.0.1:47811/", "0.8.9", &wrong));
    }
    for raw in [
        &br#"{"origin":"http://127.0.0.1:47811/","running":"0.8.9"}"#[..],
        br#"{"origin":"http://127.0.0.1:47811/","running":"0.8.9","digest":null}"#,
        br#"["http://127.0.0.1:47811/"]"#,
        b"",
    ] {
        bad.push(raw.to_vec());
    }
    let mut extra = String::from_utf8(good.clone()).unwrap();
    extra.insert_str(extra.len() - 1, r#","x":1"#);
    bad.push(extra.into_bytes());
    for bad in bad {
        assert!(
            seam::parse_seam(&bad).is_err(),
            "{}",
            String::from_utf8_lossy(&bad)
        );
    }
}

/// D-G74: in test mode the job trusts only the tarball seam.json pins, not whoever listens on the loopback port (a
/// port of 1024 or more is anyone's); production trusts GitHub's digest; only a unit test's own seam pins none.
#[test]
fn a_test_seam_trusts_only_the_tarball_it_pins() {
    let origin = url("http://127.0.0.1:47811/");
    let (ours, theirs) = ([1u8; 32], [2u8; 32]);
    let pinned = Seam::pinned(origin.clone(), v("0.8.9"), ours);
    assert!(pinned.trusts(&ours));
    assert!(
        !pinned.trusts(&theirs),
        "another listener's release, however well it agrees with itself"
    );
    assert!(Seam::Production.trusts(&theirs));
    assert!(Seam::for_test(origin.clone(), v("0.8.9")).trusts(&theirs));
    let unpinned = Seam::Test {
        origin,
        running: v("0.8.9"),
        digest: None,
        dir: Some(PathBuf::from("/tmp/w11-winpoint")),
    };
    assert!(!unpinned.trusts(&ours), "a seam from a folder always pins");
}

#[test]
fn the_seam_directory_must_be_private_marked_and_ours() {
    use std::os::unix::fs::{MetadataExt, symlink};
    let root = Temp::new("seamdir");
    let euid = std::fs::metadata(&root.0).unwrap().uid();
    let dir = root.0.join("seam");
    std::fs::create_dir(&dir).unwrap();
    let mode = |path: &Path, mode: u32| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    let check = |path: &Path| Seam::check(Some(path.as_os_str()), euid);
    assert_eq!(Seam::check(None, euid), Ok(Seam::Production));
    mode(&dir, 0o755);
    assert!(check(&dir).is_err(), "0755");
    mode(&dir, 0o700);
    assert!(check(&dir).is_err(), "no marker");
    std::fs::write(dir.join(crate::services::update::paths::MARKER), "").unwrap();
    mode(&dir.join(crate::services::update::paths::MARKER), 0o664);
    assert!(check(&dir).is_err(), "a group-writable marker");
    mode(&dir.join(crate::services::update::paths::MARKER), 0o644);
    assert!(check(&dir).is_err(), "no seam.json");
    std::fs::write(
        dir.join("seam.json"),
        r#"{"origin":"http://127.0.0.1:47811/","running":"0.8.9"}"#,
    )
    .unwrap();
    mode(&dir.join("seam.json"), 0o644);
    assert!(
        check(&dir).is_err(),
        "no digest: a seam from a folder pins its tarball"
    );
    std::fs::write(
        dir.join("seam.json"),
        format!(
            r#"{{"origin":"http://127.0.0.1:47811/","running":"0.8.9","digest":"sha256:{}"}}"#,
            "0f".repeat(32)
        ),
    )
    .unwrap();
    let Ok(Seam::Test {
        origin,
        running,
        digest,
        dir: Some(found),
    }) = check(&dir)
    else {
        panic!("{:?}", check(&dir))
    };
    assert_eq!(
        (origin.as_str(), running.to_string(), digest, found),
        (
            "http://127.0.0.1:47811/",
            "0.8.9".into(),
            Some([0x0f; 32]),
            dir.clone()
        )
    );
    // no key file of any kind (D-G74: no signing key): another file in the folder changes nothing
    std::fs::write(dir.join("allowed_signers"), "").unwrap();
    mode(&dir.join("allowed_signers"), 0o644);
    assert!(check(&dir).is_ok());
    assert!(Seam::check(Some(dir.as_os_str()), 0).is_err(), "euid 0");
    assert!(Seam::check(Some(std::ffi::OsStr::new("relative/seam")), euid).is_err());
    let link = root.0.join("link");
    symlink(&dir, &link).unwrap();
    assert!(check(&link).is_err(), "a symlink");
    assert!(
        Seam::check(Some(dir.as_os_str()), euid + 1).is_err(),
        "someone else's"
    );
    // what a test seam changes
    let Ok(test) = check(&dir) else {
        unreachable!()
    };
    assert_eq!(test.origin(), Origin::test(&url("http://127.0.0.1:47811/")));
    assert!(matches!(test.client().policy(), Policy::Test { .. }));
    assert_eq!(test.running(), Some(&v("0.8.9")));
    assert_eq!(Seam::Production.origin(), Origin::github());
    assert!(matches!(
        Seam::Production.client().policy(),
        Policy::Production
    ));
    assert_eq!(Seam::Production.running(), None);
}

// ------------------------------------------------------------------------------------------------ notification

/// The sandbox's private session bus (tests/rust-sandbox/run-tests.sh: dbus-run-session in its own TMPDIR), or
/// None: never the live session's.
fn private_session_bus() -> Option<String> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()?;
    let private = std::env::var_os("SANDBOX").is_some()
        && !address.contains("/run/user/")
        && address.starts_with(&format!("unix:path={}/", std::env::temp_dir().display()));
    private.then_some(address)
}

const NOTIFICATIONS_XML: &str = r#"<node><interface name="org.freedesktop.Notifications">
  <method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
    <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/>
    <arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
  <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
  <signal name="ActivationToken"><arg type="u"/><arg type="s"/></signal>
  <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
</interface></node>"#;

#[test]
fn the_notification_opens_update_and_recovery_with_its_activation_token() {
    let Some(address) = private_session_bus() else {
        eprintln!("skip: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let name = format!(
        "io.github.win11shell.TestNotifications.p{}",
        std::process::id()
    );
    let (calls, called) = mpsc::channel::<glib::Variant>();
    let done = Arc::new(AtomicBool::new(false));
    let (ready, is_ready) = mpsc::channel();
    let server = {
        let (address, name, done) = (address.clone(), name.clone(), done.clone());
        std::thread::spawn(move || {
            let context = glib::MainContext::new();
            context
                .with_thread_default(|| {
                    let conn = gio::DBusConnection::for_address_sync(
                        &address,
                        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                        None,
                        gio::Cancellable::NONE,
                    )
                    .unwrap();
                    let node = gio::DBusNodeInfo::for_xml(NOTIFICATIONS_XML).unwrap();
                    let iface = node
                        .lookup_interface("org.freedesktop.Notifications")
                        .unwrap();
                    let calls = Mutex::new(calls);
                    let count = std::sync::atomic::AtomicU32::new(0);
                    let _registration = conn
                        .register_object(
                            "/org/freedesktop/Notifications",
                            &iface,
                            move |conn, sender, _, _, _, params, invocation| {
                                let n = count.fetch_add(1, Ordering::SeqCst);
                                let _ = calls.lock().unwrap().send(params.clone());
                                invocation.return_value(Some(&(7u32,).to_variant()));
                                let emit = |member: &str, args: glib::Variant| {
                                    let _ = conn.emit_signal(
                                        Some(sender),
                                        "/org/freedesktop/Notifications",
                                        "org.freedesktop.Notifications",
                                        member,
                                        Some(&args),
                                    );
                                };
                                if n == 0 {
                                    // someone else's notification first: never ours
                                    emit("ActionInvoked", (8u32, "view").to_variant());
                                    emit("ActivationToken", (7u32, "token-1").to_variant());
                                    emit("ActionInvoked", (7u32, "view").to_variant());
                                } else {
                                    emit("NotificationClosed", (7u32, 2u32).to_variant());
                                }
                            },
                            |_, _, _, _, _| unreachable!("no properties"),
                            |_, _, _, _, _, _| false,
                        )
                        .unwrap();
                    let owned = conn
                        .call_sync(
                            Some("org.freedesktop.DBus"),
                            "/org/freedesktop/DBus",
                            "org.freedesktop.DBus",
                            "RequestName",
                            Some(&(name.as_str(), 4u32).to_variant()),
                            None,
                            gio::DBusCallFlags::NONE,
                            5000,
                            gio::Cancellable::NONE,
                        )
                        .unwrap();
                    assert_eq!(owned.child_value(0).get::<u32>(), Some(1), "primary owner");
                    ready.send(()).unwrap();
                    while !done.load(Ordering::SeqCst) {
                        while context.iteration(false) {}
                        std::thread::sleep(Duration::from_millis(10));
                    }
                })
                .unwrap();
        })
    };
    is_ready.recv_timeout(Duration::from_secs(10)).unwrap();
    let notifier = notify::Notifier::new(notify::Target::Bus { address, name });
    let (opened, was_opened) = mpsc::channel();
    let note = notify::Note {
        title: "WinPoint 0.9.2 is available".into(),
        body: "Open Settings › Update & recovery to install it.".into(),
        action: "View".into(),
    };
    let open: notify::Open = Arc::new(move |token| {
        let _ = opened.send(token);
    });
    notifier.show(note.clone(), open.clone());
    let params = called.recv_timeout(Duration::from_secs(10)).unwrap();
    let (app, replaces, icon, title, body, actions, hints, expire) = params
        .get::<(
            String,
            u32,
            String,
            String,
            String,
            Vec<String>,
            glib::VariantDict,
            i32,
        )>()
        .unwrap();
    assert_eq!(
        (app.as_str(), replaces, icon.as_str()),
        ("WinPoint", 0, "system-software-update")
    );
    assert_eq!(
        (title.as_str(), body.as_str(), expire),
        (note.title.as_str(), note.body.as_str(), -1)
    );
    assert_eq!(actions, ["default", "", "view", "View"]);
    assert_eq!(
        hints
            .lookup_value("desktop-entry", None)
            .and_then(|v| v.str().map(str::to_owned))
            .as_deref(),
        Some("io.github.win11shell.Settings")
    );
    assert_eq!(
        was_opened
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .as_deref(),
        Some("token-1")
    );
    // the next version replaces the shown notification; closing it opens nothing
    notifier.show(
        notify::Note {
            title: "WinPoint 0.9.3 is available".into(),
            ..note
        },
        open,
    );
    let params = called.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(params.child_value(1).get::<u32>(), Some(7), "replaces_id");
    assert!(
        was_opened
            .recv_timeout(Duration::from_millis(1500))
            .is_err()
    );
    notifier.shutdown();
    done.store(true, Ordering::SeqCst);
    server.join().unwrap();
}

#[test]
fn a_release_without_a_download_for_this_pc_is_shown_but_not_announced() {
    let root = Temp::new("noasset");
    let ctx = host(&root.0, "panel", false, "en", json!({}));
    let all = feed::parse_releases(RELEASES.as_bytes(), &Origin::github(), "x86_64").unwrap();
    for (running, shown, notified) in [("0.9.2", 0, Value::Null), ("0.9.0", 1, json!("0.9.2"))] {
        let seam = Seam::for_test(url("http://127.0.0.1:47811/"), v(running));
        let service = WinPoint::with(ctx.clone(), Ok(seam), notify::Target::Off);
        let shared = service.shared().unwrap();
        check::record(
            &shared,
            &json!({}),
            &Fetched::Releases(all.clone()),
            Trigger::Auto,
        );
        let state = shared.store.read(check::STATE).unwrap();
        assert_eq!(
            shared.notifier.shown.lock().unwrap().len(),
            shown,
            "{running}"
        );
        assert_eq!(state["notified"], notified, "{running}");
        let status = service
            .call("winpoint.status", &json!({}))
            .unwrap()
            .unwrap();
        assert_eq!(status["state"], "available");
        if running == "0.9.2" {
            assert_eq!(
                (
                    status["latest"]["version"].clone(),
                    status["latest"]["reason"].clone()
                ),
                (json!("0.9.7"), json!("noAsset"))
            );
        }
    }
    ctx.events.shutdown();
}

// ------------------------------------------------------------------------------------------------ verify (D-G74)

#[test]
fn the_sha256_file_is_exactly_one_line_for_the_exact_tarball() {
    use super::verify::{self, VerifyError};
    let name = "winpoint-0.9.1-linux-x86_64.tar.gz";
    let hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let good = format!("{hex}  {name}\n");
    let hash = verify::parse_sha256_file(good.as_bytes(), name).unwrap();
    assert_eq!(verify::to_hex(&hash), hex);
    assert_eq!(
        verify::parse_sha256_file(good.as_bytes(), "winpoint-0.9.1-linux-aarch64.tar.gz"),
        Err(VerifyError::Name)
    );
    for bad in [
        format!("{}  {name}\n", hex.to_uppercase()),
        format!("{hex}  {name}"),
        format!("{hex} {name}\n"),
        format!("{hex}  {name}\n\n"),
        format!("{hex}  {name}\r\n"),
        format!("{hex}0  {name}\n"),
        format!("{}  {name}\n", &hex[1..]),
        format!(" {hex}  {name}\n"),
        format!("{hex}  ./{name}\n"),
        format!("{hex}  \n"),
        String::new(),
    ] {
        assert_eq!(
            verify::parse_sha256_file(bad.as_bytes(), name),
            Err(VerifyError::Format),
            "{bad:?}"
        );
    }
    assert!(verify::parse_sha256_file(b"\xff\xfe", name).is_err());
    // GitHub's digest field, and SHA-256 through GLib
    assert_eq!(
        verify::parse_digest(&format!("sha256:{hex}")).map(|h| verify::to_hex(&h)),
        Some(hex.to_owned())
    );
    for bad in [
        hex.to_owned(),
        format!("sha512:{hex}"),
        format!("sha256:{}", hex.to_uppercase()),
    ] {
        assert_eq!(verify::parse_digest(&bad), None, "{bad}");
    }
    let mut hasher = verify::Hasher::new();
    hasher.update(b"a");
    hasher.update(b"bc");
    assert_eq!(verify::to_hex(&hasher.finish()), hex);
    let root = Temp::new("hash");
    let file = root.0.join("abc");
    std::fs::write(&file, b"abc").unwrap();
    let (hash, size) = verify::hash_file(&file, &AtomicBool::new(false)).unwrap();
    assert_eq!((verify::to_hex(&hash), size), (hex.to_owned(), 3));
    assert!(
        verify::hash_file(&file, &AtomicBool::new(true)).is_err(),
        "Cancel"
    );
}

// ------------------------------------------------------------------------------------------------ the job's rules

#[test]
fn the_bundles_version_is_read_from_its_workspace_table_only() {
    use super::job::workspace_version;
    let cargo = "[workspace]\nmembers = [\"host\"]\nversion = \"1.0.0\"\n\n[workspace.package]\nedition = \"2024\"\n\
                 version = \"0.9.1\"\n\n[package]\nversion = \"0.1.0\"\n";
    assert_eq!(workspace_version(cargo).as_deref(), Some("0.9.1"));
    assert_eq!(workspace_version("[package]\nversion = \"0.9.1\"\n"), None);
    assert_eq!(
        workspace_version("[workspace.package]\nversion=\"0.9.2\"\n").as_deref(),
        Some("0.9.2")
    );
}

/// D-G74: SIGTERM once the installer has run 15 minutes, SIGKILL 3 minutes after the SIGTERM the watch sent itself.
/// A new owner keeps the installer's own clock (its age at the watch's start), and one that first sees it past both
/// still sends SIGTERM first, so install.py rolls back. After the start only the watch's monotonic time moves the
/// clock, so a suspend is left out (a real suspend cannot be made in a test; job.rs reads the kernel's age once).
#[test]
fn the_installers_clock_sends_sigterm_first_and_sigkill_only_after_it() {
    use super::job::Clock;
    let min = |m: u64| Duration::from_secs(m * 60);
    let (timeout, kill) = (min(15), min(3));
    let mut own = Clock::new(Duration::ZERO, timeout, kill);
    assert_eq!(own.due(min(15) - Duration::from_millis(1)), None);
    assert_eq!(own.due(min(15)), Some(libc::SIGTERM));
    assert_eq!(own.due(min(16)), None, "once");
    assert_eq!(own.due(min(18) - Duration::from_millis(1)), None);
    assert_eq!(own.due(min(18)), Some(libc::SIGKILL));
    for (age, term_at) in [
        // before the timeout: the installer's clock, not one of its own
        (min(10), min(5)),
        // between the two (the last owner sent SIGTERM at 15) and long past both: its own SIGTERM at once
        (min(16), Duration::ZERO),
        (min(25), Duration::ZERO),
        (min(24 * 60), Duration::ZERO),
    ] {
        let mut next = Clock::new(age, timeout, kill);
        if let Some(before) = term_at.checked_sub(Duration::from_millis(1)) {
            assert_eq!(next.due(before), None, "{age:?}");
        }
        assert_eq!(
            next.due(term_at),
            Some(libc::SIGTERM),
            "{age:?}: SIGTERM first"
        );
        assert_eq!(
            next.due(term_at + kill - Duration::from_millis(1)),
            None,
            "{age:?}"
        );
        assert_eq!(next.due(term_at + kill), Some(libc::SIGKILL), "{age:?}");
    }
}

#[test]
fn a_job_shows_its_own_state_only_while_its_processes_live() {
    use super::job::effective_state;
    let me = crate::services::update::store::current_owner();
    let dead = json!({"pid": 999_999_999u32, "start": "1"});
    for (state, owner, installer, shown) in [
        ("downloading", &me, &Value::Null, "downloading"),
        ("preparing", &me, &Value::Null, "preparing"),
        ("downloading", &dead, &Value::Null, "failed"),
        ("unpacking", &dead, &Value::Null, "failed"),
        ("installing", &me, &dead, "installing"),
        ("installing", &dead, &me, "installing"),
        ("installing", &dead, &dead, "interrupted"),
        ("installed", &dead, &dead, "installed"),
        ("failed", &dead, &dead, "failed"),
        ("cancelled", &me, &Value::Null, "cancelled"),
        ("nonsense", &me, &Value::Null, ""),
    ] {
        let job = json!({"state": state, "owner": owner, "installer": installer});
        assert_eq!(effective_state(&job), shown, "{job}");
    }
    // installing with no installer pid yet (its owner died while it started one): installing while that start is
    // recent or a process with its frozen argv runs, else interrupted
    let now = crate::services::update::store::now_ms();
    for (asked, shown) in [(now - 1_000, "installing"), (now - 120_000, "interrupted")] {
        let job = json!({"state":"installing","owner":dead,"installingAt":asked,
                         "installer":{"pid":null,"start":null,"script":"/nowhere/scripts/install.py"}});
        assert_eq!(effective_state(&job), shown, "{job}");
    }
    // the guards read the same file without creating anything
    let root = Temp::new("guards");
    let cache = root.0.join("cache");
    assert!(!super::job::job_active(&cache) && !super::job::installing(&cache));
    assert!(!super::job::relogin_pending(&cache));
    assert!(!cache.exists());
    let store = crate::services::update::store::Store::open(cache.join("winpoint")).unwrap();
    let key = super::job::session_key();
    for (job, active, installing, relogin) in [
        (
            json!({"state":"downloading","owner":me}),
            true,
            false,
            false,
        ),
        (
            json!({"state":"downloading","owner":dead}),
            false,
            false,
            false,
        ),
        (
            json!({"state":"installing","owner":dead,"installer":me}),
            true,
            true,
            false,
        ),
        (
            json!({"state":"installing","owner":dead,"installer":dead}),
            false,
            false,
            false,
        ),
        (
            json!({"state":"installed","restart":"relogin","sessionKey":key}),
            false,
            false,
            true,
        ),
        (
            json!({"state":"installed","restart":"relogin","sessionKey":"other"}),
            false,
            false,
            false,
        ),
        (
            json!({"state":"installed","restart":"restart","sessionKey":key}),
            false,
            false,
            false,
        ),
    ] {
        store.write(super::job::JOB, &job).unwrap();
        assert_eq!(
            (
                super::job::job_active(&cache),
                super::job::installing(&cache),
                super::job::relogin_pending(&cache)
            ),
            (active, installing, relogin),
            "{job}"
        );
    }
}

#[test]
fn the_status_shows_a_job_until_a_check_or_a_newer_version_makes_it_old() {
    let current = v("0.9.0");
    let all = feed::parse_releases(RELEASES.as_bytes(), &Origin::github(), "x86_64").unwrap();
    let ok = check::next_state(&json!({}), &Fetched::Releases(all), 1_000, &current, 0);
    let me = crate::services::update::store::current_owner();
    let dead = json!({"pid": 999_999_999u32, "start": "1"});
    let shown = |job: &Value, state: &Value| {
        let mut w = view(state, &current);
        w.job = Some(job);
        compose(&w)
    };
    let error = json!({"code":"installer","key":"update.winpoint.err.installer","params":{"code":"3"},"detail":""});
    let failed = json!({"jobId":"j","version":"0.9.2","state":"failed","finishedAt":2_000,"outcome":"rolledBack",
                        "error":error});
    let out = shown(&failed, &ok);
    assert_eq!(out["state"], "failed");
    assert_eq!(
        out["job"]["error"]["message"],
        "The installer stopped (code 3)"
    );
    assert_eq!(
        out["job"]["outcomeMessage"],
        "Your previous version was restored."
    );
    // a check after the failure: the card offers the release again
    let mut later = ok.clone();
    later["checkedAt"] = json!(3_000);
    assert_eq!(
        (
            shown(&failed, &later)["state"].clone(),
            shown(&failed, &later)["job"].clone()
        ),
        (json!("available"), Value::Null)
    );
    // a running job wins over checking and over a failed check
    let running =
        json!({"jobId":"j","version":"0.9.2","state":"downloading","done":5,"total":10,"owner":me});
    let mut w = view(&later, &current);
    w.job = Some(&running);
    w.checking = true;
    let out = compose(&w);
    assert_eq!(
        (
            out["state"].clone(),
            out["job"]["done"].clone(),
            out["job"]["canCancel"].clone()
        ),
        (json!("downloading"), json!(5), json!(true))
    );
    // its owner died before the installer: failed, nothing changed
    let orphan = json!({"jobId":"j","version":"0.9.2","state":"unpacking","owner":dead});
    let out = shown(&orphan, &ok);
    assert_eq!(out["state"], "failed");
    assert_eq!(
        out["job"]["error"]["message"],
        "WinPoint stopped during the update"
    );
    assert_eq!(out["job"]["outcomeMessage"], "Nothing was changed.");
    // installed stays until it is cleared, while the payload names its version (not once another install took its
    // place); interrupted until that version is no longer newer
    let installed = json!({"jobId":"j","version":"0.9.2","state":"installed","restart":"relogin"});
    let mut w = view(&later, &current);
    w.job = Some(&installed);
    w.installed = Some("0.9.2".into());
    assert_eq!(compose(&w)["state"], "installed");
    w.installed = Some("0.9.3-dev".into());
    assert_eq!(
        compose(&w)["state"],
        "available",
        "another install took its place"
    );
    let interrupted =
        json!({"jobId":"j","version":"0.9.2","state":"installing","owner":dead,"installer":dead});
    assert_eq!(shown(&interrupted, &later)["state"], "interrupted");
    let installed_now = v("0.9.2");
    let mut newer = view(&later, &installed_now);
    newer.job = Some(&interrupted);
    assert_eq!(compose(&newer)["state"], "upToDate");
    // an interrupted 0.9.2 gives way once another release is offered (the card installs that one), never to nothing
    let mut other = later.clone();
    other["latest"]["version"] = json!("0.9.3");
    other["latest"]["tag"] = json!("v0.9.3");
    let out = shown(&interrupted, &other);
    assert_eq!(
        (out["state"].clone(), out["latest"]["version"].clone()),
        (json!("available"), json!("0.9.3"))
    );
    let mut none = later.clone();
    none["latest"] = Value::Null;
    assert_eq!(shown(&interrupted, &none)["state"], "interrupted");
    // the job's rev counts for the status's rev
    let mut revved = installed.clone();
    revved["rev"] = json!(99);
    assert_eq!(shown(&revved, &ok)["rev"], 99);
}

/// The version this host compares with follows the payload's VERSION only once WinPoint's own install of it ended
/// `installed` (design 5.1): after `interrupted`, `failed` with the outcome `unknown`, and while or after its Try
/// again (at its click the payload or this host said V already, job.json `found`), the payload or the binaries may
/// name V while the install is half done, so V is still offered and Try again rolls forward. A job that changed
/// nothing of what its click found does not keep V from counting once V was installed by hand (scripts/install.sh).
#[test]
fn a_payload_whose_install_did_not_finish_is_not_the_installed_version() {
    let dead = json!({"pid": 999_999_999u32, "start": "1"});
    let (own, payload) = (v("0.9.0"), v("0.9.2"));
    // `found`: the higher of the host's own version and the payload's VERSION at the job's click
    let record = |state: &str, outcome: &str, found: &str| {
        let mut job = json!({"version":"0.9.2","owner":dead,"installer":dead,"found":found});
        job["state"] = json!(state);
        job["outcome"] = json!(outcome);
        job
    };
    for (job, current) in [
        (None, "0.9.2"),
        (Some(record("installed", "", "0.9.0")), "0.9.2"),
        (Some(record("interrupted", "unknown", "0.9.0")), "0.9.0"),
        (Some(record("installing", "", "0.9.0")), "0.9.0"),
        (Some(record("failed", "unknown", "0.9.0")), "0.9.0"),
        // a Try again of such an install, while it runs (here its owner died) or after it ended without installing
        (Some(record("downloading", "", "0.9.2")), "0.9.0"),
        (
            Some(record("cancelled", "nothingChanged", "0.9.2")),
            "0.9.0",
        ),
        (Some(record("failed", "nothingChanged", "0.9.2")), "0.9.0"),
        (Some(record("failed", "rolledBack", "0.9.2")), "0.9.0"),
        // a job that changed nothing of the payload it found (0.9.0): 0.9.2 came by hand since
        (Some(record("downloading", "", "0.9.0")), "0.9.2"),
        (
            Some(record("cancelled", "nothingChanged", "0.9.0")),
            "0.9.2",
        ),
        (Some(record("failed", "nothingChanged", "0.9.0")), "0.9.2"),
        (Some(record("failed", "rolledBack", "0.9.0")), "0.9.2"),
        (
            Some(json!({"version":"0.9.1","state":"interrupted"})),
            "0.9.2",
        ),
    ] {
        assert_eq!(
            super::current_from(own.clone(), Some(payload.clone()), job.as_ref()).to_string(),
            current,
            "{job:?}"
        );
    }
    assert_eq!(super::current_from(own.clone(), None, None), own);
    assert_eq!(
        super::current_from(v("0.9.3"), Some(payload.clone()), None),
        v("0.9.3")
    );
    // install.py swapped the binaries too before it stopped: this host runs 0.9.2 itself, and the version the job
    // started from (`from`) is WinPoint's until the install ends `installed`; a record without one keeps the old rule
    let from = |state: &str, outcome: &str, found: &str| {
        let mut job = record(state, outcome, found);
        job["from"] = json!("0.9.1");
        job
    };
    for (job, installed, current) in [
        (
            from("interrupted", "unknown", "0.9.1"),
            Some(payload.clone()),
            "0.9.1",
        ),
        (
            from("failed", "unknown", "0.9.1"),
            Some(payload.clone()),
            "0.9.1",
        ),
        (
            from("downloading", "", "0.9.2"),
            Some(payload.clone()),
            "0.9.1",
        ),
        (
            from("failed", "rolledBack", "0.9.2"),
            Some(payload.clone()),
            "0.9.1",
        ),
        (
            from("interrupted", "unknown", "0.9.1"),
            Some(v("0.9.1")),
            "0.9.1",
        ),
        (from("interrupted", "unknown", "0.9.1"), None, "0.9.1"),
        // a Try again of that install after a rollback that put the payload back (0.9.1, or none) but left 0.9.2's
        // binaries: its click found 0.9.2 (this host's own version), so it is still a Try again of 0.9.2
        (from("downloading", "", "0.9.2"), Some(v("0.9.1")), "0.9.1"),
        (from("downloading", "", "0.9.2"), None, "0.9.1"),
        (
            from("failed", "nothingChanged", "0.9.2"),
            Some(v("0.9.1")),
            "0.9.1",
        ),
        (
            from("installed", "", "0.9.1"),
            Some(payload.clone()),
            "0.9.2",
        ),
        (
            record("interrupted", "unknown", "0.9.1"),
            Some(payload.clone()),
            "0.9.2",
        ),
        // a job that changed nothing (its owner died while it downloaded, a Cancel, `manual` from --check, a
        // rollback to the payload it found), then 0.9.2 by hand (scripts/install.sh restarts WinPoint)
        (
            from("downloading", "", "0.9.1"),
            Some(payload.clone()),
            "0.9.2",
        ),
        (
            from("cancelled", "nothingChanged", "0.9.1"),
            Some(payload.clone()),
            "0.9.2",
        ),
        (
            from("failed", "nothingChanged", "0.9.1"),
            Some(payload.clone()),
            "0.9.2",
        ),
        (
            from("failed", "rolledBack", "0.9.1"),
            Some(payload.clone()),
            "0.9.2",
        ),
    ] {
        assert_eq!(
            super::current_from(payload.clone(), installed.clone(), Some(&job)).to_string(),
            current,
            "{job} {installed:?}"
        );
    }
    // with the pre-install host (0.9.0) `from` counts too: the payload's last confirmed version
    assert_eq!(
        super::current_from(
            own.clone(),
            Some(payload.clone()),
            Some(&from("interrupted", "unknown", "0.9.1"))
        ),
        v("0.9.1")
    );
    // the card then keeps the job with Try again
    let all = feed::parse_releases(RELEASES.as_bytes(), &Origin::github(), "x86_64").unwrap();
    let mut state = check::next_state(&json!({}), &Fetched::Releases(all), 1, &own, 0);
    state["latest"]["assets"]["tarball"]["digest"] = json!(format!("sha256:{}", "a".repeat(64)));
    let cut = record("interrupted", "unknown", "0.9.0");
    let mut w = view(&state, &own);
    w.job = Some(&cut);
    w.installed = Some("0.9.2".into());
    let out = compose(&w);
    assert_eq!(
        (
            out["state"].clone(),
            out["job"]["canRetry"].clone(),
            out["latest"]["canInstall"].clone()
        ),
        (json!("interrupted"), json!(true), json!(true)),
        "{out}"
    );
    // the same once this host runs 0.9.2's binaries: it compares with the job's `from`, so the card keeps the job
    let mut cut = from("interrupted", "unknown", "0.9.1");
    cut["jobId"] = json!("j");
    let current = super::current_from(payload.clone(), Some(payload.clone()), Some(&cut));
    let mut w = view(&state, &current);
    w.job = Some(&cut);
    w.installed = Some("0.9.2".into());
    let out = compose(&w);
    assert_eq!(
        (
            out["state"].clone(),
            out["current"].clone(),
            out["job"]["canRetry"].clone(),
            out["latest"]["version"].clone()
        ),
        (
            json!("interrupted"),
            json!("0.9.1"),
            json!(true),
            json!("0.9.2")
        ),
        "{out}"
    );
    // --check found system files to change (`manual`: "install it with scripts/install.sh"), the user did, and
    // WinPoint runs 0.9.2 now: up to date, though job.json still holds the failed job and state.json the verdict
    let mut manual = from("failed", "nothingChanged", "0.9.1");
    manual["jobId"] = json!("m");
    manual["error"] = json!({"code":"manual","key":"update.winpoint.reason.manual",
                             "params":{"systemFiles":["session"],"version":"0.9.2"}});
    state["blocked"] = json!({"version":"0.9.2","code":"manual",
                              "params":{"systemFiles":["session"],"version":"0.9.2"},"detail":"d"});
    let current = super::current_from(payload.clone(), Some(payload.clone()), Some(&manual));
    let mut w = view(&state, &current);
    w.job = Some(&manual);
    w.installed = Some("0.9.2".into());
    let out = compose(&w);
    assert_eq!(
        (
            out["state"].clone(),
            out["current"].clone(),
            out["latest"].clone(),
            out["job"].clone()
        ),
        (json!("upToDate"), json!("0.9.2"), Value::Null, Value::Null),
        "{out}"
    );
}

#[test]
fn a_blocked_version_stays_blocked_until_another_is_offered() {
    let current = v("0.9.0");
    let all = feed::parse_releases(RELEASES.as_bytes(), &Origin::github(), "x86_64").unwrap();
    let mut state = check::next_state(&json!({}), &Fetched::Releases(all.clone()), 1, &current, 0);
    state["blocked"] = json!({"version":"0.9.2","code":"missingParts","params":{"packages":"libfoo1","version":"0.9.2"},
                              "detail":"d"});
    let again = check::next_state(&state, &Fetched::Releases(all.clone()), 2, &current, 0);
    assert_eq!(again["blocked"]["version"], "0.9.2");
    let out = compose(&view(&again, &current));
    // noDigest comes first for 0.9.2 of the fixture; with a digest the verdict shows
    assert_eq!(out["latest"]["reason"], "noDigest");
    let offline = check::next_state(&again, &Fetched::Offline, 3, &current, 0);
    assert_eq!(
        offline["blocked"]["version"], "0.9.2",
        "a failed check keeps it"
    );
    let newer = check::next_state(&again, &Fetched::Releases(all), 4, &v("0.9.2"), 0);
    assert_eq!(newer["blocked"], Value::Null, "another version is offered");
    // a manual check ("Check for updates") forgets what this PC can change (packages, GNOME, its libraries), so the
    // next click runs --check afresh; the release's own verdict (system files) and every automatic check keep it
    for (code, manual, auto) in [
        ("missingParts", false, true),
        ("incompatible", false, true),
        ("manual", true, true),
    ] {
        let blocked = json!({"version":"0.9.2","code":code});
        assert_eq!(
            (
                check::verdict_kept(&blocked, Trigger::Manual),
                check::verdict_kept(&blocked, Trigger::Auto)
            ),
            (manual, auto),
            "{code}"
        );
    }
    // the reasons' texts
    for (reason, params, text) in [
        (
            "incompatible",
            json!({"kind":"gnome","version":"1.0","needs":"51","running":"50"}),
            "WinPoint 1.0 is made for GNOME 51; this PC has GNOME 50",
        ),
        (
            "incompatible",
            json!({"kind":"abi","version":"1.0"}),
            "WinPoint 1.0 doesn’t work with the system libraries on this PC",
        ),
        (
            "missingParts",
            json!({"packages":"a b","version":"1.0"}),
            "Install these first: sudo apt install a b",
        ),
    ] {
        let (_, key) = super::job::reason_key(reason, &params);
        assert_eq!(crate::i18n::trf(Lang::En, key, &params), text);
    }
}

/// D-G74 (design 6.2): after a rollback install.py renames the original program back, so the path holds the running
/// inode again; a process whose program is " (deleted)" but whose path is that same inode needs no restart.
#[test]
fn a_program_renamed_back_by_a_rollback_is_not_a_restart() {
    use crate::services::update::{paths, restart};
    let root = Temp::new("inode");
    let bin = root.0.join("bin");
    let saved = root.0.join("tx");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&saved).unwrap();
    // a copy of bash that waits in its builtin `read` (the coreutils of 26.04 are one binary that goes by its name)
    let program = bin.join("win11-inode-test");
    std::fs::copy("/usr/bin/bash", &program).unwrap();
    let mut child = std::process::Command::new(&program)
        .args(["--norc", "-c", "read -t 30 x"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let setup = paths::resolve(&paths::Env::default()).unwrap();
    let uid = unsafe { libc::geteuid() };
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read_link(format!("/proc/{}/exe", child.id())).ok() != Some(program.clone()) {
        assert!(Instant::now() < deadline, "the copy did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    // the installer's two phases: keep the running inode, put a new file in its place
    std::fs::hard_link(&program, saved.join("win11-inode-test")).unwrap();
    let new = bin.join(".win11-inode-test.new");
    std::fs::copy("/usr/bin/bash", &new).unwrap();
    std::fs::rename(&new, &program).unwrap();
    let replaced = restart::scan(&setup.paths, uid, &bin);
    assert!(replaced.shell_parts.contains(&"self"), "{replaced:?}");
    // the rollback renames the original back: the same inode as the one running
    std::fs::rename(saved.join("win11-inode-test"), &program).unwrap();
    let exe = std::fs::read_link(format!("/proc/{}/exe", child.id())).unwrap();
    assert!(
        exe.to_string_lossy().ends_with(" (deleted)"),
        "{}",
        exe.display()
    );
    let back = restart::scan(&setup.paths, uid, &bin);
    assert!(!back.shell_parts.contains(&"self"), "{back:?}");
    let _ = child.kill();
    let _ = child.wait();
}
