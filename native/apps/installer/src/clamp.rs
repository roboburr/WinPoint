//! A holder for one child that is never wider than a maximum (CSS `max-width` with `box-sizing: border-box`),
//! centred (`margin: 0 auto`) or at the start, and that reports the width it was given, for the app's CSS media
//! queries (`@media (max-width: 650px)`). GTK 4 has no max-width of its own.
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Clamp {
        pub max: Cell<i32>,
        pub center: Cell<bool>,
        pub child: RefCell<Option<gtk::Widget>>,
        pub on_width: RefCell<Option<Rc<dyn Fn(i32)>>>,
        pub width: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Clamp {
        const NAME: &'static str = "InstallerClamp";
        type Type = super::Clamp;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Clamp {
        fn dispose(&self) {
            if let Some(child) = self.child.borrow_mut().take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Clamp {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.child.borrow().clone() else { return (0, 0, -1, -1) };
            if orientation == gtk::Orientation::Horizontal {
                let (min, nat, _, _) = child.measure(orientation, for_size);
                (min, nat.min(self.max.get()).max(min), -1, -1)
            } else {
                let width = if for_size < 0 { -1 } else { self.child_width(&child, for_size) };
                let (min, nat, _, _) = child.measure(orientation, width);
                (min, nat, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            if let Some(child) = self.child.borrow().clone() {
                let w = self.child_width(&child, width);
                let x = if self.center.get() { (width - w).max(0) / 2 } else { 0 };
                child.size_allocate(&gtk::Allocation::new(x, 0, w, height), -1);
            }
            if self.width.replace(width) != width
                && let Some(f) = self.on_width.borrow().clone()
            {
                // Not during the allocation: the callback may change CSS classes or sizes.
                glib::idle_add_local_once(move || f(width));
            }
        }
    }

    impl Clamp {
        fn child_width(&self, child: &gtk::Widget, available: i32) -> i32 {
            let (min, _, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
            available.min(self.max.get()).max(min)
        }
    }
}

glib::wrapper! {
    pub struct Clamp(ObjectSubclass<imp::Clamp>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Clamp {
    /// `child` at most `max` px wide; `center` centres it in a wider allocation, else it sits at the start.
    pub fn new(child: &impl IsA<gtk::Widget>, max: i32, center: bool) -> Clamp {
        let clamp: Clamp = glib::Object::new();
        clamp.imp().max.set(max);
        clamp.imp().center.set(center);
        child.set_parent(&clamp);
        *clamp.imp().child.borrow_mut() = Some(child.clone().upcast());
        clamp
    }

    /// A new maximum width.
    pub fn set_max(&self, max: i32) {
        if self.imp().max.replace(max) != max {
            self.queue_resize();
        }
    }

    /// Run `f(width)` (on idle) whenever the width this clamp is given changes.
    pub fn connect_width(&self, f: impl Fn(i32) + 'static) {
        *self.imp().on_width.borrow_mut() = Some(Rc::new(f));
    }
}
