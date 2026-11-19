//! The kit's widget tests under GTK: one test, as GTK belongs to the thread that initialises it. It runs only with
//! WIN11_KIT_GTK_TESTS=1, on a display of its own (a private gtk4-broadwayd with a private session bus): the test
//! suite runs inside the user's session, and a test must not open a display connection there. Without it, or when
//! gtk::init fails, the test is skipped.
use super::*;
use crate::controls::cards::{CARD_HOST_CLASS, Chevron, SettingsCard, SettingsExpander};
use crate::controls::dialog::Dialog;
use crate::controls::misc::Breadcrumb;
use std::cell::Cell;
use std::time::Instant;

/// Run the main loop until `done` holds (at most 5 s).
fn settle(done: impl Fn() -> bool) -> bool {
    let ctx = glib::MainContext::default();
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        while ctx.pending() {
            ctx.iteration(false);
        }
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

fn window_with(child: &impl IsA<gtk::Widget>, width: i32) -> gtk::Window {
    let w = gtk::Window::new();
    w.set_default_size(width, 200);
    w.set_child(Some(child));
    w.present();
    w
}

fn children(w: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    let mut c = w.first_child();
    while let Some(x) = c {
        c = x.next_sibling();
        out.push(x);
    }
    out
}

fn bounds(w: &impl IsA<gtk::Widget>, of: &impl IsA<gtk::Widget>) -> [i32; 4] {
    let r = w.compute_bounds(of).expect("bounds");
    [
        r.x().round() as i32,
        r.y().round() as i32,
        r.width().round() as i32,
        r.height().round() as i32,
    ]
}

fn named(tree: &Value, name: &str) -> bool {
    tree["name"] == name
        || tree["children"]
            .as_array()
            .is_some_and(|c| c.iter().any(|t| named(t, name)))
}

#[test]
fn kit_widgets_under_gtk() {
    if std::env::var("WIN11_KIT_GTK_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped: set WIN11_KIT_GTK_TESTS=1 on a private display (see the module docs)");
        return;
    }
    if gtk::init().is_err() {
        eprintln!("skipped: gtk::init failed (no display)");
        return;
    }
    if let Some(s) = gtk::Settings::default() {
        s.set_gtk_enable_animations(false);
    }
    // the kit's controls sheet parses without errors (the card, expander and breadcrumb rules of this package)
    let errors = std::rc::Rc::new(Cell::new(0));
    let css = gtk::CssProvider::new();
    let e = errors.clone();
    css.connect_parsing_error(move |_, section, error| {
        eprintln!("controls.css {}: {}", section.to_str(), error);
        e.set(e.get() + 1);
    });
    css.load_from_string(include_str!("../css/controls.css"));
    assert_eq!(errors.get(), 0, "controls.css parses");
    crate::theme::install();
    cards_keep_their_control_beside_the_button();
    expander_header_control_and_nested_rows();
    dialog_primary_label();
    radio_names_follow_the_group();
    breadcrumb_keeps_the_current_page();
    dev_tree_leaves_out_what_gtk_does_not_show();
}

/// K2: a clickable card's control is not inside its button, lies exactly over its place in the row, and the dev
/// click on the card (named through widget()) clicks the button.
fn cards_keep_their_control_beside_the_button() {
    let card = SettingsCard::clickable(
        "",
        "Meldingen",
        "Meldingen van apps en andere afzenders",
        Chevron::Right,
    );
    let button = card.button().expect("a clickable card has a button");
    let sw = gtk::Switch::new();
    card.set_control(&sw);
    assert!(
        !sw.is_ancestor(&button),
        "the switch is not inside the card's button"
    );
    assert!(sw.is_ancestor(card.widget()), "the switch is in the card");
    assert!(card.widget().is::<gtk::Overlay>() && card.widget().has_css_class(CARD_HOST_CLASS));
    assert!(
        button.has_css_class("w11-settings-card")
            && !card.widget().has_css_class("w11-settings-card")
    );
    // a page's class on the card reaches the button (button.w11-settings-card.st-app-row), and goes again
    card.widget().add_css_class("st-test-row");
    assert!(button.has_css_class("st-test-row"));
    card.widget().remove_css_class("st-test-row");
    assert!(!button.has_css_class("st-test-row"));
    let clicks = std::rc::Rc::new(Cell::new(0));
    let c = clicks.clone();
    card.connect_clicked(move || c.set(c.get() + 1));
    card.widget().set_widget_name("kit-test-card");
    let window = window_with(card.widget(), 640);
    assert!(
        settle(|| sw.width() > 0 && button.width() > 0),
        "the card is allocated"
    );
    let row = button.child().expect("the button's row");
    let parts = children(&row);
    assert_eq!(parts.len(), 4, "icon, text, the control's place, chevron");
    let (text, place, chevron) = (&parts[1], &parts[2], &parts[3]);
    let control = card.control_box();
    // the place is the control's margin box (its 16 px margin-left included), the bounds are its border box
    let (pb, cb) = (bounds(place, card.widget()), bounds(control, card.widget()));
    eprintln!(
        "card: place {pb:?} control {cb:?} button {:?}",
        bounds(&button, card.widget())
    );
    assert!(
        pb[2] > 0 && pb[3] > 0,
        "the place has the control's size: {pb:?}"
    );
    assert_eq!(
        (cb[0] + cb[2], cb[1], cb[3]),
        (pb[0] + pb[2], pb[1], pb[3]),
        "the control lies over its place"
    );
    assert!(
        cb[0] >= pb[0] && cb[0] - pb[0] <= 16,
        "only the control's margin lies between: {pb:?} {cb:?}"
    );
    let (t, ch) = (bounds(text, card.widget()), bounds(chevron, card.widget()));
    assert!(
        t[0] + t[2] <= cb[0],
        "the text ends before the control: {t:?} {cb:?}"
    );
    assert!(
        cb[0] + cb[2] <= ch[0],
        "the chevron follows the control: {cb:?} {ch:?}"
    );
    let b = bounds(&button, card.widget());
    assert!(
        cb[1] >= b[1] && cb[1] + cb[3] <= b[1] + b[3],
        "the control is within the card's height: {cb:?} {b:?}"
    );
    // the harness's dev click on the card's name clicks its button
    assert_eq!(
        run(&json!({"native": "click", "name": "kit-test-card"}))["clicked"],
        "kit-test-card"
    );
    assert_eq!(clicks.get(), 1);
    // without a control the place is gone too
    card.clear_control();
    assert!(!place.is_visible());
    card.set_control(&gtk::Button::with_label("Kopiëren"));
    assert!(place.is_visible());
    window.destroy();
    // a plain card stays one box, its control inside it
    let plain = SettingsCard::new("", "Titel", "");
    let sw2 = gtk::Switch::new();
    plain.set_control(&sw2);
    assert!(
        plain.button().is_none()
            && plain.widget().is::<gtk::Box>()
            && sw2.is_ancestor(plain.widget())
    );
}

