//! Instellingen › Systeem › Klembord (last-webkit:vendor/ui/js/apps/settings/pages/clipboard.js): explains Win+V, opens
//! the history, clears it. Under GNOME clipboard.list is unsupported (D-G2): the page says so.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, SettingsCard, SettingsGroup};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("clipboard", clipboard)];
pub const CSS: &str = "";

fn count_text(n: i64) -> String {
    match n {
        0 => t("Nog niets gekopieerd", "Nothing copied yet").to_owned(),
        _ => common::plural(n, t("1 item in de geschiedenis", "1 item in history"), t("{n} items in de geschiedenis", "{n} items in history")),
    }
}

fn clipboard(p: Page) -> PageFuture {
    Box::pin(async move {
        let r = p.call("clipboard.list", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let supported = matches!(&r, Ok(v) if v["supported"] != false);
        let items = if supported { r.as_ref().ok().and_then(|v| v["items"].as_array()).map(|a| a.len()).unwrap_or(0) } else { 0 };

        let clear = c::button(t("Wissen", "Clear"), c::Variant::Standard);
        clear.set_sensitive(supported && items > 0);
        clear.set_widget_name("clipboard-clear");
        let pp = p.clone();
        clear.connect_clicked(move |_| {
            let pp = pp.clone();
            c::confirm(
                &pp.anchor(),
                t("Klembordgeschiedenis wissen?", "Clear clipboard history?"),
                t(
                    "Alle gekopieerde items worden uit de geschiedenis verwijderd. Vastgemaakte items blijven bewaard. Dit kan niet ongedaan worden gemaakt.",
                    "All copied items will be removed from the history. Pinned items are kept. This can’t be undone.",
                ),
                t("Wissen", "Clear"),
                true,
                move |ok| {
                    if !ok {
                        return;
                    }
                    let fut = pp.run("clipboard.clear", json!({}));
                    let p2 = pp.clone();
                    pp.spawn(async move {
                        if fut.await.is_some() {
                            common::toast_success(&p2.anchor(), t("Klembordgeschiedenis gewist", "Clipboard history cleared"), 2500);
                            p2.refresh();
                        }
                    });
                },
            );
        });
        {
            let pp = p.clone();
            p.on("clipboard.changed", move |_| pp.refresh());
        }

        let open = c::button(t("Openen", "Open"), c::Variant::Standard);
        open.set_sensitive(supported);
        open.set_widget_name("clipboard-open");
        let pp = p.clone();
        open.connect_clicked(move |_| {
            let fut = pp.run_with("shell.command", json!({ "command": "clipboard" }), None, true);
            pp.spawn(async move {
                let _ = fut.await;
            });
        });
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        controls.add_css_class("st-row");
        controls.append(&common::value_label(&count_text(items as i64)));
        controls.append(&open);

        let history = SettingsCard::new(
            "clipboard",
            t("Klembordgeschiedenis", "Clipboard history"),
            t(
                "Bewaar de laatste 25 dingen die je hebt gekopieerd, zodat je ze later opnieuw kunt plakken",
                "Keep the last 25 things you copied so you can paste them again later",
            ),
        );
        let how = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        how.add_css_class("st-howto");
        how.append(&c::label(t("Druk op", "Press"), "w11-sc-desc", false));
        how.append(&common::kbd(&["⊞ Win", "V"]));
        let rest = c::label(t("om je klembordgeschiedenis te bekijken en te plakken.", "to view and paste from your clipboard history."), "w11-sc-desc", true);
        rest.set_hexpand(true);
        how.append(&rest);
        common::card_extra(&history, &how);
        history.set_control(&controls);

        let clear_card = SettingsCard::new(
            "delete",
            t("Klembordgegevens wissen", "Clear clipboard data"),
            t("Alles uit de geschiedenis verwijderen, behalve vastgemaakte items", "Remove everything from the history except pinned items"),
        );
        clear_card.set_control(&clear);
        let tip = SettingsCard::new(
            "bulb",
            t("Tip", "Tip"),
            t("Kopiëren doe je met Ctrl+C, knippen met Ctrl+X en plakken met Ctrl+V.", "Copy with Ctrl+C, cut with Ctrl+X and paste with Ctrl+V."),
        );

        let mut out: Content = Vec::new();
        if !supported {
            out.push(
                common::notice(t(
                    "Klembordgeschiedenis is hier niet beschikbaar, omdat GNOME apps niet laat bijhouden wat je kopieert. Kopiëren en plakken werken gewoon.",
                    "Clipboard history isn’t available here, because GNOME doesn’t let apps keep track of what you copy. Copy and paste work as usual.",
                ))
                .widget()
                .clone()
                .upcast(),
            );
        }
        let g1 = SettingsGroup::new("", "");
        g1.add(history.widget());
        g1.add(clear_card.widget());
        let g2 = SettingsGroup::new("", "");
        g2.add(tip.widget());
        out.push(g1.widget().clone().upcast());
        out.push(g2.widget().clone().upcast());
        out
    })
}
