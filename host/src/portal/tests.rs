use super::*;
use glib::variant::{ObjectPath, ToVariant};
use std::{fs, os::unix::ffi::OsStringExt};
fn params(options: BTreeMap<String, glib::Variant>) -> glib::Variant {
    (
        ObjectPath::try_from("/org/freedesktop/portal/desktop/request/1_42/test").unwrap(),
        "org.example.Test",
        "",
        "Choose fixture",
        options,
    )
        .to_variant()
}
fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("win11-portal-model-{}", crate::events::nonce()));
    fs::create_dir(&root).unwrap();
    root
}
fn fields(v: &BTreeMap<String, glib::Variant>) -> Value {
    json!({"uris":v["uris"].get::<Vec<String>>().unwrap()})
}
#[test]
fn typed_options_map_to_existing_picker_without_losing_filters_or_choices() {
    let root = fixture();
    let filter: model::Filter = (
        "Images".into(),
        vec![(0, "*.[pP][nN][gG]".into()), (1, "image/jpeg".into())],
    );
    let choice: model::Choice = (
        "encoding".into(),
        "Encoding".into(),
        vec![
            ("utf8".into(), "UTF-8".into()),
            ("latin".into(), "Latin".into()),
        ],
        "latin".into(),
    );
    let mut folder = root.as_os_str().as_encoded_bytes().to_vec();
    folder.push(0);
    let req = model::Request::parse(
        "OpenFile",
        &params(BTreeMap::from([
            ("current_folder".into(), folder.to_variant()),
            (
                "filters".into(),
                vec![
                    ("All".to_owned(), vec![(0u32, "*".to_owned())]),
                    filter.clone(),
                ]
                .to_variant(),
            ),
            ("current_filter".into(), filter.to_variant()),
            ("choices".into(), vec![choice].to_variant()),
            ("multiple".into(), true.to_variant()),
            ("accept_label".into(), "_Choose __files".to_variant()),
        ])),
    )
    .unwrap();
    let ui = req.picker();
    assert_eq!(ui["directory"], json!(root));
    assert_eq!(ui["multiple"], true);
    assert_eq!(ui["currentFilter"], 1);
    assert_eq!(ui["filters"][1]["caseSensitive"], true);
    assert_eq!(
        ui["filters"][1]["conditions"][1],
        json!({"type":1,"value":"image/jpeg"})
    );
    assert_eq!(ui["choices"][0]["selected"], "latin");
    assert_eq!(ui["acceptLabel"], "Choose _files");
    let _ = fs::remove_dir_all(root);
}
#[test]
fn savefile_returns_one_encoded_uri_and_never_creates_or_truncates() {
    let root = fixture();
    let selected = root.join("My file.png");
    let req = model::Request::parse("SaveFile", &params(BTreeMap::new())).unwrap();
    let v = req
        .result(&json!({"path":selected,"filterIndex":0,"choices":[],"writable":true}))
        .unwrap()
        .unwrap();
    assert_eq!(
        fields(&v)["uris"],
        json!([gio::File::for_path(&selected).uri().to_string()])
    );
    assert!(!selected.exists());
    fs::write(&selected, b"preserve").unwrap();
    req.result(&json!({"path":selected})).unwrap();
    assert_eq!(fs::read(&selected).unwrap(), b"preserve");
    let _ = fs::remove_dir_all(root);
}
#[test]
fn open_multiple_selection_has_exact_typed_filter_choices_and_writable() {
    let root = fixture();
    let a = root.join("one.txt");
    let b = root.join("two.txt");
    fs::write(&a, "one").unwrap();
    fs::write(&b, "two").unwrap();
    let filter: model::Filter = ("Text".into(), vec![(0, "*.txt".into())]);
    let choices: Vec<model::Choice> = vec![("read".into(), "Read".into(), vec![], "false".into())];
    let req = model::Request::parse(
        "OpenFile",
        &params(BTreeMap::from([
            ("multiple".into(), true.to_variant()),
            ("filters".into(), vec![filter.clone()].to_variant()),
            ("choices".into(), choices.to_variant()),
        ])),
    )
    .unwrap();
    let v = req
        .result(
            &json!({"paths":[a,b],"filterIndex":0,"choices":[["read","true"]],"writable":false}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(v["uris"].type_().as_str(), "as");
    assert_eq!(v["current_filter"].type_().as_str(), "(sa(us))");
    assert_eq!(v["current_filter"].get::<model::Filter>(), Some(filter));
    assert_eq!(
        v["choices"].get::<Vec<(String, String)>>(),
        Some(vec![("read".into(), "true".into())])
    );
    assert_eq!(v["writable"].get::<bool>(), Some(false));
    let _ = fs::remove_dir_all(root);
}
#[test]
fn savefiles_uses_raw_basenames_unique_targets_and_input_order_without_writing() {
    let root = fixture();
    fs::write(root.join("report.txt"), "preserve").unwrap();
    let nonutf = vec![b'n', 255, b'.', b'b', b'i', b'n', 0];
    let names = vec![b"report.txt\0".to_vec(), b"report.txt\0".to_vec(), nonutf];
    let req = model::Request::parse(
        "SaveFiles",
        &params(BTreeMap::from([("files".into(), names.to_variant())])),
    )
    .unwrap();
    assert_eq!(req.picker()["folder"], true);
    let v = req.result(&json!({"path":root})).unwrap().unwrap();
    let uris = v["uris"].get::<Vec<String>>().unwrap();
    assert!(uris[0].ends_with("report%20(1).txt"));
    assert!(uris[1].ends_with("report%20(2).txt"));
    assert!(uris[2].contains("%FF"));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(fs::read(root.join("report.txt")).unwrap(), b"preserve");
    let _ = fs::remove_dir_all(root);
}
#[test]
fn cancellation_invalid_types_paths_and_unoffered_choices_are_rejected() {
    let req = model::Request::parse("OpenFile", &params(BTreeMap::new())).unwrap();
    assert!(
        req.result(&json!({"path":null,"paths":[],"cancelled":true}))
            .unwrap()
            .is_none()
    );
    assert!(req.result(&json!({"path":"relative"})).is_err());
    assert!(req.result(&json!({"paths":["/tmp","/tmp"]})).is_err());
    for options in [
        BTreeMap::from([("multiple".into(), "true".to_variant())]),
        BTreeMap::from([(
            "current_folder".into(),
            vec![b'/', b't', b'm', b'p'].to_variant(),
        )]),
        BTreeMap::from([(
            "filters".into(),
            vec![("Bad".to_owned(), vec![(2u32, "*".to_owned())])].to_variant(),
        )]),
    ] {
        assert!(model::Request::parse("OpenFile", &params(options)).is_err());
    }
    for name in [
        b"../escape\0".to_vec(),
        b"/absolute\0".to_vec(),
        b".\0".to_vec(),
        b"bad\0extra\0".to_vec(),
    ] {
        assert!(
            model::Request::parse(
                "SaveFiles",
                &params(BTreeMap::from([("files".into(), vec![name].to_variant())]))
            )
            .is_err()
        );
    }
    let root = fixture();
    let path = root.join("one");
    fs::write(&path, "x").unwrap();
    assert!(
        req.result(&json!({"path":path,"choices":[["unoffered","value"]]}))
            .is_err()
    );
    let _ = fs::remove_dir_all(root);
}
#[test]
fn non_utf8_suggested_folder_falls_back_to_encodable_ancestor() {
    let root = fixture();
    let p = root.join(std::ffi::OsString::from_vec(vec![255]));
    fs::create_dir(&p).unwrap();
    let mut raw = p.as_os_str().as_encoded_bytes().to_vec();
    raw.push(0);
    let req = model::Request::parse(
        "SaveFile",
        &params(BTreeMap::from([(
            "current_folder".into(),
            raw.to_variant(),
        )])),
    )
    .unwrap();
    assert_eq!(req.picker()["directory"], json!(root));
    let _ = fs::remove_dir_all(root);
}

// Run in a separate process with WIN11_PORTAL_ENV=1, GTK_USE_PORTAL=0,
// and optional WIN11_PORTAL_PARENT_GTK_USE_PORTAL. No environment is mutated.
#[test]
#[ignore = "requires an isolated process with the portal's captured parent environment"]
fn portal_local_gtk_override_does_not_escape_command_or_gio_launch_context() {
    use std::{os::unix::ffi::OsStrExt, process::Command};
    assert_eq!(std::env::var("WIN11_PORTAL_ENV").as_deref(), Ok("1"));
    assert_eq!(std::env::var("GTK_USE_PORTAL").as_deref(), Ok("0"));
    let original = std::env::var_os("WIN11_PORTAL_PARENT_GTK_USE_PORTAL");
    let expected = original.as_ref().map(|v| v.as_bytes());
    let check = |rows: Vec<Vec<u8>>| {
        let value = rows
            .iter()
            .find_map(|row| row.strip_prefix(b"GTK_USE_PORTAL="));
        assert_eq!(value, expected);
        assert!(!rows.iter().any(|row| row.starts_with(b"WIN11_PORTAL_ENV=")));
        assert!(
            !rows
                .iter()
                .any(|row| row.starts_with(b"WIN11_PORTAL_PARENT_GTK_USE_PORTAL="))
        );
    };
    let context = crate::launch_context::new(None);
    check(
        context
            .environment()
            .iter()
            .map(|v| v.as_bytes().to_vec())
            .collect(),
    );
    let mut command = Command::new("/usr/bin/env");
    command.arg("-0");
    crate::launch_context::restore_portal_child_environment(&mut command);
    let output = command.output().unwrap();
    assert!(output.status.success());
    check(
        output
            .stdout
            .split(|b| *b == 0)
            .filter(|v| !v.is_empty())
            .map(Vec::from)
            .collect(),
    );
}
