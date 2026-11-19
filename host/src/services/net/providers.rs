//! The mobile operators' access points (`net.modem.apns`, notes/claude/settings-parity.md §5.4): the presets GNOME's
//! APN dialog offers, from mobile-broadband-provider-info's serviceproviders.xml (format 2.0), by the SIM's MCC and
//! MNC.
use serde_json::{Value, json};

/// The installed database.
pub(crate) const FILE: &str = "/usr/share/mobile-broadband-provider-info/serviceproviders.xml";

/// An MCC (3 digits) or MNC (2 or 3 digits).
pub(crate) fn code_ok(code: &str, mcc: bool) -> bool {
    let len_ok = if mcc {
        code.len() == 3
    } else {
        (2..=3).contains(&code.len())
    };
    len_ok && code.bytes().all(|b| b.is_ascii_digit())
}

fn child_text<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> &'a str {
    node.children()
        .find(|c| c.has_tag_name(name))
        .and_then(|c| c.text())
        .map(str::trim)
        .unwrap_or("")
}

/// The providers whose gsm network-id is (mcc, mnc), with their access points: {name, country, apns: [{apn, name,
/// usage, username, password, auth, dns}]}.
pub(crate) fn apns(xml: &str, mcc: &str, mnc: &str) -> Value {
    // the file names its DTD (an external one, which roxmltree never loads)
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        nodes_limit: 1_000_000,
    };
    let Ok(doc) = roxmltree::Document::parse_with_options(xml, options) else {
        return json!({ "providers": [] });
    };
    let mut providers = Vec::new();
    for country in doc
        .root_element()
        .children()
        .filter(|c| c.has_tag_name("country"))
    {
        let code = country.attribute("code").unwrap_or("");
        for provider in country.children().filter(|c| c.has_tag_name("provider")) {
            let Some(gsm) = provider.children().find(|c| c.has_tag_name("gsm")) else {
                continue;
            };
            let matches = gsm.children().any(|n| {
                n.has_tag_name("network-id")
                    && n.attribute("mcc") == Some(mcc)
                    && n.attribute("mnc") == Some(mnc)
            });
            if !matches {
                continue;
            }
            let list: Vec<Value> = gsm
                .children()
                .filter(|c| c.has_tag_name("apn"))
                .map(|apn| {
                    let usage: Vec<&str> = apn
                        .children()
                        .filter(|c| c.has_tag_name("usage"))
                        .filter_map(|c| c.attribute("type"))
                        .collect();
                    let dns: Vec<&str> = apn
                        .children()
                        .filter(|c| c.has_tag_name("dns"))
                        .filter_map(|c| c.text())
                        .map(str::trim)
                        .collect();
                    let auth = apn
                        .children()
                        .find(|c| c.has_tag_name("authentication"))
                        .and_then(|c| c.attribute("method"))
                        .unwrap_or("");
                    json!({
                        "apn": apn.attribute("value").unwrap_or(""),
                        "name": child_text(apn, "name"),
                        "usage": usage,
                        "username": child_text(apn, "username"),
                        "password": child_text(apn, "password"),
                        "auth": auth,
                        "dns": dns,
                    })
                })
                .collect();
            providers.push(
                json!({ "name": child_text(provider, "name"), "country": code, "apns": list }),
            );
        }
    }
    json!({ "providers": providers })
}
