//! "Win11 Shell reageert niet" (js/apps/taskmgr/hostbanner.js): an inline banner above every page while the host
//! does not answer (stuck or restarting). The page below keeps showing the last data it had; the button restarts
//! the shell. When the host answers again the banner goes away by itself, a toast says so and every page
//! refreshes (store.rs).
use crate::actions;
use crate::app::App;
use crate::hostcall::{self, State};
use kit::controls::{InfoBar, ProgressRing, Severity, Toast, Variant};
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "downTitle" => ("Win11 Shell reageert niet", "Win11 Shell is not responding"),
        "downData" => ("Taakbeheer blijft het proberen. Wat je hieronder ziet, is van {time}.", "Task Manager will keep trying. What you see below is from {time}."),
        "downNoData" => ("Taakbeheer blijft het proberen en maakt vanzelf weer verbinding.", "Task Manager will keep trying and reconnect automatically."),
        "gaveUp" => ("Win11 Shell is nog steeds niet terug. Probeer het nog een keer.", "Win11 Shell still isn\u{2019}t back. Try again."),
        "restart" => ("Win11 Shell opnieuw starten", "Restart Win11 Shell"),
        "restartingTitle" => ("Win11 Shell wordt opnieuw gestart\u{2026}", "Restarting Win11 Shell\u{2026}"),
        "restartingMsg" => ("Even geduld. Taakbeheer maakt vanzelf weer verbinding.", "Please wait. Task Manager will reconnect automatically."),
        "back" => ("Win11 Shell reageert weer", "Win11 Shell is responding again"),
        "restarted" => ("Win11 Shell is opnieuw gestart", "Win11 Shell has restarted"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

/// The banner (a cheap handle).
#[derive(Clone)]
pub struct HostBanner {
    bar: InfoBar,
    button: gtk::Button,
    ring: gtk::Box,
    gave_up: Rc<Cell<bool>>,
    shown: Rc<RefCell<String>>,
    app: Rc<App>,
}

impl HostBanner {
    pub fn new(app: &Rc<App>) -> HostBanner {
        let button = kit::controls::button_with_glyph("restart", tr("restart"), Variant::Accent);
        button.set_widget_name("taskmgr-restart-shell");
        let a = app.clone();
        button.connect_clicked(move |_| actions::restart_shell(&a));
        let ring = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ring.add_css_class("taskmgr-hostbar-busy");
        ring.set_valign(gtk::Align::Center);
        ring.append(ProgressRing::new(20, None).widget());
        let bar = InfoBar::new(Severity::Error, tr("downTitle"), "");
        bar.set_closable(false);
        bar.widget().add_css_class("taskmgr-hostbar");
        bar.widget().set_widget_name("taskmgr-hostbar");
        bar.set_visible(false);
        let me = HostBanner { bar, button, ring, gave_up: Rc::new(Cell::new(false)), shown: Rc::new(RefCell::new(String::new())), app: app.clone() };
        let m = me.clone();
        hostcall::health().on_change(move |st, info| {
            if st == State::Down {
                m.gave_up.set(info.gave_up);
            }
            if st == State::Ok {
                m.gave_up.set(false);
            }
            m.render();
            if st == State::Ok && info.prev.is_some_and(|p| p != State::Ok) && !info.cancelled {
                let title = if info.restarted { tr("restarted") } else { tr("back") };
                m.app.toast(Toast::new(title).severity(Severity::Success).timeout(3500));
            }
        });
        let m = me.clone();
        kit::i18n::on_change(move || {
            m.shown.borrow_mut().clear();
            m.render();
        });
        me.render();
        me
    }

    pub fn widget(&self) -> &gtk::Box {
        self.bar.widget()
    }

    pub fn render(&self) {
        let st = hostcall::health().state();
        crate::util::set_class(self.app.window.window(), "taskmgr-host-down", st != State::Ok);
        if st == State::Ok {
            self.bar.set_visible(false);
            self.shown.borrow_mut().clear();
            return;
        }
        let key = format!("{st:?}:{}:{}", self.gave_up.get(), kit::lang());
        let same = *self.shown.borrow() == key;
        if st == State::Restarting {
            if !same {
                self.bar.set_severity(Severity::Info);
                self.bar.set_action_widget(&self.ring);
            }
            self.bar.set_title(tr("restartingTitle"));
            self.bar.set_message(tr("restartingMsg"));
        } else {
            if !same {
                self.bar.set_severity(Severity::Error);
                self.bar.set_action_widget(&self.button);
                crate::util::set_button_label(&self.button, tr("restart"));
            }
            self.bar.set_title(tr("downTitle"));
            let at = hostcall::health().last_ok_at();
            let msg = if self.gave_up.get() {
                tr("gaveUp").to_owned()
            } else if at > 0.0 {
                crate::util::fill(tr("downData"), &[("time", &kit::format::time(at, true, kit::ctx().h24()))])
            } else {
                tr("downNoData").to_owned()
            };
            self.bar.set_message(&msg);
        }
        *self.shown.borrow_mut() = key;
        self.bar.set_visible(true);
    }
}
