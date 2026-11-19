//! libcups through dlopen (notes/claude/settings-parity.md §6.6 `dlib!`, the plan's dlopen list: libcups.so.2): one
//! HTTP connection per call to the scheduler that libcups itself chooses (`cupsServer()`: `CUPS_SERVER`,
//! client.conf, else the local socket; the tests name a server of their own), IPP requests built with `ippNew*` /
//! `ippAdd*` and their answers read into JSON (every attribute as an array of its values, see [`Attrs`]).
//!
//! libcups keeps its user, server and password callback per thread; every connection here sets a password callback
//! that answers what the call gave (the print-server authentication of `printers.discover`) or nothing, so libcups
//! never asks on a terminal.
use libc::{c_char, c_int, c_void};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::sync::OnceLock;

crate::dlib::dlib! {
    /// libcups 2 (CUPS 2.4): the scheduler's IPP API.
    pub(crate) struct Cups ["libcups.so.2"] {
        fn cupsServer() -> *const c_char;
        fn ippPort() -> c_int;
        fn cupsEncryption() -> c_int;
        fn cupsUser() -> *const c_char;
        fn cupsSetUser(*const c_char) -> ();
        fn cupsSetPasswordCB2(Option<PasswordCb>, *mut c_void) -> ();
        fn httpConnect2(*const c_char, c_int, *mut c_void, c_int, c_int, c_int, c_int, *mut c_int) -> *mut c_void;
        fn httpClose(*mut c_void) -> ();
        fn ippNewRequest(c_int) -> *mut c_void;
        fn ippDelete(*mut c_void) -> ();
        fn ippAddString(*mut c_void, c_int, c_int, *const c_char, *const c_char, *const c_char) -> *mut c_void;
        fn ippAddStrings(*mut c_void, c_int, c_int, *const c_char, c_int, *const c_char, *const *const c_char) -> *mut c_void;
        fn ippAddInteger(*mut c_void, c_int, c_int, *const c_char, c_int) -> *mut c_void;
        fn ippAddBoolean(*mut c_void, c_int, *const c_char, c_char) -> *mut c_void;
        fn cupsDoRequest(*mut c_void, *mut c_void, *const c_char) -> *mut c_void;
        fn cupsDoIORequest(*mut c_void, *mut c_void, *const c_char, c_int, c_int) -> *mut c_void;
        fn cupsLastError() -> c_int;
        fn cupsLastErrorString() -> *const c_char;
        fn ippGetStatusCode(*mut c_void) -> c_int;
        fn ippFirstAttribute(*mut c_void) -> *mut c_void;
        fn ippNextAttribute(*mut c_void) -> *mut c_void;
        fn ippGetName(*mut c_void) -> *const c_char;
        fn ippGetGroupTag(*mut c_void) -> c_int;
        fn ippGetValueTag(*mut c_void) -> c_int;
        fn ippGetCount(*mut c_void) -> c_int;
        fn ippGetString(*mut c_void, c_int, *mut *const c_char) -> *const c_char;
        fn ippGetInteger(*mut c_void, c_int) -> c_int;
        fn ippGetBoolean(*mut c_void, c_int) -> c_int;
        fn ippGetRange(*mut c_void, c_int, *mut c_int) -> c_int;
        fn ippGetResolution(*mut c_void, c_int, *mut c_int, *mut c_int) -> c_int;
        fn cupsGetPPD3(*mut c_void, *const c_char, *mut libc::time_t, *mut c_char, libc::size_t) -> c_int;
    }
}

type PasswordCb = unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    *const c_char,
    *const c_char,
    *mut c_void,
) -> *const c_char;

/// IPP operations (RFC 8011, CUPS extensions).
pub(crate) const PRINT_JOB: c_int = 0x0002;
pub(crate) const GET_JOBS: c_int = 0x000A;
pub(crate) const GET_PRINTER_ATTRIBUTES: c_int = 0x000B;
pub(crate) const SET_JOB_ATTRIBUTES: c_int = 0x0014;
pub(crate) const CREATE_PRINTER_SUBSCRIPTIONS: c_int = 0x0016;
pub(crate) const RENEW_SUBSCRIPTION: c_int = 0x001A;
pub(crate) const CANCEL_SUBSCRIPTION: c_int = 0x001B;
pub(crate) const CUPS_GET_DEFAULT: c_int = 0x4001;
pub(crate) const CUPS_GET_PRINTERS: c_int = 0x4002;
pub(crate) const CUPS_GET_PPDS: c_int = 0x400C;
pub(crate) const CUPS_AUTHENTICATE_JOB: c_int = 0x400E;

