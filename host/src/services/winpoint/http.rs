//! A minimal HTTP/1.1 GET client over GIO for WinPoint's update check (D-G74a): `gio::SocketClient`
//! with TLS from glib-networking, no HTTP library. It does GET with `Connection: close` and nothing else.
//!
//! * **Threads.** Every call blocks: it runs on a worker thread of its own (`winpoint-check`) and uses only
//!   blocking GIO calls, each with the call's `gio::Cancellable`. A watchdog thread per call ([`Client::get`])
//!   cancels it when `stop` is set (the host shuts down: [`HttpError::Cancelled`]), when the deadline has passed or
//!   when no byte arrived for `stall` ([`HttpError::Timeout`]). Cancelling from another thread also ends a blocked
//!   TLS read.
//! * **TLS.** Certificate and host-name validation are glib-networking's (the system trust store, the URI's host as
//!   the server identity), the proxy is GIO's resolver. This file never weakens either: the TLS test and a grep test
//!   in tests.rs hold it to that.
//! * **Responses.** At most 8 KiB per header line, 64 KiB of headers and 128 headers; CRLF only, no obs-fold, 1xx
//!   skipped. A body needs `Content-Length` or `Transfer-Encoding: chunked` (exactly), never both: a body read until
//!   the connection closes is refused, which also avoids the TLS end-of-file ambiguity.
//! * **Redirects.** 301, 302, 303, 307 and 308 with a `Location`, at most 5 hops. In production every hop must be
//!   https (no host allowlist: GitHub does not document its download host). The test policy stays on its loopback
//!   origin.
use gio::prelude::*;
use std::{
    io::{self, BufRead, BufReader, Read},
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    time::{Duration, Instant},
};

const MAX_LINE: usize = 8 << 10;
const MAX_HEAD: usize = 64 << 10;
const MAX_HEADERS: usize = 128;
const MAX_TRAILER: usize = 8 << 10;
const MAX_HOPS: usize = 5;
const WATCH_TICK: Duration = Duration::from_millis(200);
/// GitHub's REST API version that the feed parser was written against (feed.rs).
const API_VERSION: &str = "2022-11-28";

/// Where a request may go: GitHub over https only, or the test seam's loopback origin only (seam.rs).
#[derive(Clone, Debug)]
pub(crate) enum Policy {
    Production,
    /// exactly `http://127.0.0.1:<port>/`
    Test {
        origin: url::Url,
    },
}

/// The limits of one GET (the API list: 20 s stall, 60 s deadline, 2 MiB).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub max_body: u64,
    pub stall: Duration,
    pub deadline: Option<Duration>,
}

