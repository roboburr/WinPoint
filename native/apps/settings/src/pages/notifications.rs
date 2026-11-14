//! Instellingen › Systeem › Meldingen (vendor/ui/js/apps/settings/pages/notifications.js): toasts and their
//! sounds, Niet storen and a test notification.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("notifications", notifications)];
pub const CSS: &str = "";

fn notifications(p: Page) -> PageFuture {
    Box::pin(async move {
        let lst = p.call("notifications.list", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let supported = match &lst {
            Ok(v) => v["supported"] != false,
            Err(_) => true,
        };

        let sounds = c::checkbox(t("Meldingen mogen een geluid afspelen", "Allow notifications to play sounds"));
        sounds.set_active(common::setting_bool("notifications.sounds", true));
        sounds.set_widget_name("notifications-sounds");
        let silent = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let (pp, s2, silent) = (p.clone(), sounds.clone(), silent.clone());
            sounds.connect_toggled(move |b| {
                if silent.get() {
                    return;
                }
                let on = b.is_active();
                let (s3, silent) = (s2.clone(), silent.clone());
                common::set_setting(&pp, "notifications.sounds", json!(on), move |ok| {
                    if !ok {
                        silent.set(true);
                        s3.set_active(!on);
                        silent.set(false);
                    }
                });
            });
        }
        let toasts = common::setting_toggle(&p, "notifications.showToasts", false);
        toasts.switch().set_widget_name("notifications-toggle");
        let exp = SettingsExpander::new(
            "notification",
            t("Meldingen", "Notifications"),
            t("Meldingen ontvangen van apps en andere afzenders", "Get notifications from apps and other senders"),
        );
        exp.header().set_control(toasts.widget());
        exp.add(&sounds);
        exp.set_expanded(true);

        let (dnd_card, dnd) = common::toggle_card(
            &p,
            "notification-off",
            t("Niet storen", "Do not disturb"),
            t("Meldingen worden stil bewaard in het meldingencentrum", "Notifications are kept quietly in the notification centre"),
            "notifications.dnd",
            false,
        );
        dnd.switch().set_widget_name("notifications-dnd");
        let dnd_note = common::notice(t("Niet storen staat aan: je ziet geen meldingen in beeld.", "Do not disturb is on: notifications won’t pop up."));
        dnd_note.widget().set_visible(common::setting_bool("notifications.dnd", false));
        {
            let (note, s2) = (dnd_note.clone(), sounds.clone());
            p.on_settings(move |_, _| {
                note.widget().set_visible(common::setting_bool("notifications.dnd", false));
                silent.set(true);
                s2.set_active(common::setting_bool("notifications.sounds", true));
                silent.set(false);
            });
        }

        let send = c::button(t("Versturen", "Send"), c::Variant::Standard);
        send.set_widget_name("notifications-test");
        let pp = p.clone();
        send.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (pp, b) = (pp.clone(), b.clone());
            let wait = pp.call("notifications.test", json!({}));
            pp.clone().spawn(async move {
                let r = wait.await;
                b.set_sensitive(true);
                if r.is_ok() {
                    pp.toast(Toast::new(t("Testmelding verstuurd", "Test notification sent")).severity(Severity::Success).timeout(2500));
                } else {
                    pp.toast(
                        Toast::new(t("Testmelding", "Test notification"))
                            .glyph("notification")
                            .message(t("Zo ziet een melding eruit.", "This is what a notification looks like."))
                            .timeout(4000),
                    );
                }
            });
        });
        let test = SettingsCard::new("play", t("Testmelding versturen", "Send a test notification"), t("Bekijk hoe een melding eruitziet", "See what a notification looks like"));
        test.set_control(&send);

        let mut out: Content = Vec::new();
        if !supported {
            out.push(
                common::notice(t(
                    "Onder GNOME toont GNOME zelf de meldingen. Deze instellingen gelden voor Win11 Shell.",
                    "In GNOME, notifications are shown by GNOME itself. These settings apply to Win11 Shell.",
                ))
                .widget()
                .clone()
                .upcast(),
            );
        }
        let g1 = SettingsGroup::new("", "");
        g1.add(exp.widget());
        g1.add(dnd_card.widget());
        g1.add(dnd_note.widget());
        let g2 = SettingsGroup::new("", "");
        g2.add(test.widget());
        out.push(g1.widget().clone().upcast());
        out.push(g2.widget().clone().upcast());
        out
    })
}
