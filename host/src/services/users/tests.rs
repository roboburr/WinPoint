//! Tests of `users.*` (and the fake daemons goa.rs's tests share): pure parsers and mappings, passwd's transcript and
//! a passwd stand-in on a real pseudo-terminal, libcrypt and libpwquality when installed, and the AccountsService and
//! fprintd calls against fakes on tests/rust-sandbox/run-tests.sh's private session bus (each call recorded with
//! whether it allowed interactive authorization, as `call_interactive` must).
use super::*;
use crate::services::system::common::{self, LangContext};
use std::sync::mpsc;
use std::time::Duration;

// ------------------------------------------------------------------------------------------------- the fakes

/// One method call a fake got.
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub path: String,
    pub method: String,
    pub args: String,
    pub interactive: bool,
}

pub(crate) type Reply = Result<Option<glib::Variant>, (&'static str, String)>;
/// (object path, interface, method, parameters) → the reply.
pub(crate) type Methods = Arc<dyn Fn(&str, &str, &str, &glib::Variant) -> Reply + Send + Sync>;
/// (object path, interface, property) → its value.
pub(crate) type Props = Arc<dyn Fn(&str, &str, &str) -> Option<glib::Variant> + Send + Sync>;

/// A private session bus only: tests/rust-sandbox/run-tests.sh's (never the live one under /run/user).
pub(crate) fn private_address() -> Option<String> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()?;
    (std::env::var("GSETTINGS_BACKEND").as_deref() == Ok("keyfile")
        && !address.contains("/run/user/"))
    .then_some(address)
}

/// A fake daemon on the private session bus: its objects (path, interface of `xml`), on a thread and MainContext of
/// its own; property writes are recorded as a `Set` call (`prop = value`).
pub(crate) struct FakeBus {
    pub name: String,
    pub conn: gio::DBusConnection,
    calls: Arc<Mutex<Vec<Call>>>,
    context: glib::MainContext,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeBus {
    pub(crate) fn start(
        xml: &'static str,
        objects: Vec<(String, &'static str)>,
        methods: Methods,
        props: Props,
    ) -> Option<FakeBus> {
        let address = private_address()?;
        let calls: Arc<Mutex<Vec<Call>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let (thread_calls, thread_stop) = (calls.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let connection = gio::DBusConnection::for_address_sync(
                    &address,
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                        | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None,
                    gio::Cancellable::NONE,
                )
                .unwrap();
                let node = gio::DBusNodeInfo::for_xml(xml).unwrap();
                let mut ids = Vec::new();
                for (path, iface) in &objects {
                    let info = node.lookup_interface(iface).unwrap();
                    let (calls, methods, props, set_calls) = (
                        thread_calls.clone(),
                        methods.clone(),
                        props.clone(),
                        thread_calls.clone(),
                    );
                    let id = connection
                        .register_object(
                            path,
                            &info,
                            move |_, _, path, iface, method, params, invocation| {
                                let interactive = invocation.message().flags().contains(
                                    gio::DBusMessageFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                                );
                                calls.lock().unwrap().push(Call {
                                    path: path.into(),
                                    method: method.into(),
                                    args: params.print(true).to_string(),
                                    interactive,
                                });
                                match methods(path, iface, method, &params) {
                                    Ok(v) => invocation.return_value(v.as_ref()),
                                    Err((name, message)) => {
                                        invocation.return_dbus_error(name, &message)
                                    }
                                }
                            },
                            move |_, _, path, iface, prop| {
                                props(path, iface, prop).unwrap_or_else(|| "".to_variant())
                            },
                            move |_, _, path, _, prop, value| {
                                set_calls.lock().unwrap().push(Call {
                                    path: path.into(),
                                    method: "Set".into(),
                                    args: format!("{prop} = {}", value.print(true)),
                                    interactive: false,
                                });
                                true
                            },
                        )
                        .unwrap();
                    ids.push(id);
                }
                ready
                    .send((
                        connection.unique_name().unwrap().to_string(),
                        connection.clone(),
                        context.clone(),
                    ))
                    .unwrap();
                while !thread_stop.load(Ordering::Acquire) {
                    context.iteration(true);
                }
                for id in ids {
                    let _ = connection.unregister_object(id);
                }
            });
        });
        let (name, conn, context) = started.recv_timeout(Duration::from_secs(10)).ok()?;
        Some(FakeBus {
            name,
            conn,
            calls,
            context,
            stop,
            thread: Some(thread),
        })
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub(crate) fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }

    /// The calls other than reads (GetAll, Get and the lookups a page makes first).
    pub(crate) fn writes(&self, reads: &[&str]) -> Vec<(String, String, bool)> {
        self.calls()
            .into_iter()
            .filter(|c| !reads.contains(&c.method.as_str()))
            .map(|c| (c.method, c.args, c.interactive))
            .collect()
    }

    /// Wait until a call of `method` arrives (signals make the service call back from another thread).
    pub(crate) fn wait_for(&self, method: &str) -> bool {
        for _ in 0..200 {
            if self.calls().iter().any(|c| c.method == method) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }
}

