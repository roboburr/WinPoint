//! The desktop on the other monitors. A desktop window stays on one monitor: Mutter keeps it in that monitor's work
//! area, whatever size it is asked for (a run with two monitors, 16-11: asked for both, the desktop stayed 1440 x 852
//! on the first). So the desktop itself (grid.rs, with the icons) covers the primary monitor, and every other monitor
//! gets a window of its own with the wallpaper drawn for that monitor and the desktop's background menu on a right
//! click (its "dt" actions are the desktop's; what it makes goes to the icons on the primary monitor).
//!
//! Each window carries the Wayland app ID io.github.win11shell.SecondaryDesktop.Monitor<N>; the extension
//! (extension/desktopBridge.js) makes it a desktop window below everything on that monitor, as it does the desktop.
//! They follow desktop.geometry's `areas` (mod.rs `others`) and go with the desktop (its desktop-quit).
use super::grid::Desk;
use super::wallpaper::Wallpaper;
use gtk::gdk;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The app ID of the desktop window of another monitor, followed by its monitor's index.
pub const APP_ID_PREFIX: &str = "io.github.win11shell.SecondaryDesktop.Monitor";

struct Screen {
    index: i64,
    window: gtk::Window,
    size: Rc<Cell<(i32, i32)>>,
    wall: Rc<Wallpaper>,
}

thread_local! {
    static SCREENS: RefCell<Vec<Screen>> = const { RefCell::new(Vec::new()) };
}

/// One window per other monitor of desktop.geometry `geo` (mod.rs `others`), sized to it; the others go. Each
/// wallpaper, the desktop's too, learns where its monitor lies among all of them (wallpaper.rs `spans`).
pub fn sync(desk: &Rc<Desk>, geo: &serde_json::Value) {
    let others = super::others(geo);
    let spans = super::wallpaper::spans(geo);
    let gone: Vec<Screen> = SCREENS.with(|s| {
        let (keep, gone): (Vec<Screen>, Vec<Screen>) =
            s.borrow_mut().drain(..).partition(|sc| others.iter().any(|a| a.index == sc.index));
        *s.borrow_mut() = keep;
        gone
    });
    for screen in gone {
        screen.window.destroy();
    }
    for area in others {
        let size = (area.w.max(320), area.h.max(240));
        let span = spans.get(&area.index).copied();
        let existing = SCREENS.with(|s| {
            s.borrow().iter().find(|sc| sc.index == area.index).map(|sc| (sc.size.clone(), sc.window.clone(), sc.wall.clone()))
        });
        match existing {
            Some((cell, window, wall)) => {
                let moved = wall.set_span(span);
                let resized = cell.replace(size) != size;
                if resized {
                    window.set_default_size(size.0, size.1);
                    window.queue_resize();
                }
                if resized || moved {
                    wall.apply(&desk.shell, size, window.scale_factor().max(1), true);
                }
            }
            None => {
                let screen = create(desk, area.index, size, span);
                SCREENS.with(|s| s.borrow_mut().push(screen));
            }
        }
    }
    let primary = geo["monitor"].as_i64().and_then(|i| spans.get(&i).copied());
    if desk.wall.set_span(primary) {
        desk.apply_own_wallpaper();
    }
}

/// The language changed (Desk::on_settings): every other monitor's desktop window takes the title in the new one.
pub fn retitle(desk: &Rc<Desk>) {
    let name = desk.t("Bureaublad", "Desktop");
    SCREENS.with(|s| {
        for screen in s.borrow().iter() {
            if screen.window.title().as_deref() != Some(name.as_str()) {
                screen.window.set_title(Some(&name));
            }
        }
    });
}

/// Whether the desktop window of another monitor has the keyboard (its menu was used last).
pub fn has_focus() -> bool {
    SCREENS.with(|s| s.borrow().iter().any(|sc| sc.window.is_active()))
}

/// The wallpaper settings changed (Desk::apply_wallpaper): every other monitor's too.
pub fn apply_wallpaper(desk: &Rc<Desk>, instant: bool) {
    let all: Vec<(Rc<Wallpaper>, gtk::Window, (i32, i32))> =
        SCREENS.with(|s| s.borrow().iter().map(|sc| (sc.wall.clone(), sc.window.clone(), sc.size.get())).collect());
    for (wall, window, size) in all {
        wall.apply(&desk.shell, size, window.scale_factor().max(1), instant);
    }
}

/// The desktop went (desktop-quit): its windows on the other monitors go with it.
pub fn clear() {
    let all: Vec<Screen> = SCREENS.with(|s| s.borrow_mut().drain(..).collect());
    for screen in all {
        screen.window.destroy();
    }
}

fn create(desk: &Rc<Desk>, index: i64, size: (i32, i32), span: Option<super::wallpaper::Span>) -> Screen {
    let shell = &desk.shell;
    let window = gtk::Window::builder().decorated(false).title(shell.t("Bureaublad", "Desktop")).build();
    window.set_widget_name(&format!("desktop-window-m{index}"));
    window.add_css_class("w11-desktop");
    crate::wayland::set_app_id(&window, &format!("{APP_ID_PREFIX}{index}"));
    window.set_default_size(size.0, size.1);
    let cell = Rc::new(Cell::new(size));
    let wanted = cell.clone();
    crate::wayland::keep_size(&window, move || wanted.get());

    // as the desktop: black under a picture that is still loading, the wallpaper, a layer that takes the clicks
    let sizer = gtk::DrawingArea::new();
    sizer.set_hexpand(true);
    sizer.set_vexpand(true);
    sizer.set_draw_func(|_, cr, _, _| {
        cr.set_source_rgb(0.0, 0.0, 0.0);
        let _ = cr.paint();
    });
    let wall = Wallpaper::new();
    wall.set_span(span);
    wall.widget.set_widget_name(&format!("desktop-wall-{index}"));
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.set_widget_name(&format!("desktop-layer-m{index}"));
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&sizer));
    overlay.add_overlay(&wall.widget);
    overlay.add_overlay(&layer);
    window.set_child(Some(&overlay));
    window.insert_action_group("dt", Some(&desk.actions));

    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    let (weak, target) = (Rc::downgrade(desk), layer.downgrade());
    click.connect_pressed(move |g, _, x, y| {
        g.set_state(gtk::EventSequenceState::Claimed);
        if let (Some(desk), Some(layer)) = (weak.upgrade(), target.upgrade()) {
            super::menus::open_background_on(&desk, layer.upcast_ref(), x, y);
        }
    });
    layer.add_controller(click);
    // a click on another monitor's desktop clears the selection on the primary one, as a click on empty desktop does
    let press = gtk::GestureClick::new();
    press.set_button(gdk::BUTTON_PRIMARY);
    let weak = Rc::downgrade(desk);
    press.connect_pressed(move |_, _, _, _| {
        if let Some(desk) = weak.upgrade() {
            desk.clear_selection();
        }
    });
    layer.add_controller(press);
    // only the desktop's desktop-quit ends it (clear)
    window.connect_close_request(|_| gtk::glib::Propagation::Stop);
    crate::theme::style_window(&window);
    window.present();
    wall.apply(shell, size, window.scale_factor().max(1), true);
    Screen { index, window, size: cell, wall }
}