/// The final answer (after redirects): its status, the URL it came from and its headers. A 2xx body went to
/// the sink; any other body was not read.
#[derive(Clone, Debug)]
pub(crate) struct Response {
    pub status: u16,
    pub url: url::Url,
    pub headers: Vec<(String, String)>,
}
impl Response {
    /// A header's value by its case-insensitive name (the first one).
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }
}
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Where a body goes; `progress` after each piece (`total` is the Content-Length when there is one).
pub(crate) trait Sink {
    fn data(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn progress(&mut self, _done: u64, _total: Option<u64>) {}
}
impl Sink for Vec<u8> {
    fn data(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HttpError {
    /// a URL or redirect the policy forbids (plain http, another origin in a test)
    Policy,
    /// GIO has no TLS backend: glib-networking is missing
    NoTls,
    Dns,
    Connect,
    Tls(String),
    Proxy,
    Timeout,
    Cancelled,
    Protocol(&'static str),
    TooLarge,
    Status(u16),
    Io(String),
}
impl HttpError {
    /// The stored code (state.json `error.code`).
    pub(crate) fn code(&self) -> &'static str {
        match self {
            HttpError::Policy => "policy",
            HttpError::NoTls => "noTls",
            HttpError::Dns => "dns",
            HttpError::Connect => "connect",
            HttpError::Tls(_) => "tls",
            HttpError::Proxy => "proxy",
            HttpError::Timeout => "timeout",
            HttpError::Cancelled => "cancelled",
            HttpError::Protocol(_) => "protocol",
            HttpError::TooLarge => "tooLarge",
            HttpError::Status(_) => "status",
            HttpError::Io(_) => "io",
        }
    }
    /// The fixed text of each variant (host/i18n/update.tsv); the raw GLib message, which is in the system
    /// language, only goes into [`Self::detail`].
    pub(crate) fn key(&self) -> &'static str {
        match self {
            HttpError::NoTls => "update.winpoint.err.noTls",
            HttpError::Dns => "update.winpoint.err.dns",
            HttpError::Connect | HttpError::Io(_) => "update.winpoint.err.connect",
            HttpError::Tls(_) | HttpError::Policy => "update.winpoint.err.tls",
            HttpError::Proxy => "update.winpoint.err.proxy",
            HttpError::Timeout => "update.winpoint.err.timeout",
            HttpError::Cancelled => "update.winpoint.err.cancelled",
            HttpError::Protocol(_) | HttpError::TooLarge => "update.winpoint.err.feed",
            HttpError::Status(_) => "update.winpoint.err.status",
        }
    }
    pub(crate) fn detail(&self) -> String {
        match self {
            HttpError::Tls(message) | HttpError::Io(message) => message.clone(),
            HttpError::Protocol(what) => format!("protocol: {what}"),
            HttpError::Status(code) => format!("HTTP {code}"),
            other => other.code().to_owned(),
        }
    }
}

/// A GLib error of a socket, resolver or TLS call as the variant the card names.
pub(crate) fn from_glib(error: &glib::Error) -> HttpError {
    if let Some(kind) = error.kind::<gio::IOErrorEnum>() {
        return match kind {
            gio::IOErrorEnum::TimedOut => HttpError::Timeout,
            gio::IOErrorEnum::Cancelled => HttpError::Cancelled,
            gio::IOErrorEnum::ConnectionRefused
            | gio::IOErrorEnum::NetworkUnreachable
            | gio::IOErrorEnum::HostUnreachable
            | gio::IOErrorEnum::BrokenPipe
            | gio::IOErrorEnum::NotConnected => HttpError::Connect,
            gio::IOErrorEnum::ProxyFailed
            | gio::IOErrorEnum::ProxyAuthFailed
            | gio::IOErrorEnum::ProxyNeedAuth
            | gio::IOErrorEnum::ProxyNotAllowed => HttpError::Proxy,
            gio::IOErrorEnum::HostNotFound => HttpError::Dns,
            _ => HttpError::Io(error.message().to_owned()),
        };
    }
    if error.kind::<gio::ResolverError>().is_some() {
        return HttpError::Dns;
    }
    if error.kind::<gio::TlsError>().is_some() {
        return HttpError::Tls(error.message().to_owned());
    }
    HttpError::Io(error.message().to_owned())
}
/// An io::Error of the readers below: the GLib error inside it when there is one.
fn from_io(error: io::Error) -> HttpError {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<glib::Error>())
    {
        Some(glib) => from_glib(glib),
        None if error.kind() == io::ErrorKind::UnexpectedEof => HttpError::Protocol("truncated"),
        None => HttpError::Io(error.to_string()),
    }
}

// ------------------------------------------------------------------------------------------------ parsers

/// One line without its CRLF, at most `max` bytes. A bare LF, a longer line or an end of stream inside the line
/// is a protocol error; `Ok(None)` is an end of stream before the first byte.
fn read_line<R: BufRead>(r: &mut R, max: usize) -> Result<Option<Vec<u8>>, HttpError> {
    let mut line = Vec::new();
    loop {
        let available = r.fill_buf().map_err(from_io)?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(HttpError::Protocol("truncated"))
            };
        }
        let (take, done) = match available.iter().position(|&b| b == b'\n') {
            Some(at) => (at + 1, true),
            None => (available.len(), false),
        };
        line.extend_from_slice(&available[..take]);
        r.consume(take);
        if line.len() > max + 2 {
            return Err(HttpError::Protocol("line too long"));
        }
        if done {
            line.pop();
            if line.pop() != Some(b'\r') {
                return Err(HttpError::Protocol("bare LF"));
            }
            return Ok(Some(line));
        }
    }
}

