//! Dutch and English: `t(nl, en)` by settings.language ("nl" unless "en"), live. Widgets that show a fixed text
//! can be bound so they follow a language switch by themselves (`label`, `bind`), the way components.js
//! relabels its own texts; anything else listens with `on_change` (or `Ctx::on_settings`).
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    static FORCED: Cell<Option<bool>> = const { Cell::new(None) };
    static EN: Cell<bool> = const { Cell::new(false) };
    static LISTENERS: RefCell<Vec<Rc<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
    static BOUND: RefCell<Vec<(glib::WeakRef<gtk::Widget>, Rc<dyn Fn(&gtk::Widget)>)>> = const { RefCell::new(Vec::new()) };
}

/// The UI language ("nl" or "en") of a settings.json `language` value: contracts C1, exactly "en" is English,
/// "system" the session's own language ([`system_language`], settings parity amendment 18) and anything else Dutch.
/// Every kit reader of settings.language goes through this one place.
pub fn resolve(language: &serde_json::Value) -> &'static str {
    match language.as_str() {
        Some("en") => "en",
        Some("system") => system_language(|k| std::env::var(k).ok()),
        _ => "nl",
    }
}

/// The session's messages language as gettext picks it, from `var` (the environment): LC_ALL, LC_MESSAGES or LANG,
/// and LANGUAGE's first entry before them unless that locale is C/POSIX. "nl" when that is Dutch, else "en" (no
/// locale at all is C, whose messages are English). The host reads it the same way (host/src/i18n.rs system_code).
pub fn system_language(var: impl Fn(&str) -> Option<String>) -> &'static str {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| var(k).filter(|v| !v.is_empty())).unwrap_or_default();
    let c = locale.is_empty() || locale == "C" || locale == "POSIX" || locale.starts_with("C.");
    let first = if c { None } else { var("LANGUAGE").and_then(|l| l.split(':').find(|s| !s.is_empty()).map(str::to_owned)) };
    let pick = first.unwrap_or(locale);
    if pick.split(['_', '.', '@']).next() == Some("nl") { "nl" } else { "en" }
}

/// "nl" or "en".
pub fn lang() -> &'static str {
    if en() { "en" } else { "nl" }
}

/// Whether the UI is in English.
pub fn en() -> bool {
    EN.with(|e| e.get())
}

/// The Dutch or the English text, by the current language.
pub fn t<'a>(nl: &'a str, en_text: &'a str) -> &'a str {
    if en() { en_text } else { nl }
}

/// The Intl locale the HTML apps used for the language: "nl-NL" or "en-GB".
pub fn locale() -> &'static str {
    if en() { "en-GB" } else { "nl-NL" }
}

/// Set the language (the kit does this from settings.language); listeners run when it changed.
pub(crate) fn set_lang(lang: &str) {
    let en_now = forced_en().unwrap_or(lang == "en");
    if EN.with(|e| e.replace(en_now)) == en_now {
        return;
    }
    BOUND.with(|b| b.borrow_mut().retain(|(w, _)| w.upgrade().is_some()));
    let bound: Vec<(gtk::Widget, Rc<dyn Fn(&gtk::Widget)>)> =
        BOUND.with(|b| b.borrow().iter().filter_map(|(w, f)| w.upgrade().map(|w| (w, f.clone()))).collect());
    for (w, f) in bound {
        f(&w);
    }
    let listeners: Vec<Rc<dyn Fn()>> = LISTENERS.with(|l| l.borrow().clone());
    for f in listeners {
        f();
    }
}

/// Force the language ("nl" / "en") whatever settings.language says, like theme.js `?lang=` (screenshots);
/// None follows the settings again. WIN11_LANG=nl|en does this at startup.
pub fn set_forced_lang(lang: Option<&str>) {
    FORCED.with(|f| f.set(lang.map(|l| l == "en")));
    let current = crate::ctx::try_ctx().map(|c| resolve(&c.setting("language")) == "en").unwrap_or(en());
    set_lang(if current { "en" } else { "nl" });
    if let Some(ctx) = crate::ctx::try_ctx() {
        ctx.apply_look();
    }
}

fn forced_en() -> Option<bool> {
    FORCED.with(|f| f.get()).or_else(|| match std::env::var("WIN11_LANG").as_deref() {
        Ok("en") => Some(true),
        Ok("nl") => Some(false),
        _ => None,
    })
}

/// Run `f` after every language switch (for the life of the process).
pub fn on_change(f: impl Fn() + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Rc::new(f)));
}

/// Run `f(widget)` now and after every language switch while the widget lives (held weakly).
pub fn bind<W: IsA<gtk::Widget>>(widget: &W, f: impl Fn(&W) + 'static) {
    f(widget);
    let w: gtk::Widget = widget.clone().upcast();
    let f: Rc<dyn Fn(&gtk::Widget)> = Rc::new(move |w: &gtk::Widget| {
        if let Some(w) = w.downcast_ref::<W>() {
            f(w);
        }
    });
    BOUND.with(|b| {
        let mut b = b.borrow_mut();
        if b.len() > 512 && b.len().is_power_of_two() {
            b.retain(|(w, _)| w.upgrade().is_some());
        }
        b.push((w.downgrade(), f));
    });
}

/// A label that shows `t(nl, en)` and follows language switches.
pub fn label(nl: &'static str, en_text: &'static str) -> gtk::Label {
    let l = gtk::Label::new(None);
    bind(&l, move |l| l.set_label(t(nl, en_text)));
    l
}

/// Keep a widget's tooltip at `t(nl, en)`.
pub fn tooltip<W: IsA<gtk::Widget>>(widget: &W, nl: &'static str, en_text: &'static str) {
    bind(widget, move |w| w.set_tooltip_text(Some(t(nl, en_text))));
}

/// "A, B en C" / "A, B and C" (i18n.js list).
pub fn list(items: &[&str]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].to_owned(),
        n => format!("{} {} {}", items[..n - 1].join(", "), t("en", "and"), items[n - 1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_en_is_english() {
        // contracts C1; the one place batch E extends with "system" (settings parity, amendment 18)
        assert_eq!(resolve(&serde_json::json!("en")), "en");
        for other in [serde_json::json!("nl"), serde_json::json!("EN"), serde_json::json!("System"), serde_json::json!(null), serde_json::json!(1)] {
            assert_eq!(resolve(&other), "nl", "{other}");
        }
        assert_eq!(resolve(&serde_json::json!("system")), system_language(|k| std::env::var(k).ok()));
    }

    #[test]
    fn system_is_the_sessions_language() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned());
        assert_eq!(system_language(env(&[("LANG", "nl_NL.UTF-8")])), "nl");
        assert_eq!(system_language(env(&[("LANG", "nl_BE.UTF-8")])), "nl");
        assert_eq!(system_language(env(&[("LANG", "en_GB.UTF-8")])), "en");
        assert_eq!(system_language(env(&[("LANG", "de_DE.UTF-8")])), "en", "WinPoint has nl and en");
        assert_eq!(system_language(env(&[])), "en");
        assert_eq!(system_language(env(&[("LANG", "en_US.UTF-8"), ("LC_MESSAGES", "nl_NL.UTF-8")])), "nl");
        assert_eq!(system_language(env(&[("LC_MESSAGES", "nl_NL.UTF-8"), ("LC_ALL", "en_US.UTF-8")])), "en");
        assert_eq!(system_language(env(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "nl:en")])), "nl");
        assert_eq!(system_language(env(&[("LANG", "C"), ("LANGUAGE", "nl")])), "en", "gettext ignores LANGUAGE under C");
    }
}
