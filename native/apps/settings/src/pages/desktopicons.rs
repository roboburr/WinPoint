//! Instellingen › Persoonlijke instellingen › Bureaubladpictogrammen
//! (6ebf169f:vendor/ui/js/apps/settings/pages/desktopicons.js): show the desktop icons, which ones, their size and
//! layout.
use super::{Builder, Page, PageFuture};
use crate::common;
use kit::controls::{SettingsCard, SettingsGroup};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("desktopicons", page)];
pub const CSS: &str = ".st-di .w11-sc-icon { min-width: 24px; margin: 0 18px 0 2px; }\n";

/// A toggle card bound to `path` with a 24 px app icon (`.st-di-icon`), or a glyph when `app` is None.
fn card(p: &Page, glyph: &str, app: Option<&str>, title: &str, desc: &str, path: &str, name: &str) -> SettingsCard {
    let (card, sw) = common::toggle_card(p, glyph, title, desc, path, false);
    sw.switch().set_widget_name(name);
    if let Some(icon) = app {
        card.set_icon_widget(&kit::icons::app_icon(icon, 24));
        card.widget().add_css_class("st-di");
    }
    card
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let show = SettingsGroup::new("", "");
        show.add(
            card(
                &p,
                "desktop",
                None,
                t("Bureaubladpictogrammen weergeven", "Show desktop icons"),
                t("Bestanden en snelkoppelingen op je bureaublad laten zien", "Show files and shortcuts on your desktop"),
                "desktop.showIcons",
                "desktopicons-show",
            )
            .widget(),
        );
        let which = SettingsGroup::new(t("Welke pictogrammen staan op het bureaublad", "Which icons appear on the desktop"), "");
        for (app, title, desc, path, name) in [
            ("computer", t("Deze pc", "This PC"), t("Je schijven en USB-sticks", "Your drives and USB sticks"), "desktop.showThisPC", "desktopicons-thispc"),
            (
                "user-trash",
                t("Prullenbak", "Recycle Bin"),
                t("Verwijderde bestanden, die je nog kunt terugzetten", "Deleted files you can still restore"),
                "desktop.showTrash",
                "desktopicons-trash",
            ),
            (
                "user-home",
                t("Persoonlijke map", "Personal folder"),
                t("Je documenten, afbeeldingen en downloads", "Your documents, pictures and downloads"),
                "desktop.showHome",
                "desktopicons-home",
            ),
        ] {
            which.add(card(&p, "", Some(app), title, desc, path, name).widget());
        }
        let layout = SettingsGroup::new(t("Indeling", "Layout"), "");
        let (size_card, size_dd) = common::dropdown_card(
            &p,
            "grid",
            t("Pictogramgrootte", "Icon size"),
            "",
            "desktop.iconSize",
            vec![
                (json!("small"), t("Klein", "Small").into()),
                (json!("medium"), t("Normaal", "Medium").into()),
                (json!("large"), t("Groot", "Large").into()),
            ],
            Some(json!("medium")),
        );
        size_dd.widget().set_widget_name("desktopicons-size");
        // the bound drop-down at its CSS width: out of the card first, then back in a fixed-width slot
        size_card.clear_control();
        size_card.set_control(&common::sized(size_dd.widget(), 160));
        layout.add(size_card.widget());
        layout.add(
            card(
                &p,
                "view",
                None,
                t("Pictogrammen uitlijnen op raster", "Align icons to grid"),
                t("Pictogrammen staan netjes in rijen en kolommen", "Icons line up neatly in rows and columns"),
                "desktop.alignToGrid",
                "desktopicons-grid",
            )
            .widget(),
        );
        layout.add(
            card(
                &p,
                "sort",
                None,
                t("Pictogrammen automatisch schikken", "Auto arrange icons"),
                t("Pictogrammen worden vanzelf van boven naar beneden gezet", "Icons are placed from top to bottom automatically"),
                "desktop.autoArrange",
                "desktopicons-arrange",
            )
            .widget(),
        );
        vec![show.widget().clone().upcast(), which.widget().clone().upcast(), layout.widget().clone().upcast()]
    })
}
