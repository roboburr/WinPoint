//! Instellingen › Persoonlijke instellingen (6ebf169f:vendor/ui/js/apps/settings/pages/personalization.js): the live
//! mini desktop and the theme picker, then the links to Achtergrond, Kleuren, Thema’s, Start, Taakbalk and
//! Bureaubladpictogrammen.
use super::themes::theme_grid;
use super::{Builder, Page, PageFuture};
use crate::common;
use crate::preview::{DesktopPreview, PreviewOpts};
use crate::widgets::{Clamp, Wrap};
use kit::controls as c;
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("personalization", page)];
pub const CSS: &str = ".st-pers-hero-label { margin-bottom: 12px; }\n";

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let prev = DesktopPreview::new(&PreviewOpts::default(), true);
        {
            let prev = prev.clone();
            p.on_settings(move |_, _| prev.update(&PreviewOpts::default()));
        }
        // `.st-pers-hero`: minmax(240px, 384px) for the preview, the rest (at least 240 px) for the themes; one
        // column when they do not fit side by side
        let hero = Wrap::new(32, 24);
        hero.add_css_class("st-pers-hero");
        let left = Clamp::new(&prev, 384);
        left.set_size_request(240, -1);
        left.set_hexpand(false);
        left.set_valign(gtk::Align::Center);
        hero.append(&left);
        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right.set_hexpand(true);
        right.set_size_request(240, -1);
        right.set_valign(gtk::Align::Center);
        right.append(&c::label(t("Selecteer een thema om toe te passen", "Select a theme to apply"), "st-pers-hero-label", true));
        right.append(&theme_grid(&p, true));
        hero.append(&right);

        let links = common::child_links(&p, "personalization");
        vec![hero.upcast(), links.widget().clone().upcast()]
    })
}
