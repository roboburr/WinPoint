// compile_glob and compile_class are derived from CPython Lib/fnmatch.py (Copyright (c) 2001 Python Software Foundation, PSF-2.0): see THIRD_PARTY_NOTICES.md and LICENSES/PSF-2.0.txt.
//! Bounded breadth-first Explorer search, preserving the original find contract.
use super::{ApiError, ApiResult, Context, directory, read};
use crate::i18n::Lang;
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    fs::{self, OpenOptions},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_DIRECTORIES: usize = 50_000;

pub(super) fn call(method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
    if method != "files.find" {
        return None;
    }
    Some((|| {
        let root = directory(&args["path"], ctx.lang())?;
        let options = Options::parse(args, ctx.lang())?;
        let mut factory = read::Factory::new(ctx);
        let deadline = Instant::now() + Duration::from_secs_f64(options.budget);
        Ok(search(
            &root,
            &options,
            &mut factory,
            deadline,
            MAX_DIRECTORIES,
        ))
    })())
}

struct Options {
    matcher: Matcher,
    limit: usize,
    show_hidden: bool,
    mime_prefix: Option<String>,
    budget: f64,
}
impl Options {
    fn parse(args: &Value, lang: Lang) -> Result<Self, ApiError> {
        // Python's `query or ""` deliberately accepts empty/falsy non-string values.
        let query = if truthy(&args["query"]) {
            args["query"].as_str().ok_or_else(|| {
                ApiError::tr("bad_args", lang, "files.badQuery", crate::i18n::NO_PARAMS)
            })?
        } else {
            ""
        };
        if query.chars().count() > 500 {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "files.badQuery",
                crate::i18n::NO_PARAMS,
            ));
        }
        let budget = if truthy(&args["budget"]) {
            py_float(&args["budget"]).ok_or_else(|| {
                ApiError::tr("bad_args", lang, "files.badBudget", crate::i18n::NO_PARAMS)
            })?
        } else {
            3.0
        };
        // min(10.0, NaN) returns its first operand in the source.
        let budget = if budget.is_nan() {
            10.0
        } else {
            budget.clamp(0.5, 10.0)
        };
        Ok(Self {
            matcher: Matcher::new(query),
            limit: py_int(&args["limit"]).unwrap_or(500).clamp(1, 5000) as usize,
            show_hidden: truthy(&args["showHidden"]),
            mime_prefix: args["mimePrefix"].as_str().map(str::to_owned),
            budget,
        })
    }
}

fn search(
    root: &Path,
    options: &Options,
    factory: &mut read::Factory<'_>,
    deadline: Instant,
    directory_limit: usize,
) -> Value {
    let mut queue = VecDeque::from([(root.to_path_buf(), true)]);
    let mut directories_seen = 0;
    let mut items = Vec::new();
    let mut truncated = false;
    while let Some((directory, is_root)) = queue.pop_front() {
        directories_seen += 1;
        if directories_seen > directory_limit || Instant::now() > deadline {
            truncated = true;
            break;
        }
        // Keep a stable directory handle while collecting entries. Descendant directories
        // cannot turn into a followed symlink between queueing and opening them. An explicit
        // symlink root is allowed, as it is by the original _dir/scandir contract.
        let flags =
            libc::O_DIRECTORY | libc::O_CLOEXEC | if is_root { 0 } else { libc::O_NOFOLLOW };
        let Ok(handle) = OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(&directory)
        else {
            continue;
        };
        let anchored = PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()));
        let Ok(entries) = fs::read_dir(&anchored)
            .and_then(|entries| entries.collect::<std::io::Result<Vec<_>>>())
        else {
            continue;
        };
        let hidden = if options.show_hidden {
            HashSet::new()
        } else {
            read::hidden_names(&anchored)
        };
        // Keep scandir/getdents order; names must not be sorted before BFS or result limiting.
        for entry in entries {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !options.show_hidden && (name.starts_with('.') || hidden.contains(name)) {
                continue;
            }
            let path = directory.join(name);
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                queue.push_back((path.clone(), false));
            }
            if !options.matcher.matches(name) {
                continue;
            }
            let Ok(item) = factory.make(&path, &hidden) else {
                continue;
            };
            if options.mime_prefix.as_deref().is_some_and(|prefix| {
                !prefix.is_empty() && !item["mime"].as_str().unwrap_or("").starts_with(prefix)
            }) {
                continue;
            }
            items.push(item);
            // Reaching the limit is truncated even when it was the last entry in the tree.
            if items.len() >= options.limit {
                truncated = true;
                break;
            }
        }
        if items.len() >= options.limit {
            break;
        }
    }
    json!({"items":items,"truncated":truncated})
}