/// Group and value tags.
pub(crate) const OPERATION: c_int = 0x01;
pub(crate) const JOB: c_int = 0x02;
pub(crate) const PRINTER: c_int = 0x04;
pub(crate) const SUBSCRIPTION: c_int = 0x06;
const ZERO: c_int = 0x00;
pub(crate) const INTEGER: c_int = 0x21;
const BOOLEAN: c_int = 0x22;
pub(crate) const ENUM: c_int = 0x23;
const STRING: c_int = 0x30;
const RESOLUTION: c_int = 0x32;
const RANGE: c_int = 0x33;
const TEXTLANG: c_int = 0x35;
const NAMELANG: c_int = 0x36;
pub(crate) const TEXT: c_int = 0x41;
pub(crate) const NAME: c_int = 0x42;
pub(crate) const KEYWORD: c_int = 0x44;
pub(crate) const URI: c_int = 0x45;
pub(crate) const MIMETYPE: c_int = 0x49;
const MEMBERNAME: c_int = 0x4A;

/// The status codes the services tell apart (ipp_status_t).
pub(crate) const STATUS_NOT_AUTHENTICATED: c_int = 0x0401;
pub(crate) const STATUS_FORBIDDEN: c_int = 0x0402;
pub(crate) const STATUS_NOT_FOUND: c_int = 0x0406;
pub(crate) const STATUS_NOT_POSSIBLE: c_int = 0x0404;
pub(crate) const STATUS_SERVICE_UNAVAILABLE: c_int = 0x0502;
/// `IPP_STATUS_ERROR_CUPS_*` and libcups's own errors start at 0x1000 (the connection could not be made, ...).
pub(crate) const STATUS_INTERNAL: c_int = 0x0500;

static LIB: OnceLock<Result<Cups, crate::dlib::DlError>> = OnceLock::new();

/// libcups, loaded once for the process; None when it is not installed (the page then says printing is unavailable).
pub(crate) fn lib() -> Option<&'static Cups> {
    LIB.get_or_init(Cups::load).as_ref().ok()
}

/// A failed request: the IPP status (or libcups's own error) and its text.
#[derive(Debug, Clone)]
pub(crate) struct IppError {
    pub status: c_int,
    pub message: String,
}

impl IppError {
    /// Whether the scheduler could not be reached at all (no cupsd, refused, not running).
    pub(crate) fn unreachable(&self) -> bool {
        self.status == STATUS_SERVICE_UNAVAILABLE
            || self
                .message
                .to_ascii_lowercase()
                .contains("connection refused")
            || self.message.to_ascii_lowercase().contains("no such file")
            || self.message.to_ascii_lowercase().contains("host is down")
            || self
                .message
                .to_ascii_lowercase()
                .contains("unable to connect")
    }
}

/// Every attribute of one group of an answer, each as the array of its values (numbers, booleans, strings,
/// `[lower, upper]` for a range, `"600x600dpi"` for a resolution, null for a value Settings does not read).
pub(crate) type Attrs = Map<String, Value>;

/// The first value of an attribute.
pub(crate) fn one<'a>(attrs: &'a Attrs, name: &str) -> &'a Value {
    attrs
        .get(name)
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .unwrap_or(&Value::Null)
}

/// The first value as text ("" when absent).
pub(crate) fn text(attrs: &Attrs, name: &str) -> String {
    one(attrs, name).as_str().unwrap_or("").to_owned()
}

