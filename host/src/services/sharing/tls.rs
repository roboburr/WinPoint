//! The RDP TLS certificate (amendment 34). gnome-control-center makes it in-process with GnuTLS
//! (cc-tls-certificate.c, libbonsai's `bonsai_tls_certificate_new_generate`: an RSA key and a self-signed X.509 v3
//! certificate with the distinguished name `C=US,CN=GNOME`, PEM files `rdp-tls.crt` and `rdp-tls.key`); grdctl has no
//! command that makes one, so Desktop Sharing cannot start on a fresh account without it. `libgnutls.so.30` is opened
//! with dlopen (`crate::dlib`); without it the feature reports `sharing.noTls`.
//!
//! The fingerprint is the SHA-256 of the certificate's DER bytes, as the "Verify Fingerprint" dialog and connecting
//! RDP clients show it: 32 upper-case hex pairs joined with `:`.
use libc::{c_char, c_int, c_uint, c_void, size_t, time_t};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// gnutls_datum_t.
#[repr(C)]
pub(crate) struct Datum {
    data: *mut u8,
    size: c_uint,
}

crate::dlib::dlib! {
    /// GnuTLS 3 (the symbols gnome-control-center 50 imports for its certificate).
    pub(crate) struct GnuTls ["libgnutls.so.30"] {
        fn gnutls_x509_privkey_init(*mut *mut c_void) -> c_int;
        fn gnutls_x509_privkey_generate(*mut c_void, c_int, c_uint, c_uint) -> c_int;
        fn gnutls_x509_privkey_export2(*mut c_void, c_int, *mut Datum) -> c_int;
        fn gnutls_x509_privkey_deinit(*mut c_void) -> ();
        fn gnutls_x509_crt_init(*mut *mut c_void) -> c_int;
        fn gnutls_x509_crt_set_version(*mut c_void, c_uint) -> c_int;
        fn gnutls_x509_crt_set_serial(*mut c_void, *const c_void, size_t) -> c_int;
        fn gnutls_x509_crt_set_dn(*mut c_void, *const c_char, *mut *const c_char) -> c_int;
        fn gnutls_x509_crt_set_key(*mut c_void, *mut c_void) -> c_int;
        fn gnutls_x509_crt_set_activation_time(*mut c_void, time_t) -> c_int;
        fn gnutls_x509_crt_set_expiration_time(*mut c_void, time_t) -> c_int;
        fn gnutls_x509_crt_sign(*mut c_void, *mut c_void, *mut c_void) -> c_int;
        fn gnutls_x509_crt_export2(*mut c_void, c_int, *mut Datum) -> c_int;
        fn gnutls_x509_crt_deinit(*mut c_void) -> ();
    }
}

const GNUTLS_PK_RSA: c_int = 1;
const GNUTLS_X509_FMT_PEM: c_int = 1;
/// The key size (libbonsai's).
const RSA_BITS: c_uint = 4096;
/// How long the certificate is valid: five years.
const VALID_SECONDS: i64 = 5 * 365 * 24 * 3600;
/// The distinguished name gnome-control-center gives it.
pub(crate) const DN: &str = "C=US,CN=GNOME";
pub(crate) const CERT_FILE: &str = "rdp-tls.crt";
pub(crate) const KEY_FILE: &str = "rdp-tls.key";

static LIB: OnceLock<Result<GnuTls, crate::dlib::DlError>> = OnceLock::new();

/// GnuTLS, once per process.
pub(crate) fn lib() -> Option<&'static GnuTls> {
    LIB.get_or_init(GnuTls::load).as_ref().ok()
}

/// A certificate and its key, as PEM text.
pub(crate) struct Pair {
    pub cert: String,
    pub key: String,
}

/// gnutls_free (a variable holding the allocator's free function; libc's free when it cannot be read).
fn free_datum(lib: &GnuTls, d: &mut Datum) {
    if d.data.is_null() {
        return;
    }
    let slot = unsafe { libc::dlsym(lib._handle as *mut c_void, c"gnutls_free".as_ptr()) };
    let free: Option<unsafe extern "C" fn(*mut c_void)> = if slot.is_null() {
        None
    } else {
        unsafe { *(slot as *const Option<unsafe extern "C" fn(*mut c_void)>) }
    };
    unsafe { free.unwrap_or(libc::free)(d.data.cast()) };
    d.data = std::ptr::null_mut();
}

