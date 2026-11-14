//! A one-child widget for two CSS width rules GTK lacks:
//! - `Clamp::new(child, max)`: the child gets the width offered, but at most `max`, centred (alles.css: the
//!   search box is `width: min(100%, 680px)` in the title bar, an error bar `min(620px, 100%)`).
//! - `Clamp::shrink(child)`: asks no width of its own and clips the child (the command bar of a narrow window,
//!   where the labels go at the breakpoints like alles.css's container queries, instead of the window refusing
//!   to get narrower).
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Clamp {
        pub max: Cell<i32>,
        pub shrink: Cell<bool>,
        pub child: RefCell<Option<gtk::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Clamp {
        const NAME: &'static str = "AllesClamp";
        type Type = super::Clamp;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Clamp {
        fn dispose(&self) {
            if let Some(c) = self.child.borrow_mut().take() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Clamp {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(c) = self.child.borrow().clone() else { return (0, 0, -1, -1) };
            if self.shrink.get() {
                if orientation == gtk::Orientation::Horizontal {
                    let (_, nat, _, _) = c.measure(orientation, for_size);
                    return (0, nat, -1, -1);
                }
                let (min_w, _, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                let w = if for_size < 0 { -1 } else { for_size.max(min_w) };
                let (min, nat, _, _) = c.measure(orientation, w);
                return (min, nat, -1, -1);
            }
            let max = self.max.get();
            if orientation == gtk::Orientation::Horizontal {
                let (min, _, _, _) = c.measure(orientation, for_size);
                (min.min(max), max.max(min.min(max)), -1, -1)
            } else {
                let w = if for_size < 0 { -1 } else { for_size.min(max) };
                c.measure(orientation, w)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let Some(c) = self.child.borrow().clone() else { return };
            if self.shrink.get() {
                let (min_w, _, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                c.allocate(width.max(min_w), height, baseline, None);
                return;
            }
            let w = width.min(self.max.get()).max(0);
            let x = (width - w) / 2;
            let t = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, 0.0));
            c.allocate(w, height, baseline, Some(t));
        }
    }
}

glib::wrapper! {
    pub struct Clamp(ObjectSubclass<imp::Clamp>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Clamp {
    /// At most `max` wide, centred.
    pub fn new(child: &impl IsA<gtk::Widget>, max: i32) -> Clamp {
        let c: Clamp = glib::Object::new();
        c.imp().max.set(max);
        c.set_hexpand(true);
        child.set_parent(&c);
        *c.imp().child.borrow_mut() = Some(child.as_ref().clone());
        c
    }

    /// No width of its own; the child is clipped when it does not fit.
    pub fn shrink(child: &impl IsA<gtk::Widget>) -> Clamp {
        let c: Clamp = glib::Object::new();
        c.imp().shrink.set(true);
        c.set_overflow(gtk::Overflow::Hidden);
        child.set_parent(&c);
        *c.imp().child.borrow_mut() = Some(child.as_ref().clone());
        c
    }
}