fn status_of(line: &[u8]) -> Result<u16, HttpError> {
    let bad = HttpError::Protocol("status line");
    let rest = line.strip_prefix(b"HTTP/1.").ok_or(bad.clone())?;
    // "1 200 OK", or "1 204" without the reason phrase
    if rest.len() < 5 || !rest[0].is_ascii_digit() || rest[1] != b' ' {
        return Err(bad);
    }
    let code = &rest[2..5];
    if !code.iter().all(u8::is_ascii_digit) || (rest.len() > 5 && rest[5] != b' ') {
        return Err(bad);
    }
    let status = code.iter().fold(0u16, |n, d| n * 10 + u16::from(d - b'0'));
    if !(100..=599).contains(&status) {
        return Err(bad);
    }
    Ok(status)
}

fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// The status line and headers of the final answer (1xx answers are skipped), with the limits above.
pub(crate) fn read_head<R: BufRead>(r: &mut R) -> Result<(u16, Vec<(String, String)>), HttpError> {
    let mut total = 0usize;
    loop {
        let line = read_line(r, MAX_LINE)?.ok_or(HttpError::Protocol("no answer"))?;
        total += line.len() + 2;
        let status = status_of(&line)?;
        let mut headers = Vec::new();
        loop {
            let line = read_line(r, MAX_LINE)?.ok_or(HttpError::Protocol("truncated"))?;
            total += line.len() + 2;
            if total > MAX_HEAD {
                return Err(HttpError::Protocol("head too large"));
            }
            if line.is_empty() {
                break;
            }
            if line[0] == b' ' || line[0] == b'\t' {
                return Err(HttpError::Protocol("obs-fold"));
            }
            let colon = line
                .iter()
                .position(|&b| b == b':')
                .ok_or(HttpError::Protocol("header"))?;
            let (name, value) = (&line[..colon], &line[colon + 1..]);
            if name.is_empty() || !name.iter().all(|&b| is_tchar(b)) {
                return Err(HttpError::Protocol("header name"));
            }
            if headers.len() == MAX_HEADERS {
                return Err(HttpError::Protocol("too many headers"));
            }
            let value = String::from_utf8_lossy(value);
            headers.push((
                String::from_utf8_lossy(name).into_owned(),
                value.trim_matches([' ', '\t']).to_owned(),
            ));
        }
        if !(100..200).contains(&status) {
            return Ok((status, headers));
        }
    }
}

/// How the body of an answer is delimited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Framing {
    Empty,
    Length(u64),
    Chunked,
}

/// The body framing (RFC 9112 §6.3, the strict subset): no body for 1xx, 204 and 304; exactly `chunked`, never
/// together with a Content-Length; one Content-Length value (repeats must agree); neither is refused.
pub(crate) fn framing(status: u16, headers: &[(String, String)]) -> Result<Framing, HttpError> {
    if (100..200).contains(&status) || status == 204 || status == 304 {
        return Ok(Framing::Empty);
    }
    let named = |name: &str| -> Vec<&str> {
        headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    };
    let (coding, length) = (named("transfer-encoding"), named("content-length"));
    if !coding.is_empty() {
        if !length.is_empty() {
            return Err(HttpError::Protocol("length and chunked"));
        }
        if coding.len() != 1 || !coding[0].eq_ignore_ascii_case("chunked") {
            return Err(HttpError::Protocol("transfer coding"));
        }
        return Ok(Framing::Chunked);
    }
    let mut found: Option<u64> = None;
    for value in length.iter().flat_map(|v| v.split(',')) {
        let value = value.trim_matches([' ', '\t']);
        if value.is_empty() || value.len() > 19 || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(HttpError::Protocol("content length"));
        }
        let n: u64 = value
            .parse()
            .map_err(|_| HttpError::Protocol("content length"))?;
        if found.is_some_and(|f| f != n) {
            return Err(HttpError::Protocol("two lengths"));
        }
        found = Some(n);
    }
    found
        .map(Framing::Length)
        .ok_or(HttpError::Protocol("no length"))
}

