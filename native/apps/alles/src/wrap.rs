//! A box that wraps its children into centred lines, each child at its natural width (alles.css .alles-quick:
//! `display: flex; flex-wrap: wrap; justify-content: center`). gtk::FlowBox lines its children up in columns.
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::Cell;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct WrapBox {
        pub spacing: Cell<i32>,
        pub line_spacing: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WrapBox {
        const NAME: &'static str = "AllesWrapBox";
        type Type = super::WrapBox;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for WrapBox {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for WrapBox {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let children = self.obj().children();
            if orientation == gtk::Orientation::Horizontal {
                let mut min = 0;
                let mut nat = 0;
                for (i, c) in children.iter().enumerate() {
                    let (m, n, _, _) = c.measure(orientation, -1);
                    min = min.max(m);
                    nat += n + if i > 0 { self.spacing.get() } else { 0 };
                }
                return (min, nat, -1, -1);
            }
            let width = if for_size < 0 { i32::MAX } else { for_size };
            let lines = self.lines(width);
            let h = lines.iter().map(|l| l.height).sum::<i32>() + self.line_spacing.get() * (lines.len() as i32 - 1).max(0);
            (h, h, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let mut y = 0;
            for line in self.lines(width) {
                let mut x = ((width - line.width) / 2).max(0);
                for (c, w) in &line.items {
                    let t = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, y as f32));
                    c.allocate(*w, line.height, -1, Some(t));
                    x += w + self.spacing.get();
                }
                y += line.height + self.line_spacing.get();
            }
        }
    }

    pub struct Line {
        pub items: Vec<(gtk::Widget, i32)>,
        pub width: i32,
        pub height: i32,
    }

    impl WrapBox {
        pub fn lines(&self, width: i32) -> Vec<Line> {
            let gap = self.spacing.get();
            let mut lines: Vec<Line> = Vec::new();
            for c in self.obj().children() {
                if !c.is_visible() {
                    continue;
                }
                let (min, nat, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                let w = nat.min(width).max(min);
                let (_, h, _, _) = c.measure(gtk::Orientation::Vertical, w);
                let fits = lines.last().is_some_and(|l| l.width + gap + w <= width);
                if fits {
                    if let Some(l) = lines.last_mut() {
                        l.width += gap + w;
                        l.height = l.height.max(h);
                        l.items.push((c, w));
                    }
                } else {
                    lines.push(Line { items: vec![(c, w)], width: w, height: h });
                }
            }
            lines
        }
    }
}

glib::wrapper! {
    pub struct WrapBox(ObjectSubclass<imp::WrapBox>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl WrapBox {
    pub fn new(spacing: i32, line_spacing: i32) -> WrapBox {
        let w: WrapBox = glib::Object::new();
        w.imp().spacing.set(spacing);
        w.imp().line_spacing.set(line_spacing);
        w
    }

    pub fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.set_parent(self);
    }

    fn children(&self) -> Vec<gtk::Widget> {
        let mut out = Vec::new();
        let mut c = self.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            out.push(w);
        }
        out
    }
}