/// Every value of an attribute as text.
pub(crate) fn texts(attrs: &Attrs, name: &str) -> Vec<String> {
    attrs
        .get(name)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every value of an attribute as a number.
pub(crate) fn numbers(attrs: &Attrs, name: &str) -> Vec<i64> {
    attrs
        .get(name)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

unsafe fn owned(p: *const c_char) -> Option<String> {
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

thread_local! {
    /// The password the callback gives on this thread (print-server authentication), once per connection: libcups
    /// asks again after a refusal, and the same password would only be refused again.
    static PASSWORD: RefCell<Option<CString>> = const { RefCell::new(None) };
    /// The password given last, kept alive while libcups uses it.
    static GIVEN: RefCell<Option<CString>> = const { RefCell::new(None) };
}

unsafe extern "C" fn password_cb(
    _prompt: *const c_char,
    _http: *mut c_void,
    _method: *const c_char,
    _resource: *const c_char,
    _data: *mut c_void,
) -> *const c_char {
    match PASSWORD.with(|p| p.borrow_mut().take()) {
        Some(password) => GIVEN.with(|g| g.borrow_mut().insert(password).as_ptr()),
        None => std::ptr::null(),
    }
}

/// Whether a scheduler is this computer's, as g-c-c's printer_set_default decides it: "localhost", "127.0.0.1" or
/// "::1" at the start (any case), or a socket path.
pub(crate) fn is_local(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    ["localhost", "127.0.0.1", "::1"]
        .iter()
        .any(|p| lower.starts_with(p))
        || host.starts_with('/')
}

/// A scheduler (or printer) to talk to: libcups's own choice, or a `host:port` / socket path.
#[derive(Clone, Debug, Default)]
pub(crate) struct Server {
    /// None: `cupsServer()` / `ippPort()` (CUPS_SERVER, client.conf, the local socket).
    pub address: Option<(String, u16)>,
}

impl Server {
    /// The scheduler's host as libcups names it (`cupsServer()`: "localhost", "/run/cups/cups.sock",
    /// "print.example:631"), or the tests' address; None without libcups.
    pub(crate) fn host(&self) -> Option<String> {
        match &self.address {
            Some((host, _)) => Some(host.clone()),
            None => {
                let lib = lib()?;
                unsafe { owned((lib.cupsServer)()) }
            }
        }
    }

    /// `host:port`, `[v6]:port`, `host` (port 631) or a socket path (the tests' way to name one).
    #[cfg(test)]
    pub(crate) fn parse(text: &str) -> Option<(String, u16)> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        if text.starts_with('/') {
            return Some((text.to_owned(), 0));
        }
        if let Some(rest) = text.strip_prefix('[') {
            let (host, tail) = rest.split_once(']')?;
            let port = tail
                .strip_prefix(':')
                .map_or(Some(631), |p| p.parse().ok())?;
            return Some((host.to_owned(), port));
        }
        match text.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => {
                Some((host.to_owned(), port.parse().ok()?))
            }
            Some(_) => Some((text.to_owned(), 631)),
            None => Some((text.to_owned(), 631)),
        }
    }
}

/// One connection, closed when dropped. Not Send: it lives on the thread of one call.
pub(crate) struct Conn {
    lib: &'static Cups,
    http: *mut c_void,
    /// The call named a user: libcups's user of this thread goes back to the default afterwards.
    user_set: bool,
}

impl Drop for Conn {
    fn drop(&mut self) {
        unsafe { (self.lib.httpClose)(self.http) };
        PASSWORD.with(|p| *p.borrow_mut() = None);
        GIVEN.with(|g| *g.borrow_mut() = None);
        if self.user_set {
            unsafe { (self.lib.cupsSetUser)(std::ptr::null()) };
        }
    }
}

/// A request being built (freed by libcups when sent, or here when dropped unsent).
pub(crate) struct Request {
    lib: &'static Cups,
    ipp: *mut c_void,
}

impl Drop for Request {
    fn drop(&mut self) {
        if !self.ipp.is_null() {
            unsafe { (self.lib.ippDelete)(self.ipp) };
        }
    }
}

impl Request {
    pub(crate) fn string(
        &mut self,
        group: c_int,
        tag: c_int,
        name: &str,
        value: &str,
    ) -> &mut Self {
        let (n, v) = (cstr(name), cstr(value));
        unsafe {
            (self.lib.ippAddString)(
                self.ipp,
                group,
                tag,
                n.as_ptr(),
                std::ptr::null(),
                v.as_ptr(),
            )
        };
        self
    }

    pub(crate) fn strings(
        &mut self,
        group: c_int,
        tag: c_int,
        name: &str,
        values: &[&str],
    ) -> &mut Self {
        if values.is_empty() {
            return self;
        }
        let n = cstr(name);
        let owned: Vec<CString> = values.iter().map(|v| cstr(v)).collect();
        let ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        unsafe {
            (self.lib.ippAddStrings)(
                self.ipp,
                group,
                tag,
                n.as_ptr(),
                ptrs.len() as c_int,
                std::ptr::null(),
                ptrs.as_ptr(),
            )
        };
        self
    }

    pub(crate) fn integer(
        &mut self,
        group: c_int,
        tag: c_int,
        name: &str,
        value: i32,
    ) -> &mut Self {
        let n = cstr(name);
        unsafe { (self.lib.ippAddInteger)(self.ipp, group, tag, n.as_ptr(), value) };
        self
    }

    pub(crate) fn boolean(&mut self, group: c_int, name: &str, value: bool) -> &mut Self {
        let n = cstr(name);
        unsafe { (self.lib.ippAddBoolean)(self.ipp, group, n.as_ptr(), value as c_char) };
        self
    }

    /// The operation attributes every request carries after charset and language: the user.
    fn user(&mut self) -> &mut Self {
        let user = unsafe { owned((self.lib.cupsUser)()) }.unwrap_or_default();
        self.string(OPERATION, NAME, "requesting-user-name", &user)
    }
}

impl Conn {
    /// Connect to `server` (or libcups's own choice), with `credentials` (user, password) for a print server that
    /// asks; None and the reason when there is no scheduler.
    pub(crate) fn open(
        server: &Server,
        credentials: Option<(&str, &str)>,
        timeout_ms: i32,
    ) -> Result<Conn, IppError> {
        let Some(lib) = lib() else {
            return Err(IppError {
                status: -1,
                message: "libcups.so.2".into(),
            });
        };
        unsafe { (lib.cupsSetPasswordCB2)(Some(password_cb), std::ptr::null_mut()) };
        match credentials {
            Some((user, password)) => {
                let u = cstr(user);
                unsafe { (lib.cupsSetUser)(u.as_ptr()) };
                PASSWORD.with(|p| *p.borrow_mut() = Some(cstr(password)));
            }
            None => PASSWORD.with(|p| *p.borrow_mut() = None),
        }
        let (host, port, encryption) = match &server.address {
            Some((host, port)) => (host.clone(), *port as c_int, 0),
            None => {
                let host =
                    unsafe { owned((lib.cupsServer)()) }.unwrap_or_else(|| "localhost".into());
                (host, unsafe { (lib.ippPort)() }, unsafe {
                    (lib.cupsEncryption)()
                })
            }
        };
        let h = cstr(&host);
        let http = unsafe {
            (lib.httpConnect2)(
                h.as_ptr(),
                port,
                std::ptr::null_mut(),
                libc::AF_UNSPEC,
                encryption,
                1,
                timeout_ms,
                std::ptr::null_mut(),
            )
        };
        if http.is_null() {
            // httpConnect2 sets no IPP status: the reason is errno's
            return Err(IppError {
                status: STATUS_SERVICE_UNAVAILABLE,
                message: format!(
                    "Unable to connect to {host}: {}",
                    std::io::Error::last_os_error()
                ),
            });
        }
        Ok(Conn {
            lib,
            http,
            user_set: credentials.is_some(),
        })
    }

    /// A new request for `op` with the user as requesting-user-name.
    pub(crate) fn request(&self, op: c_int) -> Request {
        let mut r = Request {
            lib: self.lib,
            ipp: unsafe { (self.lib.ippNewRequest)(op) },
        };
        r.user();
        r
    }

    /// Send a request to `resource` ("/", "/admin/", "/printers/NAME", "/jobs/") and read the answer.
    pub(crate) fn send(&self, mut req: Request, resource: &str) -> Result<Response, IppError> {
        let res = cstr(resource);
        let ipp = std::mem::replace(&mut req.ipp, std::ptr::null_mut());
        let answer = unsafe { (self.lib.cupsDoRequest)(self.http, ipp, res.as_ptr()) };
        Response::check(self.lib, answer)
    }

    /// Send a request with a document read from `fd` (Print-Job).
    pub(crate) fn send_document(
        &self,
        mut req: Request,
        resource: &str,
        fd: c_int,
    ) -> Result<Response, IppError> {
        let res = cstr(resource);
        let ipp = std::mem::replace(&mut req.ipp, std::ptr::null_mut());
        let answer = unsafe { (self.lib.cupsDoIORequest)(self.http, ipp, res.as_ptr(), fd, -1) };
        Response::check(self.lib, answer)
    }

    /// A queue's PPD as text (cupsGetPPD3 writes it to a temporary file, which is removed here); None for a queue
    /// without one (a raw or a remote queue).
    pub(crate) fn ppd(&self, name: &str) -> Option<String> {
        let n = cstr(name);
        let mut buffer = vec![0 as c_char; 1024];
        let mut modtime: libc::time_t = 0;
        let status = unsafe {
            (self.lib.cupsGetPPD3)(
                self.http,
                n.as_ptr(),
                &mut modtime,
                buffer.as_mut_ptr(),
                buffer.len(),
            )
        };
        let path = unsafe { owned(buffer.as_ptr()) }.unwrap_or_default();
        // HTTP_STATUS_OK = 200
        let text = (status == 200 && !path.is_empty())
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .map(|bytes| decode_ppd_bytes(&bytes));
        if !path.is_empty() && status == 200 {
            let _ = std::fs::remove_file(&path);
        }
        text
    }
}

/// A PPD's bytes as text: UTF-8 when it is, else ISO 8859-1 (PPD's `*LanguageEncoding: ISOLatin1`).
pub(crate) fn decode_ppd_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes.iter().map(|b| *b as char).collect(),
    }
}