impl Drop for FakeBus {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.context.wakeup();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A service context of its own (a temporary config dir), with or without `--dry-run`.
pub(crate) fn context(dry: bool) -> (LangContext, Arc<ServiceContext>) {
    let lang = LangContext::new("en", "settings");
    let ctx = ServiceContext::new(
        lang.ctx.events.clone(),
        lang.root.join("config"),
        lang.root.join("cache"),
        lang.root.clone(),
        "settings".into(),
        dry,
    )
    .unwrap();
    (lang, ctx)
}

// ------------------------------------------------------------------------------------------- AccountsService

const ACCOUNTS_XML: &str = r#"<node>
  <interface name="org.freedesktop.Accounts">
    <method name="FindUserById"><arg type="x" direction="in"/><arg type="o" direction="out"/></method>
    <method name="ListCachedUsers"><arg type="ao" direction="out"/></method>
    <method name="CreateUser"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="i" direction="in"/><arg type="o" direction="out"/></method>
    <method name="DeleteUser"><arg type="x" direction="in"/><arg type="b" direction="in"/></method>
  </interface>
  <interface name="org.freedesktop.Accounts.User">
    <method name="SetRealName"><arg type="s" direction="in"/></method>
    <method name="SetIconFile"><arg type="s" direction="in"/></method>
    <method name="SetLanguage"><arg type="s" direction="in"/></method>
    <method name="SetFormatsLocale"><arg type="s" direction="in"/></method>
    <method name="SetAccountType"><arg type="i" direction="in"/></method>
    <method name="SetAutomaticLogin"><arg type="b" direction="in"/></method>
    <method name="SetPassword"><arg type="s" direction="in"/><arg type="s" direction="in"/></method>
    <method name="SetPasswordMode"><arg type="i" direction="in"/></method>
    <property name="Uid" type="t" access="read"/>
    <property name="UserName" type="s" access="read"/>
    <property name="RealName" type="s" access="read"/>
    <property name="AccountType" type="i" access="read"/>
    <property name="IconFile" type="s" access="read"/>
    <property name="AutomaticLogin" type="b" access="read"/>
    <property name="PasswordMode" type="i" access="read"/>
    <property name="Language" type="s" access="read"/>
    <property name="Locked" type="b" access="read"/>
    <property name="LocalAccount" type="b" access="read"/>
    <property name="SystemAccount" type="b" access="read"/>
  </interface>
</node>"#;

/// The uid of the fake's other user and of its system account.
fn anna() -> u64 {
    accounts::self_uid() + 1
}
const SYSTEM_UID: u64 = 125;

fn accounts_fake() -> Option<FakeBus> {
    let me = accounts::self_uid();
    let path = |uid: u64| format!("/org/freedesktop/Accounts/User{uid}");
    let objects = vec![
        (
            "/org/freedesktop/Accounts".to_owned(),
            "org.freedesktop.Accounts",
        ),
        (path(me), "org.freedesktop.Accounts.User"),
        (path(anna()), "org.freedesktop.Accounts.User"),
        (path(SYSTEM_UID), "org.freedesktop.Accounts.User"),
    ];
    let methods: Methods = Arc::new(move |_, _, method, params| match method {
        "FindUserById" => {
            let uid = params.child_value(0).get::<i64>().unwrap_or(-1);
            if uid == me as i64 || uid == anna() as i64 || uid == SYSTEM_UID as i64 {
                Ok(Some(
                    (glib::variant::ObjectPath::try_from(format!(
                        "/org/freedesktop/Accounts/User{uid}"
                    ))
                    .unwrap(),)
                        .to_variant(),
                ))
            } else {
                Err((
                    "org.freedesktop.Accounts.Error.Failed",
                    format!("Failed to look up user with uid {uid}."),
                ))
            }
        }
        "ListCachedUsers" => {
            let list: Vec<glib::variant::ObjectPath> = [anna(), SYSTEM_UID]
                .iter()
                .map(|u| {
                    glib::variant::ObjectPath::try_from(format!(
                        "/org/freedesktop/Accounts/User{u}"
                    ))
                    .unwrap()
                })
                .collect();
            Ok(Some((list,).to_variant()))
        }
        "CreateUser" => Ok(Some(
            (glib::variant::ObjectPath::try_from(format!(
                "/org/freedesktop/Accounts/User{}",
                anna()
            ))
            .unwrap(),)
                .to_variant(),
        )),
        _ => Ok(None),
    });
    let props: Props = Arc::new(move |path, _, prop| {
        let uid: u64 = path.rsplit("User").next()?.parse().ok()?;
        let (name, real, kind) = if uid == me {
            ("ik", "Ik Zelf", 1)
        } else if uid == SYSTEM_UID {
            ("gdm", "Gnome Display Manager", 0)
        } else {
            ("anna", "Anna de Vries", 0)
        };
        Some(match prop {
            "Uid" => uid.to_variant(),
            "UserName" => name.to_variant(),
            "RealName" => real.to_variant(),
            "AccountType" => (kind as i32).to_variant(),
            "IconFile" => "/var/lib/AccountsService/icons/none-such".to_variant(),
            "AutomaticLogin" => (uid == anna()).to_variant(),
            "PasswordMode" => 0i32.to_variant(),
            // what Ubuntu's accounts-daemon stores (language-validate drops the codeset); the own account has none
            "Language" => if uid == me {
                ""
            } else if uid == SYSTEM_UID {
                "nl_NL.UTF-8"
            } else {
                "nl_NL"
            }
            .to_variant(),
            "Locked" => false.to_variant(),
            "LocalAccount" => true.to_variant(),
            "SystemAccount" => (uid == SYSTEM_UID).to_variant(),
            _ => return None,
        })
    });
    FakeBus::start(ACCOUNTS_XML, objects, methods, props)
}

const LOGIN_XML: &str = r#"<node>
  <interface name="org.freedesktop.login1.Manager">
    <method name="ListSessions"><arg type="a(susso)" direction="out"/></method>
  </interface>
  <interface name="org.freedesktop.login1.Session">
    <property name="Class" type="s" access="read"/>
    <property name="State" type="s" access="read"/>
  </interface>
</node>"#;

/// logind with one session per (uid, class, state).
fn login_fake(sessions: Vec<(u64, &'static str, &'static str)>) -> Option<FakeBus> {
    let path = |i: usize| format!("/org/freedesktop/login1/session/_3{i}");
    let mut objects = vec![(accounts::LOGIN_PATH.to_owned(), accounts::LOGIN_MANAGER)];
    objects.extend((0..sessions.len()).map(|i| (path(i), accounts::LOGIN_SESSION)));
    let list = sessions.clone();
    let methods: Methods = Arc::new(move |_, _, method, _| match method {
        "ListSessions" => {
            let rows: Vec<(String, u32, String, String, glib::variant::ObjectPath)> = list
                .iter()
                .enumerate()
                .map(|(i, (uid, _, _))| {
                    (
                        format!("{}", 30 + i),
                        *uid as u32,
                        format!("user{uid}"),
                        "seat0".to_owned(),
                        glib::variant::ObjectPath::try_from(path(i)).unwrap(),
                    )
                })
                .collect();
            Ok(Some((rows,).to_variant()))
        }
        _ => Ok(None),
    });
    let props: Props = Arc::new(move |p, _, prop| {
        let i: usize = p.rsplit("_3").next()?.parse().ok()?;
        let (_, class, state) = sessions.get(i)?;
        Some(match prop {
            "Class" => class.to_variant(),
            "State" => state.to_variant(),
            _ => return None,
        })
    });
    FakeBus::start(LOGIN_XML, objects, methods, props)
}

/// The locales an account's language resolves against in these tests.
fn test_locales() -> crate::services::machine::locale::LocaleData {
    crate::services::machine::locale::LocaleData {
        generated: Arc::new(vec![
            "en_GB.UTF-8".into(),
            "en_US.UTF-8".into(),
            "nl_BE.UTF-8".into(),
            "nl_NL.UTF-8".into(),
        ]),
        main: Arc::new(
            [("en", "en_US"), ("nl", "nl_NL")]
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        ),
    }
}

/// Users on the AccountsService fake, with a logind fake (the own account and the greeter have a session; anna's is
/// closing, so she is not logged in) and fixed locales.
fn users_on(fake: &FakeBus, dry: bool) -> (LangContext, Arc<Users>, FakeBus) {
    let login = login_fake(vec![
        (accounts::self_uid(), "user", "active"),
        (SYSTEM_UID, "greeter", "online"),
        (anna(), "user", "closing"),
    ])
    .expect("the logind fake");
    let (lang, users) = users_with(fake, dry, &login);
    (lang, users, login)
}

fn users_with(fake: &FakeBus, dry: bool, login: &FakeBus) -> (LangContext, Arc<Users>) {
    let (lang, ctx) = context(dry);
    let bus = common::session().unwrap();
    let users = Users::with(
        ctx,
        Buses {
            accounts: Some(Daemon {
                bus: bus.clone(),
                name: fake.name.clone(),
            }),
            fprint: None,
            passwd: None,
            login: Some(Daemon {
                bus,
                name: login.name.clone(),
            }),
            locales: Some(test_locales()),
        },
    )
    .unwrap();
    (lang, users)
}

const READS: &[&str] = &["GetAll", "Get", "FindUserById", "ListCachedUsers"];

#[test]
fn accountsservice_calls_ask_polkit_and_carry_gnomes_arguments() {
    let Some(fake) = accounts_fake() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let (_lang, users, _login) = users_on(&fake, false);
    let call = |m: &str, a: Value| {
        users
            .call(m, &a)
            .expect("a users method")
            .map_err(|e| e.key.unwrap_or(e.code))
    };
    let me = call("users.self", json!({})).unwrap();
    assert_eq!(
        (
            me["userName"].as_str(),
            me["accountType"].as_str(),
            me["self"].as_bool()
        ),
        (Some("ik"), Some("administrator"), Some(true))
    );
    assert_eq!(
        me["iconFile"], "",
        "a picture file that does not exist is no picture"
    );
    assert_eq!(me["available"], true);
    let list = call("users.list", json!({})).unwrap();
    let names: Vec<&str> = list["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["userName"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["ik", "anna"],
        "the own account first, no system account"
    );
    assert_eq!(list["administrators"], 1);
    // Ubuntu stores "nl_NL": the API's language is the generated locale machine.locales lists, the raw one kept
    let anna_json = &list["users"][1];
    assert_eq!(
        (
            anna_json["language"].as_str(),
            anna_json["languageRaw"].as_str()
        ),
        (Some("nl_NL.UTF-8"), Some("nl_NL"))
    );
    assert_eq!(
        (
            anna_json["loggedIn"].as_bool(),
            list["users"][0]["loggedIn"].as_bool()
        ),
        (Some(false), Some(true)),
        "a closing session is no login (libaccountsservice), the own account is logged in"
    );
    assert_eq!(list["users"][0]["languageRaw"], "");
    fake.clear();
    call("users.setRealName", json!({"name": "  Ik Zelf  "})).unwrap();
    assert_eq!(
        call("users.setRealName", json!({"name": "a:b"})).unwrap_err(),
        "users.badName"
    );
    call(
        "users.setLanguage",
        json!({"uid": anna(), "language": "en_GB.UTF-8"}),
    )
    .unwrap();
    assert_eq!(
        call("users.setLanguage", json!({"language": "nl; rm -rf"})).unwrap_err(),
        "users.badLanguage"
    );
    call("users.setFormats", json!({"locale": "nl_NL.UTF-8"})).unwrap();
    assert_eq!(
        call("users.setFormats", json!({"locale": "nl NL"})).unwrap_err(),
        "users.badLanguage"
    );
    assert_eq!(
        call(
            "users.setAccountType",
            json!({"uid": accounts::self_uid(), "type": "standard"})
        )
        .unwrap_err(),
        "users.lastAdmin"
    );
    call(
        "users.setAccountType",
        json!({"uid": anna(), "type": "administrator"}),
    )
    .unwrap();
    call("users.setAutoLogin", json!({"on": true})).unwrap();
    assert_eq!(
        call(
            "users.delete",
            json!({"uid": accounts::self_uid(), "removeFiles": true})
        )
        .unwrap_err(),
        "users.deleteSelf"
    );
    call("users.delete", json!({"uid": anna(), "removeFiles": false})).unwrap();
    assert_eq!(
        call("users.delete", json!({"uid": 4242, "removeFiles": false})).unwrap_err(),
        "users.noSuchUser"
    );
    call(
        "users.setPassword",
        json!({"uid": anna(), "mode": "atLogin"}),
    )
    .unwrap();
    assert_eq!(
        call(
            "users.create",
            json!({"realName": "Bo", "userName": "Bo!", "accountType": "standard"})
        )
        .unwrap_err(),
        "users.badUserName"
    );
    assert_eq!(
        call(
            "users.create",
            json!({"realName": "Root", "userName": "root"})
        )
        .unwrap_err(),
        "users.userNameTaken"
    );
    call(
        "users.create",
        json!({"realName": "Bo Jansen", "userName": "bo", "accountType": "administrator"}),
    )
    .unwrap();
    let me_path = format!("/org/freedesktop/Accounts/User{}", accounts::self_uid());
    let writes = fake.writes(READS);
    assert_eq!(
        writes,
        vec![
            ("SetRealName".to_owned(), "('Ik Zelf',)".to_owned(), true),
            ("SetLanguage".into(), "('en_GB.UTF-8',)".into(), true),
            ("SetFormatsLocale".into(), "('nl_NL.UTF-8',)".into(), true),
            ("SetAccountType".into(), "(1,)".into(), true),
            ("SetAutomaticLogin".into(), "(true,)".into(), true),
            (
                "DeleteUser".into(),
                format!("(int64 {}, false)", anna()),
                true
            ),
            ("SetPasswordMode".into(), "(1,)".into(), true),
            ("CreateUser".into(), "('bo', 'Bo Jansen', 1)".into(), true),
            ("SetPasswordMode".into(), "(1,)".into(), true),
        ]
    );
    let paths: Vec<String> = fake
        .calls()
        .into_iter()
        .filter(|c| c.method == "SetRealName" || c.method == "SetAutomaticLogin")
        .map(|c| c.path)
        .collect();
    assert_eq!(
        paths,
        [me_path.clone(), me_path],
        "without uid: the own account"
    );
    if crypt::available() {
        fake.clear();
        call(
            "users.setPassword",
            json!({"uid": anna(), "mode": "now", "password": "Een lang geheim 42!"}),
        )
        .unwrap();
        let set = fake
            .calls()
            .into_iter()
            .find(|c| c.method == "SetPassword")
            .unwrap();
        assert!(
            set.interactive && set.args.starts_with("('$") && set.args.ends_with("', '')"),
            "a crypt(3) hash and no hint: {}",
            set.args.len()
        );
        assert!(!set.args.contains("geheim"), "never the password itself");
    }
}

#[test]
fn a_dry_run_checks_the_arguments_and_writes_nothing() {
    let Some(fake) = accounts_fake() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let (_lang, users, _login) = users_on(&fake, true);
    let face = faces(Path::new(FACES_DIR))["faces"][0]["path"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_default();
    for (method, args) in [
        ("users.setRealName", json!({"name": "Iemand"})),
        ("users.setIcon", json!({"path": ""})),
        ("users.setLanguage", json!({"language": "nl_NL.UTF-8"})),
        ("users.setFormats", json!({"locale": "nl_NL.UTF-8"})),
        (
            "users.setAccountType",
            json!({"uid": anna(), "type": "standard"}),
        ),
        ("users.setAutoLogin", json!({"uid": anna(), "on": false})),
        ("users.create", json!({"realName": "Bo", "userName": "bo"})),
        ("users.delete", json!({"uid": anna(), "removeFiles": true})),
        (
            "users.setPassword",
            json!({"uid": anna(), "mode": "atLogin"}),
        ),
        ("users.fingerprint.enroll", json!({"finger": "left-thumb"})),
        ("users.fingerprint.cancel", json!({})),
        ("users.fingerprint.delete", json!({})),
    ] {
        let answer = users
            .call(method, &args)
            .unwrap()
            .unwrap_or_else(|e| panic!("{method}: {}", e.message));
        assert_eq!(answer["dryRun"], true, "{method}");
    }
    if !face.is_empty() {
        assert_eq!(
            users
                .call("users.setIcon", &json!({"path": face}))
                .unwrap()
                .unwrap()["dryRun"],
            true
        );
    }
    // the checks still run
    let bad = users
        .call(
            "users.create",
            &json!({"realName": "Bo", "userName": "-bo"}),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(bad.key.as_deref(), Some("users.badUserName"));
    let missing = users
        .call("users.setIcon", &json!({"path": "/no/such/picture.png"}))
        .unwrap()
        .unwrap_err();
    assert_eq!(missing.key.as_deref(), Some("users.badPicture"));
    assert!(fake.writes(READS).is_empty(), "{:?}", fake.writes(READS));
}

#[test]
fn a_chosen_picture_is_scaled_and_a_stock_one_goes_as_it_is() {
    let Some(fake) = accounts_fake() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let (lang, users, _login) = users_on(&fake, false);
    let big = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 900, 600).unwrap();
    big.fill(0x3366ccff);
    let source = lang.root.join("photo.png");
    big.savev(&source, "png", &[]).unwrap();
    fake.clear();
    users
        .call("users.setIcon", &json!({"path": source.to_str().unwrap()}))
        .unwrap()
        .unwrap();
    let set = fake
        .calls()
        .into_iter()
        .find(|c| c.method == "SetIconFile")
        .unwrap();
    assert!(set.interactive);
    let sent = set
        .args
        .trim_start_matches("('")
        .trim_end_matches("',)")
        .to_owned();
    assert!(
        sent.starts_with(lang.root.join("cache").to_str().unwrap()) && sent.ends_with(".png"),
        "{sent}"
    );
    assert!(
        !Path::new(&sent).exists(),
        "the scaled copy is removed after the call"
    );
    let scaled = scale_icon(&source, &lang.root.join("cache")).unwrap();
    let pb = gdk_pixbuf::Pixbuf::from_file(&scaled).unwrap();
    assert_eq!((pb.width(), pb.height()), (512, 512), "square, 512 px");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&scaled).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fake.clear();
    users
        .call("users.setIcon", &json!({"path": ""}))
        .unwrap()
        .unwrap();
    assert_eq!(
        fake.writes(READS),
        vec![("SetIconFile".into(), "('',)".into(), true)],
        "an empty path removes the picture"
    );
    if let Some(face) = faces(Path::new(FACES_DIR))["faces"][0]["path"].as_str() {
        fake.clear();
        users
            .call("users.setIcon", &json!({"path": face}))
            .unwrap()
            .unwrap();
        assert_eq!(
            fake.writes(READS),
            vec![("SetIconFile".into(), format!("('{face}',)"), true)]
        );
    }
}

#[test]
fn a_user_who_is_still_signed_in_is_not_deleted() {
    let Some(fake) = accounts_fake() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    // anna has a running session behind "Gebruiker wisselen"; the greeter's session is no login
    let login = login_fake(vec![
        (SYSTEM_UID, "greeter", "online"),
        (anna(), "user", "online"),
    ])
    .expect("the logind fake");
    for dry in [false, true] {
        let (_lang, users) = users_with(&fake, dry, &login);
        fake.clear();
        let refused = users
            .call("users.delete", &json!({"uid": anna(), "removeFiles": true}))
            .unwrap()
            .unwrap_err();
        assert_eq!(
            (refused.code.as_str(), refused.key.as_deref()),
            ("denied", Some("users.loggedIn")),
            "dry run {dry}"
        );
        assert!(
            fake.writes(READS).is_empty(),
            "no DeleteUser: {:?}",
            fake.writes(READS)
        );
    }
    let (_lang, users) = users_with(&fake, false, &login);
    let list = users.call("users.list", &json!({})).unwrap().unwrap();
    let anna_json = list["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["uid"] == anna())
        .unwrap();
    assert_eq!(
        anna_json["loggedIn"], true,
        "the page can say so before asking"
    );
    assert!(
        login.calls().iter().any(|c| c.method == "ListSessions"),
        "{:?}",
        login.calls()
    );
}

#[test]
fn an_account_language_is_the_generated_locale_ubuntu_means() {
    use crate::services::machine::locale::{account_language, language_locale};
    let data = test_locales();
    for (raw, want) in [
        ("nl_NL", Some("nl_NL.UTF-8")),
        ("nl_NL.UTF-8", Some("nl_NL.UTF-8")),
        ("nl_NL.utf8", Some("nl_NL.UTF-8")),
        ("nl_BE", Some("nl_BE.UTF-8")),
        ("nl", Some("nl_NL.UTF-8")),
        ("en", Some("en_US.UTF-8")),
        ("en:nl", Some("en_US.UTF-8")),
        ("de_DE", None),
        ("de", None),
        ("C.UTF-8", None),
        ("", None),
    ] {
        assert_eq!(language_locale(raw, &data).as_deref(), want, "{raw}");
    }
    // a language without a main country takes its first generated locale; Chinese does not guess
    let other = crate::services::machine::locale::LocaleData {
        generated: Arc::new(vec![
            "fy_DE.UTF-8".into(),
            "fy_NL.UTF-8".into(),
            "sr_RS.UTF-8@latin".into(),
            "zh_TW.UTF-8".into(),
        ]),
        main: Arc::new(
            [("sr", "sr_RS")]
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        ),
    };
    assert_eq!(
        language_locale("fy", &other).as_deref(),
        Some("fy_DE.UTF-8")
    );
    assert_eq!(
        language_locale("sr@latin", &other).as_deref(),
        Some("sr_RS.UTF-8@latin")
    );
    assert_eq!(
        language_locale("sr_RS@latin", &other).as_deref(),
        Some("sr_RS.UTF-8@latin")
    );
    assert_eq!(language_locale("zh", &other), None);
    // what users.* reports: the own account without a language has the session's; none fits: the value itself
    assert_eq!(account_language("", "nl_NL.UTF-8", &data), "nl_NL.UTF-8");
    assert_eq!(account_language("", "", &data), "");
    assert_eq!(
        account_language("en_GB", "nl_NL.UTF-8", &data),
        "en_GB.UTF-8"
    );
    assert_eq!(account_language("de_DE", "", &data), "de_DE");
    assert_eq!(
        crate::services::machine::locale::parse_main_countries("# x\naa\taa_ET\nnl\tnl_NL\n")["nl"],
        "nl_NL"
    );
}

// ------------------------------------------------------------------------------------------------ fprintd

const FPRINT_XML: &str = r#"<node>
  <interface name="net.reactivated.Fprint.Manager">
    <method name="GetDefaultDevice"><arg type="o" direction="out"/></method>
  </interface>
  <interface name="net.reactivated.Fprint.Device">
    <method name="Claim"><arg type="s" direction="in"/></method>
    <method name="Release"/>
    <method name="ListEnrolledFingers"><arg type="s" direction="in"/><arg type="as" direction="out"/></method>
    <method name="EnrollStart"><arg type="s" direction="in"/></method>
    <method name="EnrollStop"/>
    <method name="DeleteEnrolledFingers2"/>
    <signal name="EnrollStatus"><arg type="s"/><arg type="b"/></signal>
    <property name="name" type="s" access="read"/>
    <property name="num-enroll-stages" type="i" access="read"/>
    <property name="scan-type" type="s" access="read"/>
  </interface>
</node>"#;
const DEVICE: &str = "/net/reactivated/Fprint/Device/0";

fn fprint_fake() -> Option<FakeBus> {
    let objects = vec![
        (fprint::MANAGER_PATH.to_owned(), fprint::MANAGER_IFACE),
        (DEVICE.to_owned(), fprint::DEVICE_IFACE),
    ];
    let methods: Methods = Arc::new(|_, _, method, _| match method {
        "GetDefaultDevice" => Ok(Some(
            (glib::variant::ObjectPath::try_from(DEVICE).unwrap(),).to_variant(),
        )),
        "ListEnrolledFingers" => Ok(Some((vec!["right-index-finger"],).to_variant())),
        _ => Ok(None),
    });
    let props: Props = Arc::new(|_, _, prop| {
        Some(match prop {
            "name" => "Fake Reader".to_variant(),
            "num-enroll-stages" => 3i32.to_variant(),
            "scan-type" => "swipe".to_variant(),
            _ => return None,
        })
    });
    FakeBus::start(FPRINT_XML, objects, methods, props)
}

#[test]
fn fingerprint_enrolment_claims_the_reader_and_releases_it_when_done() {
    let Some(fake) = fprint_fake() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let (_lang, ctx) = context(false);
    let bus = common::session().unwrap();
    let f = Fprint {
        bus,
        name: fake.name.clone(),
    };
    let users = Users::with(
        ctx,
        Buses {
            accounts: None,
            fprint: Some(f),
            ..Buses::default()
        },
    )
    .unwrap();
    let get = users
        .call("users.fingerprint.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(get["available"], true);
    assert_eq!(get["device"]["scanType"], "swipe");
    assert_eq!(get["device"]["stages"], 3);
    assert_eq!(get["enrolled"], json!(["right-index-finger"]));
    assert_eq!(
        users
            .call("users.fingerprint.enroll", &json!({"finger": "nose"}))
            .unwrap()
            .unwrap_err()
            .code,
        "bad_args"
    );
    fake.clear();
    let started = users
        .call("users.fingerprint.enroll", &json!({"finger": "left-thumb"}))
        .unwrap()
        .unwrap();
    assert_eq!(started["stages"], 3);
    // the watcher follows the reader's EnrollStatus: done releases the claim
    for (result, done) in [
        ("enroll-stage-passed", false),
        ("enroll-retry-scan", false),
        ("enroll-completed", true),
    ] {
        fake.conn
            .emit_signal(
                None,
                DEVICE,
                fprint::DEVICE_IFACE,
                "EnrollStatus",
                Some(&(result, done).to_variant()),
            )
            .unwrap();
    }
    assert!(fake.wait_for("Release"), "{:?}", fake.calls());
    let seen: Vec<(String, String, bool)> =
        fake.writes(&["GetDefaultDevice", "GetAll", "ListEnrolledFingers"]);
    assert_eq!(
        seen,
        vec![
            ("Claim".to_owned(), "('',)".to_owned(), true),
            ("EnrollStart".into(), "('left-thumb',)".into(), true),
            ("EnrollStop".into(), "()".into(), false),
            ("Release".into(), "()".into(), false),
        ]
    );
    fake.clear();
    users
        .call("users.fingerprint.delete", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(
        fake.writes(&["GetDefaultDevice", "GetAll", "ListEnrolledFingers"]),
        vec![
            ("Claim".to_owned(), "('',)".to_owned(), true),
            ("DeleteEnrolledFingers2".into(), "()".into(), true),
            ("Release".into(), "()".into(), false)
        ]
    );
    users.shutdown();
}

#[test]
fn enrolment_progress_counts_the_stages() {
    let mut e = fprint::Enrolment {
        device: DEVICE.into(),
        stages: 3,
        passed: 0,
    };
    assert_eq!(
        e.status("enroll-stage-passed", false),
        json!({"stage": 1, "stages": 3, "done": false, "result": "enroll-stage-passed"})
    );
    assert_eq!(
        e.status("enroll-finger-not-centered", false)["stage"],
        1,
        "a retry is no stage"
    );
    assert_eq!(e.status("enroll-completed", true)["stage"], 3);
    assert!(fprint::FINGERS.contains(&"right-index-finger") && fprint::FINGERS.len() == 10);
}

#[test]
fn a_reader_in_use_is_busy_in_any_language() {
    let objects = vec![(DEVICE.to_owned(), fprint::DEVICE_IFACE)];
    // fprintd translates its messages into the system's language: only the error's name says what happened
    let methods: Methods = Arc::new(|_, _, method, _| match method {
        "Claim" => Err((
            fprint::ALREADY_IN_USE,
            "Apparaat is al in gebruik door een andere gebruiker".into(),
        )),
        _ => Ok(None),
    });
    let props: Props = Arc::new(|_, _, _| None);
    let Some(fake) = FakeBus::start(FPRINT_XML, objects, methods, props) else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let f = Fprint {
        bus: common::session().unwrap(),
        name: fake.name.clone(),
    };
    let busy = f.claim(DEVICE, Lang::En).unwrap_err();
    assert_eq!(busy.key.as_deref(), Some("users.fingerprint.busy"));
    assert!(
        fake.calls()
            .iter()
            .any(|c| c.method == "Claim" && c.interactive),
        "the claim may ask polkit: {:?}",
        fake.calls()
    );
    // other errors keep call_interactive's mapping
    let refused = fprint::claim_error(
        "GDBus.Error:net.reactivated.Fprint.Error.PermissionDenied: Niet geautoriseerd",
        Lang::En,
    );
    assert_eq!(refused.key.as_deref(), Some("core.notAuthorized"));
    let other = fprint::claim_error(
        "GDBus.Error:net.reactivated.Fprint.Error.Internal: Interne fout",
        Lang::En,
    );
    assert_eq!(other.key.as_deref(), Some("core.callFailed"));
}

// ------------------------------------------------------------------------------------------------- passwd

fn feed_all(t: &mut passwd::Transcript, chunks: &[&str]) -> Vec<passwd::Step> {
    chunks.iter().flat_map(|c| t.feed(c)).collect()
}

#[test]
fn passwd_transcripts_read_as_pam_says_them() {
    use passwd::{Outcome::*, Step::*};
    // a change that works (Ubuntu 26.04's pam_unix, LC_ALL=C), with prompts split over reads
    let mut t = passwd::Transcript::default();
    assert_eq!(
        feed_all(
            &mut t,
            &[
                "Changing password for ik.\r\nCurrent pass",
                "word: ",
                "\r\nNew password: ",
                "\r\nRetype new password: ",
                "\r\npasswd: password updated successfully\r\n"
            ]
        ),
        vec![Current, New, Retype, Done(Changed)]
    );
    // the current password is wrong
    let mut t = passwd::Transcript::default();
    assert_eq!(
        feed_all(
            &mut t,
            &[
                "Changing password for ik.\nCurrent password: ",
                "\npasswd: Authentication token manipulation error\npasswd: password unchanged\n"
            ]
        ),
        vec![Current, Done(WrongCurrent)]
    );
    // pam_pwquality refuses the new one and asks again: stop there
    let mut t = passwd::Transcript::default();
    assert_eq!(
        feed_all(
            &mut t,
            &[
                "Current password: ",
                "\nNew password: ",
                "\nBAD PASSWORD: The password is shorter than 8 characters\nNew password: "
            ]
        ),
        vec![Current, New, Done(TooWeak)]
    );
    // pam_unix's own length rule, older prompts
    let mut t = passwd::Transcript::default();
    assert_eq!(
        feed_all(
            &mut t,
            &[
                "(current) UNIX password: ",
                "\nEnter new UNIX password: ",
                "\nRetype new UNIX password: ",
                "\nYou must choose a longer password.\nEnter new UNIX password: "
            ]
        ),
        vec![Current, New, Retype, Done(TooWeak)]
    );
    // the minimum age
    let mut t = passwd::Transcript::default();
    assert_eq!(
        feed_all(
            &mut t,
            &[
                "Current password: ",
                "\nYou must wait longer to change your password.\npasswd: Authentication token manipulation error\n"
            ]
        ),
        vec![Current, Done(TooSoon)]
    );
    // the output ends without a message
    let mut t = passwd::Transcript::default();
    feed_all(
        &mut t,
        &[
            "Current password: ",
            "\nNew password: ",
            "\nRetype new password: ",
        ],
    );
    assert_eq!(t.finish(true), Changed);
    let mut t = passwd::Transcript::default();
    feed_all(&mut t, &["Current password: "]);
    assert_eq!(t.finish(false), Unchanged);
}

/// A passwd stand-in: the three prompts on its terminal, success for the current password "oud".
fn fake_passwd(dir: &Path) -> PathBuf {
    let path = dir.join("passwd");
    std::fs::write(
        &path,
        "#!/bin/sh\n[ -t 0 ] || { echo 'no terminal'; exit 3; }\nstty -echo\nprintf 'Changing password for ik.\\n'\nprintf 'Current password: '; read cur; printf '\\n'\nif [ \"$cur\" != oud ]; then echo 'passwd: Authentication token manipulation error'; echo 'passwd: password unchanged'; exit 10; fi\nprintf 'New password: '; read new; printf '\\n'\nif [ ${#new} -lt 8 ]; then echo 'BAD PASSWORD: The password is shorter than 8 characters'; printf 'New password: '; read again; exit 10; fi\nprintf 'Retype new password: '; read again; printf '\\n'\n[ \"$new\" = \"$again\" ] && echo 'passwd: password updated successfully' && exit 0\nexit 10\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[test]
fn passwd_runs_on_its_own_terminal_and_is_stopped_where_it_would_retry() {
    let (lang, ctx) = context(false);
    let program = fake_passwd(&lang.root);
    assert_eq!(
        passwd::change(&program, "oud", "Een nieuw wachtwoord").unwrap(),
        passwd::Outcome::Changed
    );
    assert_eq!(
        passwd::change(&program, "fout", "Een nieuw wachtwoord").unwrap(),
        passwd::Outcome::WrongCurrent
    );
    let started = std::time::Instant::now();
    assert_eq!(
        passwd::change(&program, "oud", "kort").unwrap(),
        passwd::Outcome::TooWeak
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "no wait for a retry"
    );
    // the method: errors with their keys, the passwords nowhere in them
    let users = Users::with(
        ctx,
        Buses {
            accounts: None,
            fprint: None,
            passwd: Some(program),
            ..Buses::default()
        },
    )
    .unwrap();
    assert_eq!(
        users
            .call(
                "users.changePassword",
                &json!({"current": "oud", "new": "Een nieuw wachtwoord"})
            )
            .unwrap()
            .unwrap()["ok"],
        true
    );
    let wrong = users
        .call(
            "users.changePassword",
            &json!({"current": "fout", "new": "Een nieuw wachtwoord"}),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(wrong.key.as_deref(), Some("users.passwd.wrongCurrent"));
    let weak = users
        .call(
            "users.changePassword",
            &json!({"current": "oud", "new": "kort"}),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(weak.key.as_deref(), Some("users.passwd.tooWeak"));
    assert!(!weak.message.contains("kort") && !weak.json().to_string().contains("oud"));
    let empty = users
        .call("users.changePassword", &json!({"current": "", "new": "x"}))
        .unwrap()
        .unwrap_err();
    assert_eq!(empty.key.as_deref(), Some("users.passwordRequired"));
    let newline = users
        .call(
            "users.changePassword",
            &json!({"current": "oud", "new": "a\nb"}),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(newline.code, "bad_args");
}

#[test]
fn a_dry_run_password_change_runs_nothing() {
    let (lang, ctx) = context(true);
    let marker = lang.root.join("ran");
    let program = lang.root.join("passwd");
    std::fs::write(&program, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let users = Users::with(
        ctx,
        Buses {
            accounts: None,
            fprint: None,
            passwd: Some(program),
            ..Buses::default()
        },
    )
    .unwrap();
    assert_eq!(
        users
            .call("users.changePassword", &json!({"current": "a", "new": "b"}))
            .unwrap()
            .unwrap()["dryRun"],
        true
    );
    assert!(!marker.exists());
}

// --------------------------------------------------------------------------------------- crypt, pwquality

#[test]
fn hashes_are_libxcrypts_default_and_verify() {
    if !crypt::available() {
        eprintln!("skipped: no libcrypt.so.1");
        return;
    }
    let a = crypt::hash("geheim wachtwoord").unwrap();
    let b = crypt::hash("geheim wachtwoord").unwrap();
    assert!(a.starts_with('$') && a.len() > 20, "{}", a.len());
    assert_ne!(a, b, "a fresh salt every time");
    assert!(crypt::verify("geheim wachtwoord", &a));
    assert!(!crypt::verify("ander wachtwoord", &a));
    if std::path::Path::new("/etc/debian_version").exists() {
        assert!(
            a.starts_with("$y$"),
            "yescrypt, Debian's and Ubuntu's default: {}",
            &a[..3]
        );
    }
}

#[test]
fn strength_levels_and_hints_follow_gnome() {
    assert_eq!(quality::level(-14, true), 0);
    assert_eq!(quality::level(-14, false), 1);
    assert_eq!(quality::level(30, false), 2);
    assert_eq!(quality::level(60, false), 3);
    assert_eq!(quality::level(80, false), 4);
    assert_eq!(quality::level(95, false), 5);
    assert_eq!(quality::hint_key(-14, false), "users.quality.short");
    assert_eq!(quality::hint_key(-20, true), "users.quality.empty");
    assert_eq!(quality::hint_key(-25, false), "users.quality.userName");
    assert_eq!(quality::hint_key(95, false), "users.quality.strong");
    assert_eq!(quality::hint_key(40, false), "users.quality.stronger");
    // every hint has its text in both languages
    for rc in [
        -30, -29, -28, -27, -26, -25, -22, -21, -19, -18, -17, -16, -15, -14, -13, -12, -11, -10,
        -9, -1, 10, 95,
    ] {
        for empty in [false, true] {
            let key = quality::hint_key(rc, empty);
            for lang in Lang::ALL {
                assert_ne!(crate::i18n::tr(lang, key), key, "{key}");
            }
        }
    }
    if let Some(rc) = quality::check("abc", None, Some("ik")) {
        assert!(rc < 0, "too short: {rc}");
        let strong = quality::check("Vier paarden; 17 rode kastanjes!", None, Some("ik")).unwrap();
        assert!(strong >= 0, "{strong}");
        let g = quality::generate().unwrap();
        assert!(g.len() >= 8 && quality::check(&g, None, None).unwrap() >= 0);
        let (_lang, ctx) = context(false);
        let users = Users::with(ctx, Buses::default()).unwrap();
        let weak = users
            .call("users.passwordQuality", &json!({"password": "abc"}))
            .unwrap()
            .unwrap();
        assert_eq!(
            (weak["level"].as_u64(), weak["ok"].as_bool()),
            (Some(1), Some(false))
        );
        assert_eq!(
            weak["hint"],
            "Password needs to be longer. Try to add more letters, numbers and punctuation."
        );
        assert!(
            users
                .call("users.passwordQuality", &json!({"generate": true}))
                .unwrap()
                .unwrap()["password"]
                .as_str()
                .is_some_and(|p| p.len() >= 8)
        );
    }
}

// -------------------------------------------------------------------------------------- names, faces, maps

#[test]
fn names_locales_and_user_objects_map_as_gnome_does() {
    for ok in ["anna", "a", "_svc", "bo-jansen", "x1_2", &"a".repeat(32)] {
        assert!(accounts::valid_user_name(ok), "{ok}");
    }
    for bad in [
        "",
        "Anna",
        "1anna",
        "-bo",
        "bo jansen",
        "bo.jansen",
        "bø",
        &"a".repeat(33),
    ] {
        assert!(!accounts::valid_user_name(bad), "{bad}");
    }
    assert_eq!(
        accounts::clean_real_name("  Anna de Vries "),
        Some("Anna de Vries".into())
    );
    for bad in ["", "   ", "a:b", "a,b", "a\nb", &"x".repeat(256)] {
        assert_eq!(accounts::clean_real_name(bad), None, "{bad:?}");
    }
    for ok in [
        "",
        "nl_NL.UTF-8",
        "en_GB.UTF-8",
        "nl",
        "fy_NL",
        "ca_ES.UTF-8@valencia",
        "ast_ES.UTF-8",
    ] {
        assert!(accounts::valid_locale(ok), "{ok}");
    }
    for bad in [
        "NL",
        "nl_nl",
        "nl_NL.",
        "nl NL",
        "nl_NL.UTF-8@",
        "../x",
        "nl_NL;x",
    ] {
        assert!(!accounts::valid_locale(bad), "{bad}");
    }
    assert_eq!(accounts::account_type_value("administrator"), Some(1));
    assert_eq!(accounts::account_type_value("root"), None);
    let props = json!({"Uid": 1001, "UserName": "anna", "RealName": "Anna", "AccountType": 1, "PasswordMode": 1,
        "AutomaticLogin": true, "IconFile": "/no/such/file", "SystemAccount": false, "Language": "nl_NL.UTF-8"});
    let u = accounts::user_json(&props, "/org/freedesktop/Accounts/User1001", 1000);
    assert_eq!(u["accountType"], "administrator");
    assert_eq!(u["passwordMode"], "setAtLogin");
    assert_eq!(u["self"], false);
    assert_eq!(u["iconFile"], "");
    assert_eq!(u["localAccount"], true);
    let me = accounts::passwd_self();
    assert_eq!(me["uid"], accounts::self_uid());
    assert!(!me["userName"].as_str().unwrap_or("").is_empty());
}

#[test]
fn the_face_gallery_lists_pictures_only() {
    let (lang, _ctx) = context(false);
    let dir = lang.root.join("faces");
    std::fs::create_dir_all(dir.join("legacy")).unwrap();
    for f in ["tree.jpg", "cat.png", "readme.txt", "book.JPG"] {
        std::fs::write(dir.join(f), b"x").unwrap();
    }
    let names: Vec<String> = faces(&dir)["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["book", "cat", "tree"], "no text file, no folder");
}

#[test]
fn every_users_method_is_known_and_unknown_ones_are_not_answered() {
    let (_lang, ctx) = context(false);
    let users = Users::with(ctx, Buses::default()).unwrap();
    // slice 2's users.setFormats is answered: a missing locale is bad_args before any bus is asked
    let formats = users
        .call("users.setFormats", &json!({}))
        .unwrap()
        .unwrap_err();
    assert_eq!(formats.code, "bad_args");
    assert!(users.call("users.nonsense", &json!({})).is_none());
    // without AccountsService (the sandbox has no system bus): the password database, marked unavailable
    let me = users.call("users.self", &json!({})).unwrap().unwrap();
    if me["available"] == false {
        assert_eq!(me["uid"], accounts::self_uid());
        let list = users.call("users.list", &json!({})).unwrap().unwrap_err();
        assert_eq!(list.key.as_deref(), Some("users.unavailable"));
    }
    let mut names: Vec<&str> = METHODS.iter().map(|(n, _)| *n).collect();
    let sorted = {
        let mut s = names.clone();
        s.sort();
        s
    };
    assert_eq!(names, sorted, "METHODS is sorted");
    names.dedup();
    assert_eq!(names.len(), METHODS.len());
}

// ------------------------------------------------------------------------- tests/dbus-fakes (system bus)

/// The calls a fake of tests/dbus-fakes/fakes/e/ recorded (`WIN11_FAKE_CALLS_<NAME>`): (method, args, interactive).
pub(crate) fn fake_calls(var: &str) -> Vec<(String, String, bool)> {
    let path = std::env::var(var).expect("run through tests/dbus-fakes/run.sh --batch e");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(|v| {
            (
                v["method"].as_str().unwrap_or("").to_owned(),
                v["args"].as_str().unwrap_or("").to_owned(),
                v["interactive"] == true,
            )
        })
        .collect()
}

#[test]
#[ignore = "tests/dbus-fakes/run.sh --batch e: AccountsService and fprintd fakes on a private system bus"]
fn fakes_users_on_the_system_bus() {
    let (_lang, ctx) = context(false);
    let users = Users::new(ctx).unwrap();
    let me = users.call("users.self", &json!({})).unwrap().unwrap();
    assert_eq!(
        (
            me["userName"].as_str(),
            me["available"].as_bool(),
            me["accountType"].as_str()
        ),
        (Some("ik"), Some(true), Some("administrator"))
    );
    users
        .call("users.setRealName", &json!({"name": "Ik Anders"}))
        .unwrap()
        .unwrap();
    assert_eq!(
        users.call("users.self", &json!({})).unwrap().unwrap()["realName"],
        "Ik Anders"
    );
    let list = users.call("users.list", &json!({})).unwrap().unwrap();
    assert_eq!(list["users"].as_array().map(Vec::len), Some(2));
    let fp = users
        .call("users.fingerprint.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(
        (fp["available"].as_bool(), fp["device"]["stages"].as_i64()),
        (Some(true), Some(2))
    );
    users
        .call(
            "users.fingerprint.enroll",
            &json!({"finger": "right-thumb"}),
        )
        .unwrap()
        .unwrap();
    for _ in 0..300 {
        if fake_calls("WIN11_FAKE_CALLS_ACCOUNTS")
            .iter()
            .any(|c| c.0 == "Release")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let reads = [
        "FindUserById",
        "ListCachedUsers",
        "GetDefaultDevice",
        "ListEnrolledFingers",
    ];
    let writes: Vec<(String, String, bool)> = fake_calls("WIN11_FAKE_CALLS_ACCOUNTS")
        .into_iter()
        .filter(|c| !reads.contains(&c.0.as_str()))
        .collect();
    assert_eq!(
        writes,
        vec![
            ("SetRealName".to_owned(), "('Ik Anders',)".to_owned(), true),
            ("Claim".into(), "('',)".into(), true),
            ("EnrollStart".into(), "('right-thumb',)".into(), true),
            ("EnrollStop".into(), "()".into(), false),
            ("Release".into(), "()".into(), false),
        ]
    );
    users.shutdown();
}

// ------------------------------------------------------------------------------------------------- the watchers

/// A burst of daemon signals is one event, and the event comes without anybody iterating the default MainContext: the
/// watchers' threads run their own, and the host's main thread owns the default one (glib's `timeout_add_local*`
/// attached there panicked on a watcher thread and aborted the host; the GNOME harness found it).
#[test]
fn a_burst_of_signals_is_one_event_off_the_default_main_context() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pending = Arc::new(AtomicBool::new(false));
    let burst = |n: usize| {
        for _ in 0..n {
            let c = count.clone();
            debounce(&pending, Duration::from_millis(60), move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
        }
    };
    let worker = std::thread::spawn({
        let (count, pending) = (count.clone(), pending.clone());
        move || {
            // as on a watcher thread: a MainContext of its own, the default one untouched
            let own = glib::MainContext::new();
            own.with_thread_default(|| {
                for _ in 0..3 {
                    let c = count.clone();
                    debounce(&pending, Duration::from_millis(60), move || {
                        c.fetch_add(1, Ordering::SeqCst);
                    });
                }
            })
            .unwrap();
        }
    });
    worker.join().unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(count.load(Ordering::SeqCst), 1, "three signals, one event");
    burst(2);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "a later burst is the next event"
    );
}
