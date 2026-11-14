//! Win11 controls after vendor/ui/css/components.css and js/core/components.js, as GTK widgets with the kit's
//! CSS classes. Simple ones are functions returning the GTK widget; ones with state are small handle structs
//! (Clone, Rc inside) with `widget()` and `connect_*` methods. Texts the kit owns (On/Off, Search, Cancel,
//! Close, ...) follow a language switch by themselves; texts an app passes are the app's to relabel.
pub mod buttons;
pub mod cards;
pub mod dialog;
pub mod feedback;
pub mod inputs;
pub mod menu;
pub mod misc;
pub mod nav;
pub mod progress;
pub mod tabs;
pub mod toggles;

pub use buttons::{DropDownButton, SplitButton, Variant, button, button_with_glyph, hyperlink, icon_button};
pub use cards::{Chevron, Expander, SettingsCard, SettingsExpander, SettingsGroup};
pub use dialog::{Dialog, DialogResult, alert, confirm};
pub use feedback::{InfoBar, Severity, Toast, ToastHandle};
pub use inputs::{Dropdown, DropdownOption, SearchBox, Slider, Suggestion, field, number_box, text_area, text_box};
pub use menu::{MenuItem, TopAction, bind_context_menu, show_menu_at, show_menu_for, show_menu_with_top};
pub use misc::{Breadcrumb, ColorSwatches, avatar, badge, clear, empty_state, list_item, page_header, scrolled};
pub use nav::{NavItem, NavView};
pub use progress::{ProgressBar, ProgressRing, ProgressState};
pub use tabs::{Pivot, Segmented};
pub use toggles::{RadioGroup, ToggleSwitch, checkbox, checkbox_desc};

use gtk::prelude::*;

/// The widget owns a control's state, so the control keeps working when the app drops its handle. The state
/// must not hold `widget` itself strongly (that would be a cycle): it holds it as a WeakRef.
pub(crate) fn own<T: 'static>(widget: &impl IsA<gtk::Widget>, state: &std::rc::Rc<T>) {
    let key = format!("w11-kit-state-{}", std::any::type_name::<T>());
    // SAFETY: the value is only ever read back with the same type through this key (never, in fact: it is
    // there to be dropped with the widget).
    unsafe { widget.as_ref().set_data::<std::rc::Rc<T>>(&key, state.clone()) };
}

/// A label with the kit's defaults (left aligned, ellipsized unless `wrap`).
pub fn label(text: &str, class: &str, wrap: bool) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    if wrap {
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    } else {
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    }
    for c in class.split_whitespace() {
        l.add_css_class(c);
    }
    l
}