fn last_error(lib: &Cups) -> Option<IppError> {
    let status = unsafe { (lib.cupsLastError)() };
    let message = unsafe { owned((lib.cupsLastErrorString)()) }.unwrap_or_default();
    (status != 0).then_some(IppError { status, message })
}

/// An answer, freed when dropped.
pub(crate) struct Response {
    lib: &'static Cups,
    ipp: *mut c_void,
}

impl Drop for Response {
    fn drop(&mut self) {
        unsafe { (self.lib.ippDelete)(self.ipp) };
    }
}

impl Response {
    fn check(lib: &'static Cups, ipp: *mut c_void) -> Result<Response, IppError> {
        if ipp.is_null() {
            return Err(last_error(lib).unwrap_or(IppError {
                status: STATUS_INTERNAL,
                message: String::new(),
            }));
        }
        let response = Response { lib, ipp };
        let status = unsafe { (lib.ippGetStatusCode)(ipp) };
        if status >= 0x0400 {
            let message = last_error(lib).map(|e| e.message).unwrap_or_default();
            return Err(IppError { status, message });
        }
        Ok(response)
    }

    /// The groups of one kind (PRINTER, JOB, ...) in order: a new group starts at each separator.
    pub(crate) fn groups(&self, group: c_int) -> Vec<Attrs> {
        let lib = self.lib;
        let mut out = Vec::new();
        let mut current: Option<(c_int, Attrs)> = None;
        let mut attr = unsafe { (lib.ippFirstAttribute)(self.ipp) };
        while !attr.is_null() {
            let name = unsafe { owned((lib.ippGetName)(attr)) };
            let tag = unsafe { (lib.ippGetGroupTag)(attr) };
            let new_group =
                name.is_none() || tag == ZERO || current.as_ref().is_some_and(|(t, _)| *t != tag);
            if new_group
                && let Some((t, attrs)) = current.take()
                && t == group
            {
                out.push(attrs);
            }
            if let Some(name) = name {
                let (_, attrs) = current.get_or_insert_with(|| (tag, Attrs::new()));
                attrs.insert(name, values(lib, attr));
            }
            attr = unsafe { (lib.ippNextAttribute)(self.ipp) };
        }
        if let Some((t, attrs)) = current
            && t == group
        {
            out.push(attrs);
        }
        out
    }