/// K2: a control on a SettingsExpander's header is not inside the header button; a clickable card added as a nested
/// row is a card row (no w11-nested-other padding).
fn expander_header_control_and_nested_rows() {
    let exp = SettingsExpander::new("", "Meldingen", "");
    let sw = gtk::Switch::new();
    exp.header().set_control(&sw);
    assert!(
        !sw.is_ancestor(exp.header_button()),
        "the header's switch is not inside the header button"
    );
    assert!(sw.is_ancestor(exp.widget()));
    assert_eq!(exp.header().button().as_ref(), Some(exp.header_button()));
    let row = SettingsCard::clickable("", "Geavanceerd", "", Chevron::Right);
    exp.add(row.widget());
    assert!(!row.widget().has_css_class("w11-nested-other"));
    let other = gtk::Label::new(Some("x"));
    exp.add(&other);
    assert!(other.has_css_class("w11-nested-other"));
    let toggled = std::rc::Rc::new(Cell::new(0));
    let t = toggled.clone();
    exp.connect_toggled(move |_| t.set(t.get() + 1));
    let window = window_with(exp.widget(), 640);
    assert!(settle(|| sw.width() > 0), "the header is allocated");
    // the chevron of the expander follows the control
    let parts = children(&exp.header_button().child().expect("row"));
    let (sb, cb) = (
        bounds(&sw, exp.widget()),
        bounds(parts.last().expect("chevron"), exp.widget()),
    );
    assert!(
        sb[0] + sb[2] <= cb[0],
        "the expander chevron follows the header's control: {sb:?} {cb:?}"
    );
    exp.header_button().emit_clicked();
    assert!(exp.expanded() && toggled.get() == 1);
    window.destroy();
}

/// K6: the primary button takes a new text after the dialog is built.
fn dialog_primary_label() {
    let d = Dialog::new("Aanmelden")
        .primary("Aanmelden")
        .close_label("Annuleren");
    let buttons = d.build_buttons_for_test();
    d.set_primary_label("Opnieuw proberen");
    assert_eq!(buttons[0].widget_name(), "w11-dialog-primary");
    assert_eq!(buttons[0].label().as_deref(), Some("Opnieuw proberen"));
    assert_eq!(buttons[1].label().as_deref(), Some("Annuleren"));
}

