//! JSON ↔ GVariant for a GSettings key, by the key's own type (notes/claude/settings-parity.md §6.1): `b i u q d s
//! as ai ad a(ss) (dd)`, an enum as its nick (a string), flags as an array of nicks. Every other type is refused.
use glib::variant::ToVariant;
use serde_json::{Value, json};

/// What a key holds, as the API sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Bool,
    Int,
    Uint,
    Uint16,
    Double,
    Str,
    StrList,
    IntList,
    DoubleList,
    PairList,
    Point,
    Enum(Vec<String>),
    Flags(Vec<String>),
    /// A type the API does not map (its GVariant type string).
    Unsupported(String),
}

/// The kind of a GVariant type string (no enum or flags: those come from the key's range, [`kind`]).
pub(crate) fn kind_of(type_string: &str) -> Kind {
    match type_string {
        "b" => Kind::Bool,
        "i" => Kind::Int,
        "u" => Kind::Uint,
        "q" => Kind::Uint16,
        "d" => Kind::Double,
        "s" => Kind::Str,
        "as" => Kind::StrList,
        "ai" => Kind::IntList,
        "ad" => Kind::DoubleList,
        "a(ss)" => Kind::PairList,
        "(dd)" => Kind::Point,
        other => Kind::Unsupported(other.to_owned()),
    }
}

fn strings(v: &glib::Variant) -> Vec<String> {
    (0..v.n_children())
        .filter_map(|i| v.child_value(i).str().map(str::to_owned))
        .collect()
}

/// The kind of a schema key: its range says enum or flags, otherwise its value type.
pub(crate) fn kind(key: &gio::SettingsSchemaKey) -> Kind {
    let range = key.range();
    let (name, data) = (range.child_value(0), range.child_value(1).as_variant());
    match (name.str(), data) {
        (Some("enum"), Some(data)) => Kind::Enum(strings(&data)),
        (Some("flags"), Some(data)) => Kind::Flags(strings(&data)),
        _ => kind_of(key.value_type().as_str()),
    }
}

/// The type name the API reports: the GVariant type string, or "enum" / "flags".
pub(crate) fn type_name(kind: &Kind) -> String {
    match kind {
        Kind::Bool => "b".into(),
        Kind::Int => "i".into(),
        Kind::Uint => "u".into(),
        Kind::Uint16 => "q".into(),
        Kind::Double => "d".into(),
        Kind::Str => "s".into(),
        Kind::StrList => "as".into(),
        Kind::IntList => "ai".into(),
        Kind::DoubleList => "ad".into(),
        Kind::PairList => "a(ss)".into(),
        Kind::Point => "(dd)".into(),
        Kind::Enum(_) => "enum".into(),
        Kind::Flags(_) => "flags".into(),
        Kind::Unsupported(t) => t.clone(),
    }
}

/// The API's `range`: `{choices}` for enum and flags, `{min, max}` for a numeric range, else null.
pub(crate) fn range_json(key: &gio::SettingsSchemaKey, kind: &Kind) -> Value {
    match kind {
        Kind::Enum(choices) | Kind::Flags(choices) => json!({ "choices": choices }),
        _ => {
            let range = key.range();
            if range.child_value(0).str() != Some("range") {
                return Value::Null;
            }
            let Some(bounds) = range.child_value(1).as_variant() else {
                return Value::Null;
            };
            if bounds.n_children() != 2 {
                return Value::Null;
            }
            json!({
                "min": to_json(kind, &bounds.child_value(0)),
                "max": to_json(kind, &bounds.child_value(1)),
            })
        }
    }
}

fn int<T: TryFrom<i64>>(value: &Value) -> Option<T> {
    value.as_i64().and_then(|n| T::try_from(n).ok())
}

fn uint<T: TryFrom<u64>>(value: &Value) -> Option<T> {
    value.as_u64().and_then(|n| T::try_from(n).ok())
}

fn double(value: &Value) -> Option<f64> {
    value.as_f64().filter(|f| f.is_finite())
}

fn list<T>(value: &Value, each: impl Fn(&Value) -> Option<T>) -> Option<Vec<T>> {
    let array = value.as_array().filter(|a| a.len() <= 1024)?;
    array.iter().map(each).collect()
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| s.len() <= 64 * 1024 && !s.contains('\0'))
        .map(str::to_owned)
}

/// The GVariant of a JSON value for a key of this kind; None when the JSON does not fit the type (the range is
/// checked by the caller with `range_check`).
pub(crate) fn to_variant(kind: &Kind, value: &Value) -> Option<glib::Variant> {
    Some(match kind {
        Kind::Bool => value.as_bool()?.to_variant(),
        Kind::Int => int::<i32>(value)?.to_variant(),
        Kind::Uint => uint::<u32>(value)?.to_variant(),
        Kind::Uint16 => uint::<u16>(value)?.to_variant(),
        Kind::Double => double(value)?.to_variant(),
        Kind::Str | Kind::Enum(_) => text(value)?.to_variant(),
        Kind::StrList | Kind::Flags(_) => list(value, text)?.to_variant(),
        Kind::IntList => list(value, int::<i32>)?.to_variant(),
        Kind::DoubleList => list(value, double)?.to_variant(),
        Kind::PairList => list(value, |pair| {
            let pair = pair.as_array().filter(|p| p.len() == 2)?;
            Some((text(&pair[0])?, text(&pair[1])?))
        })?
        .to_variant(),
        Kind::Point => {
            let point = value.as_array().filter(|p| p.len() == 2)?;
            (double(&point[0])?, double(&point[1])?).to_variant()
        }
        Kind::Unsupported(_) => return None,
    })
}

/// The JSON of a key's GVariant (null for a type the API does not map).
pub(crate) fn to_json(kind: &Kind, v: &glib::Variant) -> Value {
    match kind {
        Kind::Bool => json!(v.get::<bool>()),
        Kind::Int => json!(v.get::<i32>()),
        Kind::Uint => json!(v.get::<u32>()),
        Kind::Uint16 => json!(v.get::<u16>()),
        Kind::Double => v.get::<f64>().map_or(Value::Null, |f| json!(f)),
        Kind::Str | Kind::Enum(_) => json!(v.str()),
        Kind::StrList | Kind::Flags(_) => json!(v.get::<Vec<String>>()),
        Kind::IntList => json!(v.get::<Vec<i32>>()),
        Kind::DoubleList => json!(v.get::<Vec<f64>>()),
        Kind::PairList => v
            .get::<Vec<(String, String)>>()
            .map_or(Value::Null, |pairs| {
                Value::Array(pairs.into_iter().map(|(a, b)| json!([a, b])).collect())
            }),
        Kind::Point => v
            .get::<(f64, f64)>()
            .map_or(Value::Null, |(a, b)| json!([a, b])),
        Kind::Unsupported(_) => Value::Null,
    }
}