    /// The first group of one kind (an empty map when the answer has none).
    pub(crate) fn group(&self, group: c_int) -> Attrs {
        self.groups(group).into_iter().next().unwrap_or_default()
    }
}

fn values(lib: &Cups, attr: *mut c_void) -> Value {
    let tag = unsafe { (lib.ippGetValueTag)(attr) };
    let count = unsafe { (lib.ippGetCount)(attr) };
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for i in 0..count {
        out.push(match tag {
            INTEGER | ENUM => json!(unsafe { (lib.ippGetInteger)(attr, i) }),
            BOOLEAN => json!(unsafe { (lib.ippGetBoolean)(attr, i) } != 0),
            RANGE => {
                let mut upper: c_int = 0;
                let lower = unsafe { (lib.ippGetRange)(attr, i, &mut upper) };
                json!([lower, upper])
            }
            RESOLUTION => {
                let (mut y, mut units): (c_int, c_int) = (0, 0);
                let x = unsafe { (lib.ippGetResolution)(attr, i, &mut y, &mut units) };
                // IPP_RES_PER_INCH = 3, IPP_RES_PER_CM = 4
                json!(format!(
                    "{x}x{y}{}",
                    if units == 4 { "dpcm" } else { "dpi" }
                ))
            }
            STRING | TEXTLANG | NAMELANG | TEXT..=MEMBERNAME => {
                let s = unsafe { owned((lib.ippGetString)(attr, i, std::ptr::null_mut())) };
                s.map(Value::String).unwrap_or(Value::Null)
            }
            _ => Value::Null,
        });
    }
    Value::Array(out)
}

/// The printer URI the scheduler takes for a queue name.
pub(crate) fn printer_uri(name: &str) -> String {
    format!("ipp://localhost/printers/{}", uri_encode(name))
}

/// A queue name in a URI path (CUPS allows everything but space, tab, '/', '#', '\\', quotes and '?').
pub(crate) fn uri_encode(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