/// K5: the earlier items give way before the current page; the crumb buttons keep their label and name.
fn breadcrumb_keeps_the_current_page() {
    let bc = Breadcrumb::new(true);
    bc.set_items(&[
        ("privacy", "Privacy en beveiliging"),
        ("diagnostics", "Geavanceerde opties"),
    ]);
    let parts = children(bc.widget().upcast_ref());
    assert_eq!(parts.len(), 3);
    let item = parts[0].downcast_ref::<gtk::Button>().expect("item button");
    assert_eq!(item.widget_name(), "crumb-privacy");
    assert_eq!(item.label().as_deref(), Some("Privacy en beveiliging"));
    let item_label = item
        .child()
        .and_downcast::<gtk::Label>()
        .expect("label child");
    assert_eq!(item_label.ellipsize(), gtk::pango::EllipsizeMode::End);
    let current = parts[2]
        .downcast_ref::<gtk::Label>()
        .expect("current label");
    assert!(current.has_css_class("w11-breadcrumb-current"));
    // Root and allocate before measuring: inherited theme fonts are resolved on the display.
    bc.widget().set_hexpand(true);
    let window = window_with(bc.widget(), 1000);
    assert!(
        settle(|| current.width() > 0 && bc.widget().width() > 0),
        "styled and allocated"
    );
    let (_, current_nat, _, _) = current.measure(gtk::Orientation::Horizontal, -1);
    let (item_min, item_nat, _, _) = item.measure(gtk::Orientation::Horizontal, -1);
    let (sep_min, _, _, _) = parts[1].measure(gtk::Orientation::Horizontal, -1);
    // room for the whole current title and a little of the item: the item shrinks, the title stays whole
    let (sep_nat, all) = (
        parts[1].measure(gtk::Orientation::Horizontal, -1).1,
        current_nat + item_nat,
    );
    let width = current_nat + sep_min + item_min + (item_nat - item_min) / 3;
    window.set_default_size(width, 200);
    assert!(
        settle(|| bc.widget().width() < all + sep_nat),
        "resized to the planned room"
    );
    let got = bc.widget().width();
    eprintln!(
        "breadcrumb: {got} px of {} natural; current {} of {current_nat}, item {} of {item_nat}",
        all + sep_nat,
        current.width(),
        item.width()
    );
    assert!(
        got >= current_nat + sep_min + item_min && got < all + sep_nat,
        "the window gave the planned room: {got}"
    );
    // measure() includes CSS padding; width() is the content allocation. Check the actual text layout instead.
    let layout = current.layout();
    let (text_width, _) = layout.pixel_size();
    assert!(
        !layout.is_ellipsized() && current.width() >= text_width,
        "the current crumb is whole: {} content pixels for {text_width} text pixels",
        current.width()
    );
    assert!(
        item.width() < item_nat,
        "the earlier crumb gives way: {} of {item_nat}",
        item.width()
    );
    window.destroy();
    // a short current title is never shortened
    bc.set_items(&[("system", "Systeem"), ("display", "Beeldscherm")]);
    let short = children(bc.widget().upcast_ref())[2]
        .clone()
        .downcast::<gtk::Label>()
        .expect("label");
    assert_eq!(short.ellipsize(), gtk::pango::EllipsizeMode::None);
    // Titles beyond the 24-character guarantee may shorten, but retain the planned minimum.
    bc.set_items(&[
        ("privacy", "Privacy"),
        ("diagnostics", "Diagnostische gegevens en feedback"),
    ]);
    let long = children(bc.widget().upcast_ref())[2]
        .clone()
        .downcast::<gtk::Label>()
        .expect("long label");
    assert_eq!(long.ellipsize(), gtk::pango::EllipsizeMode::End);
    assert_eq!(long.width_chars(), 24);
}

/// K7: the dev state leaves out a closed expander's rows (a revealer keeps them visible, not child-visible), find
/// with `visible` skips them, the name search still finds them, and they are back once it opens.
fn dev_tree_leaves_out_what_gtk_does_not_show() {
    let exp = SettingsExpander::new("", "Kopje", "");
    let nested = gtk::Button::with_label("Binnen");
    nested.set_widget_name("kit-test-nested");
    exp.add(&nested);
    let window = window_with(exp.widget(), 480);
    assert!(settle(|| exp.header_button().width() > 0), "allocated");
    let revealer = nested
        .ancestor(gtk::Revealer::static_type())
        .and_downcast::<gtk::Revealer>()
        .expect("revealer");
    let content = revealer.child().expect("content");
    assert!(
        nested.is_visible() && content.is_visible() && !content.is_child_visible(),
        "GTK's closed revealer"
    );
    let root: gtk::Widget = window.clone().upcast();
    assert!(
        !named(&tree(&root), "kit-test-nested"),
        "a closed expander's row is not in the state"
    );
    assert!(find_in(&root, "kit-test-nested", true).is_none());
    assert!(
        find_in(&root, "kit-test-nested", false).is_some(),
        "the second pass still finds it by name"
    );
    exp.set_expanded(true);
    assert!(
        settle(|| content.is_child_visible() && named(&tree(&root), "kit-test-nested")),
        "an open expander's row"
    );
    assert!(find_in(&root, "kit-test-nested", true).is_some());
    window.destroy();
}

fn radio_names_follow_the_group() {
    let group = crate::controls::RadioGroup::new(&[("one", "Een"), ("two", "Twee")], Some("one"), None, false);
    group.widget().set_widget_name("kit-test-radio");
    assert_eq!(group.button("one").unwrap().widget_name(), "kit-test-radio-one");
    group.set_options(&[("three", "Drie")], Some("three"));
    assert_eq!(group.button("three").unwrap().widget_name(), "kit-test-radio-three");
    group.widget().set_widget_name("kit-renamed-radio");
    assert_eq!(group.button("three").unwrap().widget_name(), "kit-renamed-radio-three");
}