enum Matcher {
    Terms(Vec<String>),
    Glob(Vec<Token>),
}
impl Matcher {
    fn new(query: &str) -> Self {
        let query = glib::casefold(query.trim_matches(py_space)).to_string();
        if query.contains(['*', '?', '[']) {
            let pattern = if query.contains(['*', '?']) {
                query
            } else {
                format!("*{query}*")
            };
            Self::Glob(compile_glob(&pattern))
        } else {
            Self::Terms(
                query
                    .split(py_space)
                    .filter(|term| !term.is_empty())
                    .map(str::to_owned)
                    .collect(),
            )
        }
    }
    fn matches(&self, name: &str) -> bool {
        let folded = glib::casefold(name);
        match self {
            Self::Terms(terms) => terms.iter().all(|term| folded.contains(term.as_str())),
            Self::Glob(pattern) => glob_matches(pattern, &folded),
        }
    }
}

enum Token {
    Star,
    Any,
    Literal(char),
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}
impl Token {
    fn accepts(&self, value: char) -> bool {
        match self {
            Self::Any => true,
            Self::Literal(character) => value == *character,
            Self::Class { negated, ranges } => {
                *negated
                    != ranges
                        .iter()
                        .any(|(start, end)| *start <= value && value <= *end)
            }
            Self::Star => false,
        }
    }
}

fn compile_glob(pattern: &str) -> Vec<Token> {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let mut result = Vec::new();
    let mut index = 0;
    while index < pattern.len() {
        let character = pattern[index];
        index += 1;
        match character {
            '*' => {
                if !matches!(result.last(), Some(Token::Star)) {
                    result.push(Token::Star);
                }
            }
            '?' => result.push(Token::Any),
            '[' => {
                let mut end = index;
                if pattern.get(end) == Some(&'!') {
                    end += 1;
                }
                if pattern.get(end) == Some(&']') {
                    end += 1;
                }
                while end < pattern.len() && pattern[end] != ']' {
                    end += 1;
                }
                if end == pattern.len() {
                    result.push(Token::Literal('['));
                } else {
                    result.push(compile_class(&pattern[index..end]));
                    index = end + 1;
                }
            }
            character => result.push(Token::Literal(character)),
        }
    }
    result
}

fn compile_class(characters: &[char]) -> Token {
    // Python fnmatch removes descending ranges, retaining the remaining literal
    // pieces. Hyphens within a chunk are literals; only joins between chunks are ranges.
    let mut chunks = Vec::<Vec<char>>::new();
    let mut start = 0;
    if characters.contains(&'-') {
        let mut index = if characters.first() == Some(&'!') {
            2
        } else {
            1
        };
        loop {
            let Some(found) = (index..characters.len()).find(|index| characters[*index] == '-')
            else {
                break;
            };
            chunks.push(characters[start..found].to_vec());
            start = found + 1;
            index = found + 3;
        }
        if start < characters.len() {
            chunks.push(characters[start..].to_vec());
        } else if let Some(chunk) = chunks.last_mut() {
            chunk.push('-');
        }
        for index in (1..chunks.len()).rev() {
            if chunks[index - 1].last() > chunks[index].first() {
                chunks[index - 1].pop();
                let remainder = chunks.remove(index);
                chunks[index - 1].extend(remainder.into_iter().skip(1));
            }
        }
    } else {
        chunks.push(characters.to_vec());
    }
    let negated = chunks.first().and_then(|chunk| chunk.first()) == Some(&'!');
    if negated {
        chunks[0].remove(0);
    }
    let mut ranges = Vec::new();
    for (index, chunk) in chunks.iter().enumerate() {
        for (offset, character) in chunk.iter().enumerate() {
            if index > 0 && offset == 0 {
                continue;
            }
            if offset + 1 == chunk.len() && index + 1 < chunks.len() {
                if let Some(next) = chunks[index + 1].first() {
                    ranges.push((*character, *next));
                }
            } else {
                ranges.push((*character, *character));
            }
        }
    }
    Token::Class { negated, ranges }
}