/// Exactly `n` bytes into the sink (a shorter stream is truncated).
fn copy_exact<R: BufRead>(
    r: &mut R,
    mut n: u64,
    done: &mut u64,
    total: Option<u64>,
    sink: &mut dyn Sink,
) -> Result<(), HttpError> {
    while n > 0 {
        let available = r.fill_buf().map_err(from_io)?;
        if available.is_empty() {
            return Err(HttpError::Protocol("truncated"));
        }
        let take = available
            .len()
            .min(usize::try_from(n).unwrap_or(usize::MAX));
        sink.data(&available[..take])
            .map_err(|e| HttpError::Io(e.to_string()))?;
        r.consume(take);
        n -= take as u64;
        *done += take as u64;
        sink.progress(*done, total);
    }
    Ok(())
}

/// The body into the sink, at most `max` bytes (across all chunks); returns its length.
pub(crate) fn copy_body<R: BufRead>(
    r: &mut R,
    framing: Framing,
    max: u64,
    sink: &mut dyn Sink,
) -> Result<u64, HttpError> {
    let mut done = 0u64;
    match framing {
        Framing::Empty => {}
        Framing::Length(n) => {
            if n > max {
                return Err(HttpError::TooLarge);
            }
            copy_exact(r, n, &mut done, Some(n), sink)?;
        }
        Framing::Chunked => loop {
            let line = read_line(r, MAX_LINE)?.ok_or(HttpError::Protocol("truncated"))?;
            let size = line.split(|&b| b == b';').next().unwrap_or_default();
            let size = size.trim_ascii();
            if size.is_empty() || size.len() > 16 || !size.iter().all(u8::is_ascii_hexdigit) {
                return Err(HttpError::Protocol("chunk size"));
            }
            let size = u64::from_str_radix(std::str::from_utf8(size).unwrap_or("x"), 16)
                .map_err(|_| HttpError::Protocol("chunk size"))?;
            if size == 0 {
                let mut trailers = 0usize;
                loop {
                    let line =
                        read_line(r, MAX_TRAILER)?.ok_or(HttpError::Protocol("truncated"))?;
                    if line.is_empty() {
                        break;
                    }
                    trailers += line.len() + 2;
                    if trailers > MAX_TRAILER {
                        return Err(HttpError::Protocol("trailers too large"));
                    }
                }
                break;
            }
            if done.checked_add(size).is_none_or(|end| end > max) {
                return Err(HttpError::TooLarge);
            }
            copy_exact(r, size, &mut done, None, sink)?;
            if read_line(r, 0)? != Some(Vec::new()) {
                return Err(HttpError::Protocol("chunk end"));
            }
        },
    }
    Ok(done)
}

/// Whether `url` may be fetched under `policy`.
fn allowed(url: &url::Url, policy: &Policy) -> bool {
    let plain = url.username().is_empty() && url.password().is_none() && url.host_str().is_some();
    match policy {
        Policy::Production => plain && url.scheme() == "https",
        Policy::Test { origin } => {
            plain && url.scheme() == "http" && url.origin() == origin.origin()
        }
    }
}