fn datum_text(lib: &GnuTls, mut d: Datum) -> String {
    let text = if d.data.is_null() {
        String::new()
    } else {
        let bytes = unsafe { std::slice::from_raw_parts(d.data, d.size as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    free_datum(lib, &mut d);
    text
}

/// A new key and self-signed certificate (libbonsai's generate worker), or None when GnuTLS is missing or a step
/// fails.
pub(crate) fn generate(dn: &str, now: i64) -> Option<Pair> {
    let lib = lib()?;
    let dn = std::ffi::CString::new(dn).ok()?;
    let mut key: *mut c_void = std::ptr::null_mut();
    let mut crt: *mut c_void = std::ptr::null_mut();
    let result = (|| unsafe {
        let ok = |r: c_int| (r >= 0).then_some(());
        ok((lib.gnutls_x509_privkey_init)(&mut key))?;
        ok((lib.gnutls_x509_privkey_generate)(
            key,
            GNUTLS_PK_RSA,
            RSA_BITS,
            0,
        ))?;
        ok((lib.gnutls_x509_crt_init)(&mut crt))?;
        ok((lib.gnutls_x509_crt_set_version)(crt, 3))?;
        // a positive big-endian serial (the leftmost bit zero), as GnuTLS asks
        let serial = ((now.max(1) as u64) << 16 | glib::random_int() as u64 & 0xffff)
            & 0x7fff_ffff_ffff_ffff;
        let serial = serial.to_be_bytes();
        ok((lib.gnutls_x509_crt_set_serial)(
            crt,
            serial.as_ptr().cast(),
            serial.len(),
        ))?;
        let mut err: *const c_char = std::ptr::null();
        ok((lib.gnutls_x509_crt_set_dn)(crt, dn.as_ptr(), &mut err))?;
        ok((lib.gnutls_x509_crt_set_key)(crt, key))?;
        ok((lib.gnutls_x509_crt_set_activation_time)(
            crt,
            now as time_t,
        ))?;
        ok((lib.gnutls_x509_crt_set_expiration_time)(
            crt,
            (now + VALID_SECONDS) as time_t,
        ))?;
        ok((lib.gnutls_x509_crt_sign)(crt, crt, key))?;
        let mut c = Datum {
            data: std::ptr::null_mut(),
            size: 0,
        };
        ok((lib.gnutls_x509_crt_export2)(
            crt,
            GNUTLS_X509_FMT_PEM,
            &mut c,
        ))?;
        let cert = datum_text(lib, c);
        let mut k = Datum {
            data: std::ptr::null_mut(),
            size: 0,
        };
        ok((lib.gnutls_x509_privkey_export2)(
            key,
            GNUTLS_X509_FMT_PEM,
            &mut k,
        ))?;
        let key_text = datum_text(lib, k);
        Some(Pair {
            cert,
            key: key_text,
        })
    })();
    unsafe {
        if !crt.is_null() {
            (lib.gnutls_x509_crt_deinit)(crt);
        }
        if !key.is_null() {
            (lib.gnutls_x509_privkey_deinit)(key);
        }
    }
    result.filter(|p| p.cert.contains("BEGIN CERTIFICATE") && p.key.contains("PRIVATE KEY"))
}

/// The DER bytes of the first certificate of a PEM text.
pub(crate) fn pem_der(pem: &str) -> Option<Vec<u8>> {
    let begin = "-----BEGIN CERTIFICATE-----";
    let start = pem.find(begin)? + begin.len();
    let end = start + pem[start..].find("-----END CERTIFICATE-----")?;
    let body: String = pem[start..end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if body.is_empty()
        || !body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
    {
        return None;
    }
    let der = glib::base64_decode(&body);
    (!der.is_empty()).then_some(der)
}

/// The fingerprint of DER bytes: SHA-256 as `AB:CD:…` (32 pairs).
pub(crate) fn fingerprint(der: &[u8]) -> String {
    let hex = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, der)
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_default();
    hex.as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

/// The fingerprint of a certificate file (at most 1 MiB), or None.
pub(crate) fn file_fingerprint(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return None;
    }
    pem_der(&std::fs::read_to_string(path).ok()?).map(|der| fingerprint(&der))
}

/// Whether a certificate and key file load as one TLS identity (GIO's check, as GNOME's "Configured TLS certificate
/// invalid").
pub(crate) fn pair_ok(cert: &Path, key: &Path) -> bool {
    cert.is_file() && key.is_file() && gio::TlsCertificate::from_files(cert, key).is_ok()
}

/// Write a file through a temporary file and a rename, with its mode.
pub(crate) fn write_file(path: &Path, text: &str, mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().unwrap_or(Path::new("/"));
    let tmp = dir.join(format!(
        ".{}.{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp"),
        std::process::id()
    ));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Write a pair into `dir` (made 0755 as GNOME's certificates directory; the key 0600): (cert path, key path).
pub(crate) fn save(dir: &Path, pair: &Pair) -> std::io::Result<(PathBuf, PathBuf)> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(dir)?;
    let (cert, key) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
    write_file(&key, &pair.key, 0o600)?;
    write_file(&cert, &pair.cert, 0o644)?;
    Ok((cert, key))
}
