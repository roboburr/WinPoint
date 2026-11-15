//! Instellingen › Apps (6ebf169f:vendor/ui/js/apps/settings/pages/apps.js): the overview with links to
//! Geïnstalleerde apps, Standaard-apps and Opstarten. Its CSS has the app lists' toolbar and rows (settings.css "Apps
//! lists"), shared by installed.rs and startup.rs.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::SettingsGroup;
use kit::prelude::*;

pub const PAGES: &[(&str, Builder)] = &[("apps", apps)];

/// settings.css "Apps lists": the toolbar (search box, "Sorteren op" fields), the count line and the 60 px
/// rows with a 32 px app icon.
pub const CSS: &str = r#"
.st-toolbar-field { color: var(--text-2); }
.st-count { margin-top: -10px; color: var(--text-2); font-size: var(--fs-caption); }
.w11-settings-card.st-app-row, button.w11-settings-card.st-app-row { min-height: 40px; padding-top: 10px; padding-bottom: 10px; }
.st-app-row .w11-sc-icon { min-width: 32px; margin: 0 16px 0 0; }
"#;

fn apps(p: Page) -> PageFuture {
    Box::pin(async move {
        let g = SettingsGroup::new("", "");
        for id in ["installed", "defaultapps", "startup"] {
            g.add(common::link_card(&p, id).widget());
        }
        let out: Content = vec![g.widget().clone().upcast()];
        out
    })
}

/// A "Sorteren op" field of the toolbar (`label.st-toolbar__field`): the caption and its control.
pub(super) fn toolbar_field(caption: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    b.add_css_class("st-toolbar-field");
    b.set_valign(gtk::Align::Center);
    let l = gtk::Label::new(Some(caption));
    l.set_valign(gtk::Align::Center);
    b.append(&l);
    b.append(control.as_ref());
    control.as_ref().update_property(&[gtk::accessible::Property::Label(caption)]);
    b
}
