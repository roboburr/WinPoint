//! A gtk::Box with an "activate" signal (its class activate signal), for the rows, tabs and page items that
//! are not buttons (a button would claim the press that a drag source or the rubber band needs): the dev
//! channel's "click" calls `gtk_widget_activate`, which emits it, so the harness can click them by name.
use gtk::glib;
use gtk::glib::subclass::Signal;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::sync::OnceLock;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ActBox;

    #[glib::object_subclass]
    impl ObjectSubclass for ActBox {
        const NAME: &'static str = "W11ExplorerActBox";
        type Type = super::ActBox;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.set_activate_signal_from_name("activate");
        }
    }

    impl ObjectImpl for ActBox {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder("activate").action().build()])
        }
    }

    impl WidgetImpl for ActBox {}
    impl BoxImpl for ActBox {}
}

glib::wrapper! {
    /// A box that can be activated (see the module docs).
    pub struct ActBox(ObjectSubclass<imp::ActBox>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl ActBox {
    pub fn new(orientation: gtk::Orientation, spacing: i32) -> ActBox {
        glib::Object::builder().property("orientation", orientation).property("spacing", spacing).build()
    }

    /// Run `f` on activation (the dev channel's click, `activate()`).
    pub fn connect_activated(&self, f: impl Fn(&ActBox) + 'static) {
        self.connect_local("activate", false, move |args| {
            if let Some(Ok(w)) = args.first().map(|a| a.get::<ActBox>()) {
                f(&w);
            }
            None
        });
    }
}