/// The next hop of a redirect: `location` resolved against `base` (relative or absolute), without its fragment,
/// and allowed by the policy.
pub(crate) fn redirect_target(
    base: &url::Url,
    location: &str,
    policy: &Policy,
) -> Result<url::Url, HttpError> {
    let mut target = base
        .join(location)
        .map_err(|_| HttpError::Protocol("location"))?;
    target.set_fragment(None);
    if !allowed(&target, policy) {
        return Err(HttpError::Policy);
    }
    Ok(target)
}

// ------------------------------------------------------------------------------------------------ the client

/// Why the watchdog cancelled the call.
const BY_STOP: u8 = 1;
const BY_DEADLINE: u8 = 2;
const BY_STALL: u8 = 3;

/// What the watchdog of one call watches: the time of the last byte (ms since the call started), whether the
/// call is over, and why it cancelled.
struct Watch {
    started: Instant,
    last: AtomicU64,
    done: AtomicBool,
    reason: AtomicU8,
}
impl Watch {
    fn touch(&self) {
        self.last
            .store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
    /// A cancelled GLib call as the reason the watchdog recorded.
    fn map(&self, error: HttpError) -> HttpError {
        match (self.reason.load(Ordering::SeqCst), error) {
            (BY_STOP, _) => HttpError::Cancelled,
            (BY_DEADLINE | BY_STALL, _) => HttpError::Timeout,
            (_, error) => error,
        }
    }
}

/// `Read` over a GIO stream that passes the call's cancellable to every read (gio-rs `into_read()` passes none).
struct CancelRead<'a> {
    stream: gio::InputStream,
    cancel: &'a gio::Cancellable,
    watch: &'a Watch,
}
impl Read for CancelRead<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self
            .stream
            .read(&mut *buf, Some(self.cancel))
            .map_err(io::Error::other)?;
        self.watch.touch();
        Ok(n)
    }
}

/// The client: its policy and User-Agent (GitHub requires one).
pub(crate) struct Client {
    policy: Policy,
    user_agent: String,
}
impl Client {
    pub(crate) fn production() -> Self {
        Self::with(Policy::Production)
    }
    pub(crate) fn for_test(origin: url::Url) -> Self {
        Self::with(Policy::Test { origin })
    }
    fn with(policy: Policy) -> Self {
        Self {
            policy,
            user_agent: format!(
                "WinPoint/{} (+https://github.com/roboburr/WinPoint)",
                crate::VERSION
            ),
        }
    }
    #[cfg(test)]
    pub(crate) fn policy(&self) -> &Policy {
        &self.policy
    }

    /// GET `url`, following up to 5 redirects; a 2xx body goes to `sink`. `stop` is the host's shutdown (later
    /// also a job's Cancel): the watchdog ends a blocked call within 200 ms of it.
    pub(crate) fn get(
        &self,
        url: &url::Url,
        accept: &str,
        limits: &Limits,
        sink: &mut dyn Sink,
        stop: &AtomicBool,
    ) -> Result<Response, HttpError> {
        if !allowed(url, &self.policy) {
            return Err(HttpError::Policy);
        }
        if accept.bytes().any(|b| b.is_ascii_control()) {
            return Err(HttpError::Protocol("accept"));
        }
        let cancel = gio::Cancellable::new();
        let watch = Watch {
            started: Instant::now(),
            last: AtomicU64::new(0),
            done: AtomicBool::new(false),
            reason: AtomicU8::new(0),
        };
        std::thread::scope(|scope| {
            // no call without its watchdog: without it nothing would end a server that never answers
            let watchdog = std::thread::Builder::new()
                .name("winpoint-watch".into())
                .spawn_scoped(scope, || watchdog(&watch, &cancel, stop, limits))
                .map_err(|e| HttpError::Io(e.to_string()))?;
            let result = self.run(url, accept, limits, sink, &cancel, &watch);
            watch.done.store(true, Ordering::SeqCst);
            watchdog.thread().unpark();
            result.map_err(|error| watch.map(error))
        })
    }