fn glob_matches(pattern: &[Token], value: &str) -> bool {
    let value = value.chars().collect::<Vec<_>>();
    let (mut token, mut character) = (0, 0);
    let mut last_star = None;
    let mut retry = 0;
    while character < value.len() {
        if pattern
            .get(token)
            .is_some_and(|token| token.accepts(value[character]))
        {
            token += 1;
            character += 1;
        } else if matches!(pattern.get(token), Some(Token::Star)) {
            last_star = Some(token);
            token += 1;
            retry = character;
        } else if let Some(star) = last_star {
            retry += 1;
            character = retry;
            token = star + 1;
        } else {
            return false;
        }
    }
    while matches!(pattern.get(token), Some(Token::Star)) {
        token += 1;
    }
    token == pattern.len()
}

fn py_space(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}
fn normalized_number(value: &str) -> Option<String> {
    let characters = value.trim_matches(py_space).chars().collect::<Vec<_>>();
    let mut output = String::new();
    for (index, character) in characters.iter().enumerate() {
        let digit = unsafe { glib::ffi::g_unichar_digit_value(*character as u32) };
        if *character == '_' {
            if index == 0
                || index + 1 == characters.len()
                || unsafe { glib::ffi::g_unichar_digit_value(characters[index - 1] as u32) } < 0
                || unsafe { glib::ffi::g_unichar_digit_value(characters[index + 1] as u32) } < 0
            {
                return None;
            }
        } else if digit >= 0 {
            output.push(char::from(b'0' + digit as u8));
        } else {
            output.push(*character);
        }
    }
    Some(output)
}
fn py_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Number(value) => value
            .as_i64()
            .or_else(|| {
                value
                    .as_u64()
                    .map(|value| value.min(i64::MAX as u64) as i64)
            })
            .or_else(|| {
                value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .map(|value| value as i64)
            }),
        Value::String(value) => {
            let value = normalized_number(value)?;
            let (negative, digits) = match value.as_bytes().first() {
                Some(b'-') => (true, &value[1..]),
                Some(b'+') => (false, &value[1..]),
                _ => (false, value.as_str()),
            };
            if digits.is_empty() || !digits.bytes().all(|value| value.is_ascii_digit()) {
                return None;
            }
            let number = digits.bytes().fold(0_i64, |number, value| {
                number
                    .saturating_mul(10)
                    .saturating_add(i64::from(value - b'0'))
            });
            Some(if negative { -number } else { number })
        }
        _ => None,
    }
}
fn py_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        Value::Number(value) => value.as_f64(),
        Value::String(value) => normalized_number(value)?.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::{ffi::OsStringExt, fs::symlink},
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "win11-find-{}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn context(&self) -> Context {
            Context::new(self.0.clone(), self.0.join("Desktop"), "en".into(), true)
        }
        fn find(&self, mut args: Value) -> Value {
            args["path"] = json!(self.0);
            call("files.find", &args, &self.context()).unwrap().unwrap()
        }
        fn write(&self, name: &str) {
            fs::write(self.0.join(name), "test file\n").unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn names(result: &Value) -> Vec<&str> {
        result["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn unicode_casefold_terms_and_python_whitespace_match_original() {
        for (query, name, expected) in [
            ("STRASSE", "Straße.txt", true),
            ("ος", "ΟΣ.txt", true),
            ("ss file", "File Straße.txt", true),
            ("file missing", "File Straße.txt", false),
            ("\u{1c}STRASSE\u{1f}", "Straße.txt", true),
            ("é", "e\u{301}.txt", false),
            ("İ", "i\u{307}.txt", true),
            (" ", "anything", true),
        ] {
            assert_eq!(
                Matcher::new(query).matches(name),
                expected,
                "{query:?} {name:?}"
            );
        }
    }

    #[test]
    fn python_globs_classes_invalid_ranges_and_literal_brackets() {
        for (pattern, name, expected) in [
            ("*.TXT", "résumé.txt", true),
            ("file?.txt", "file٢.txt", true),
            ("[a-c]", "prefixBpostfix", true),
            ("[!a-c]", "b", false),
            ("[z-a]*", "z", false),
            ("[!z-a]*", "anything", true),
            ("[a--b]", "b", true),
            ("[a--b]", "a", false),
            ("[]]", "literal]bracket", true),
            ("[!]]*", "]", false),
            ("[[]", "literal[bracket", true),
            ("[", "literal[bracket", true),
            ("[!]", "literal[!]bracket", true),
            ("[^a]", "^", true),
            ("[&~|]", "~", true),
            ("a\\b", "a\\b", true),
            ("a**?c", "abbbc", true),
            ("a*?c", "ac", false),
        ] {
            assert_eq!(
                Matcher::new(pattern).matches(name),
                expected,
                "{pattern:?} {name:?}"
            );
        }
    }

    #[test]
    fn breadth_first_order_and_hidden_subtree_filtering_are_preserved() {
        let fixture = Fixture::new();
        for directory in ["first", "second", ".dot-directory", "hidden-directory"] {
            fs::create_dir(fixture.0.join(directory)).unwrap();
            fixture.write(&format!("{directory}/needle-child.txt"));
        }
        fixture.write("needle-root.txt");
        fixture.write(".needle-dot");
        fixture.write("needle-hidden.txt");
        fixture.write("needle-backup~");
        fs::write(
            fixture.0.join(".hidden"),
            "needle-hidden.txt\nhidden-directory\n",
        )
        .unwrap();
        let result = fixture.find(json!({"query":"needle"}));
        let found = names(&result);
        let root_order = fs::read_dir(&fixture.0)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().into_string().unwrap())
            .filter(|name| name == "needle-root.txt" || name == "needle-backup~")
            .collect::<Vec<_>>();
        assert_eq!(found[..2], root_order);
        assert_eq!(
            found
                .iter()
                .filter(|name| **name == "needle-child.txt")
                .count(),
            2
        );
        assert_eq!(result["truncated"], false);
        assert!(
            result["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["name"] == "needle-backup~" && item["hidden"] == true)
        );
        let shown = fixture.find(json!({"query":"needle", "showHidden":true}));
        assert_eq!(
            names(&shown)
                .iter()
                .filter(|name| **name == "needle-child.txt")
                .count(),
            4
        );
        let hidden = shown["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "needle-hidden.txt")
            .unwrap();
        assert_eq!(hidden["hidden"], false);
    }

    #[test]
    fn links_are_returned_but_never_recursed_and_invalid_utf8_is_skipped() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("real-directory")).unwrap();
        fixture.write("real-directory/only-once.txt");
        symlink("real-directory", fixture.0.join("link-directory")).unwrap();
        symlink("missing", fixture.0.join("dangling")).unwrap();
        let invalid = fixture
            .0
            .join(std::ffi::OsString::from_vec(b"invalid-\xff".to_vec()));
        fs::create_dir(&invalid).unwrap();
        fs::write(invalid.join("must-not-be-visited"), "").unwrap();
        let result = fixture.find(json!({}));
        assert_eq!(
            names(&result)
                .iter()
                .filter(|name| **name == "only-once.txt")
                .count(),
            1
        );
        assert!(!names(&result).contains(&"must-not-be-visited"));
        let items = result["items"].as_array().unwrap();
        assert!(items.iter().any(|item| item["name"] == "link-directory"
            && item["isDir"] == true
            && item["isSymlink"] == true));
        assert!(
            items
                .iter()
                .any(|item| item["name"] == "dangling" && item["mime"] == "inode/symlink")
        );
        let alias = fixture.0.join("link-directory");
        let explicit = call("files.find", &json!({"path":alias}), &fixture.context())
            .unwrap()
            .unwrap();
        assert_eq!(names(&explicit), vec!["only-once.txt"]);
    }

    #[test]
    fn mime_filters_still_descend_into_nonmatching_directories() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("folder")).unwrap();
        fixture.write("folder/note.txt");
        fixture.write("image.png");
        let result = fixture.find(json!({"mimePrefix":"text/"}));
        assert_eq!(names(&result), vec!["note.txt"]);
        let ctx = fixture.context();
        let expected = ctx.item(&fixture.0.join("folder/note.txt")).unwrap();
        assert_eq!(result["items"][0], expected);
    }

    #[test]
    fn limits_deadlines_and_directory_budget_report_truncation() {
        let fixture = Fixture::new();
        fixture.write("one.txt");
        assert_eq!(fixture.find(json!({"limit":1}))["truncated"], true);
        fs::create_dir(fixture.0.join("child")).unwrap();
        fixture.write("child/two.txt");
        let ctx = fixture.context();
        let options = Options::parse(&json!({}), Lang::En).unwrap();
        let mut factory = read::Factory::new(&ctx);
        let timed = search(
            &fixture.0,
            &options,
            &mut factory,
            Instant::now() - Duration::from_millis(1),
            MAX_DIRECTORIES,
        );
        assert_eq!(timed, json!({"items":[],"truncated":true}));
        let bounded = search(
            &fixture.0,
            &options,
            &mut factory,
            Instant::now() + Duration::from_secs(5),
            1,
        );
        assert_eq!(bounded["truncated"], true);
        assert!(!names(&bounded).contains(&"two.txt"));
        for (value, expected) in [
            (json!(0), 1),
            (json!(false), 1),
            (json!("٢"), 2),
            (json!("5_001"), 5000),
            (json!("bad"), 500),
            (json!(3.8), 3),
        ] {
            assert_eq!(
                Options::parse(&json!({"limit":value}), Lang::En)
                    .unwrap()
                    .limit,
                expected
            );
        }
        assert_eq!(
            Options::parse(&json!({"budget":0}), Lang::En)
                .unwrap()
                .budget,
            3.0
        );
        assert_eq!(
            Options::parse(&json!({"budget":-1}), Lang::En)
                .unwrap()
                .budget,
            0.5
        );
        assert_eq!(
            Options::parse(&json!({"budget":"NaN"}), Lang::En)
                .unwrap()
                .budget,
            10.0
        );
    }

    #[test]
    fn fifo_metadata_cannot_block_and_query_errors_match_contract() {
        let fixture = Fixture::new();
        let path =
            std::ffi::CString::new(fixture.0.join("pipe").as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let directory = fixture.0.clone();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = Context::new(
                directory.clone(),
                directory.join("Desktop"),
                "en".into(),
                false,
            );
            send.send(
                call("files.find", &json!({"path":directory}), &ctx)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
        });
        let result = receive
            .recv_timeout(Duration::from_secs(5))
            .expect("FIFO search must not read the pipe");
        assert_eq!(result["items"][0]["mime"], "inode/fifo");
        for query in [json!(true), json!([1]), json!("é".repeat(501))] {
            assert_eq!(
                call(
                    "files.find",
                    &json!({"path":fixture.0,"query":query}),
                    &fixture.context()
                )
                .unwrap()
                .unwrap_err()
                .code,
                "bad_args"
            );
        }
        for query in [json!(false), json!(0), json!([])] {
            assert_eq!(
                fixture.find(json!({"query":query}))["items"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}