    fn run(
        &self,
        url: &url::Url,
        accept: &str,
        limits: &Limits,
        sink: &mut dyn Sink,
        cancel: &gio::Cancellable,
        watch: &Watch,
    ) -> Result<Response, HttpError> {
        let mut url = url.clone();
        for hop in 0..=MAX_HOPS {
            let connection = self.connect(&url, limits, cancel)?;
            watch.touch();
            self.send(&connection, &url, accept, cancel)?;
            let mut reader = BufReader::with_capacity(
                16 << 10,
                CancelRead {
                    stream: connection.input_stream(),
                    cancel,
                    watch,
                },
            );
            let (status, headers) = read_head(&mut reader)?;
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(location) = header(&headers, "location")
            {
                if hop == MAX_HOPS {
                    return Err(HttpError::Protocol("too many redirects"));
                }
                url = redirect_target(&url, location, &self.policy)?;
                continue;
            }
            if (200..300).contains(&status) {
                let framing = framing(status, &headers)?;
                copy_body(&mut reader, framing, limits.max_body, sink)?;
            }
            drop(reader);
            let _ = connection.close(gio::Cancellable::NONE);
            return Ok(Response {
                status,
                url,
                headers,
            });
        }
        Err(HttpError::Protocol("too many redirects"))
    }

    fn connect(
        &self,
        url: &url::Url,
        limits: &Limits,
        cancel: &gio::Cancellable,
    ) -> Result<gio::SocketConnection, HttpError> {
        let tls = url.scheme() == "https";
        if tls && !gio::TlsBackend::default().supports_tls() {
            return Err(HttpError::NoTls);
        }
        let client = gio::SocketClient::new();
        client.set_tls(tls);
        client.set_timeout(limits.stall.as_secs().clamp(1, 600) as u32);
        if matches!(self.policy, Policy::Test { .. }) {
            // the seam's loopback server, never through a proxy
            client.set_enable_proxy(false);
        }
        let port = url.port_or_known_default().unwrap_or(443);
        client
            .connect_to_uri(url.as_str(), port, Some(cancel))
            .map_err(|e| from_glib(&e))
    }

    fn send(
        &self,
        connection: &gio::SocketConnection,
        url: &url::Url,
        accept: &str,
        cancel: &gio::Cancellable,
    ) -> Result<(), HttpError> {
        let target = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        let host = match (url.host_str(), url.port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_owned(),
            (None, _) => return Err(HttpError::Policy),
        };
        let mut request = format!(
            "GET {target} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {}\r\nAccept: {accept}\r\n\
             Accept-Encoding: identity\r\nConnection: close\r\n",
            self.user_agent
        );
        if accept.starts_with("application/vnd.github") {
            request.push_str(&format!("X-GitHub-Api-Version: {API_VERSION}\r\n"));
        }
        request.push_str("\r\n");
        let output = connection.output_stream();
        output
            .write_all(request.as_bytes(), Some(cancel))
            .map_err(|e| from_glib(&e))?;
        output.flush(Some(cancel)).map_err(|e| from_glib(&e))
    }
}

/// The watchdog of one call: wakes every 200 ms (or when the call ends) and cancels for `stop`, the deadline or a
/// stall, recording why.
fn watchdog(watch: &Watch, cancel: &gio::Cancellable, stop: &AtomicBool, limits: &Limits) {
    while !watch.done.load(Ordering::SeqCst) {
        let elapsed = watch.started.elapsed();
        let idle =
            elapsed.saturating_sub(Duration::from_millis(watch.last.load(Ordering::Relaxed)));
        let reason = if stop.load(Ordering::SeqCst) {
            BY_STOP
        } else if limits.deadline.is_some_and(|d| elapsed >= d) {
            BY_DEADLINE
        } else if idle >= limits.stall {
            BY_STALL
        } else {
            0
        };
        if reason != 0 {
            watch.reason.store(reason, Ordering::SeqCst);
            cancel.cancel();
            return;
        }
        std::thread::park_timeout(WATCH_TICK);
    }
}
