//! The panes on the right (panes.js): the preview (thumbnail or icon, name, type, size, date, location and
//! Open / Open folder) and the search tips (examples to try, keyboard shortcuts).
use crate::model::Entry;
use crate::text;
use kit::controls::{self, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

const THUMB_EXT: [&str; 14] = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "heic", "heif", "avif", "tif", "tiff", "ico", "jxl"];

type EntryFn = Rc<dyn Fn(&Entry)>;

struct PreviewInner {
    scroller: gtk::ScrolledWindow,
    media: gtk::Box,
    body: gtk::Box,
    current: RefCell<Option<Entry>>,
    on_open: EntryFn,
    on_folder: EntryFn,
}

/// The preview pane: `show(entry)`, `show_many(count, size)`, `clear()`.
#[derive(Clone)]
pub struct PreviewPane {
    inner: Rc<PreviewInner>,
}

impl PreviewPane {
    pub fn new(on_open: impl Fn(&Entry) + 'static, on_folder: impl Fn(&Entry) + 'static) -> PreviewPane {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("alles-pane");
        root.add_css_class("alles-preview");
        root.set_widget_name("alles-preview-pane");
        let media = gtk::Box::new(gtk::Orientation::Vertical, 0);
        media.add_css_class("alles-pv-media");
        media.set_size_request(-1, 188);
        // 188 px high whatever its picture wants (its child expands inside it)
        media.set_vexpand(false);
        media.set_overflow(gtk::Overflow::Hidden);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.add_css_class("alles-pv-body");
        root.append(&media);
        root.append(&body);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.add_css_class("alles-pane-host");
        scroller.set_child(Some(&root));
        let me = PreviewPane {
            inner: Rc::new(PreviewInner {
                scroller,
                media,
                body,
                current: RefCell::new(None),
                on_open: Rc::new(on_open),
                on_folder: Rc::new(on_folder),
            }),
        };
        me.clear();
        me
    }

    /// The pane (scrolling).
    pub fn widget(&self) -> gtk::Widget {
        self.inner.scroller.clone().upcast()
    }

    fn media_center(&self, w: &impl IsA<gtk::Widget>) {
        controls::clear(&self.inner.media);
        let w = w.as_ref();
        w.set_halign(gtk::Align::Center);
        w.set_valign(gtk::Align::Center);
        w.set_vexpand(true);
        self.inner.media.append(w);
    }

    /// Nothing chosen.
    pub fn clear(&self) {
        *self.inner.current.borrow_mut() = None;
        let g = kit::icons::glyph("view", 40);
        g.add_css_class("alles-pv-glyph");
        g.add_css_class("alles-muted");
        self.media_center(&g);
        self.inner.media.add_css_class("alles-empty-media");
        controls::clear(&self.inner.body);
        let l = controls::label(t("Kies een bestand om er meer over te zien", "Choose a file to see more about it"), "alles-pv-hint", true);
        l.set_justify(gtk::Justification::Center);
        l.set_xalign(0.5);
        self.inner.body.append(&l);
    }

    /// Show the language anew (a language switch).
    pub fn relabel(&self) {
        let cur = self.inner.current.borrow_mut().take();
        match cur {
            Some(e) => self.show(Some(&e)),
            None => self.clear(),
        }
    }

    fn big_icon(&self, e: &Entry) {
        let img = kit::icons::file_icon(if e.is_dir { "folder" } else { e.icon_mime() }, 64);
        img.add_css_class("alles-pv-icon");
        self.media_center(&img);
    }

    /// One item (None: nothing chosen).
    pub fn show(&self, e: Option<&Entry>) {
        let Some(e) = e else {
            self.clear();
            return;
        };
        if let Some(cur) = self.inner.current.borrow().as_ref()
            && cur.path == e.path
            && cur.modified == e.modified
            && cur.size == e.size
        {
            return;
        }
        *self.inner.current.borrow_mut() = Some(e.clone());
        self.inner.media.remove_css_class("alles-empty-media");
        self.big_icon(e);
        if !e.is_dir && THUMB_EXT.contains(&e.ext.to_lowercase().as_str()) {
            let weak = Rc::downgrade(&self.inner);
            let path = e.path.clone();
            kit::icons::thumbnail(&e.path, 256, move |tex| {
                let (Some(inner), Some(tex)) = (weak.upgrade(), tex) else { return };
                if inner.current.borrow().as_ref().is_none_or(|c| c.path != path) {
                    return;
                }
                let pic = gtk::Picture::for_paintable(&tex);
                pic.set_content_fit(gtk::ContentFit::Contain);
                pic.set_can_shrink(true);
                pic.add_css_class("alles-pv-thumb");
                pic.set_hexpand(true);
                pic.set_vexpand(true);
                controls::clear(&inner.media);
                inner.media.append(&pic);
            });
        }
        let body = &self.inner.body;
        controls::clear(body);
        let name = controls::label(&e.name, "alles-pv-name", true);
        name.set_selectable(true);
        name.set_can_focus(false);
        body.append(&name);
        body.append(&controls::label(&text::type_name(e), "alles-pv-type", true));
        let props = gtk::Box::new(gtk::Orientation::Vertical, 10);
        props.add_css_class("alles-pv-props");
        let row = |label: &str, value: &str| {
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            r.add_css_class("alles-pv-row");
            let dt = controls::label(label, "alles-pv-dt", true);
            dt.set_size_request(96, -1);
            dt.set_valign(gtk::Align::Start);
            let dd = controls::label(value, "alles-pv-dd", true);
            dd.set_selectable(true);
            dd.set_can_focus(false);
            dd.set_hexpand(true);
            r.append(&dt);
            r.append(&dd);
            r
        };
        let size = if e.is_dir && e.size == 0.0 { "–".to_owned() } else { kit::format::bytes(e.size) };
        props.append(&row(if e.is_dir { t("Grootte van inhoud", "Size of contents") } else { t("Grootte", "Size") }, &size));
        let modified = if e.modified > 0.0 { kit::format::file_time(e.modified) } else { "–".to_owned() };
        props.append(&row(t("Gewijzigd op", "Date modified"), &modified));
        let loc = gtk::Box::new(gtk::Orientation::Vertical, 2);
        loc.add_css_class("alles-pv-row");
        loc.append(&controls::label(t("Locatie", "Location"), "alles-pv-dt", true));
        let link = gtk::Button::new();
        link.add_css_class("alles-pv-link");
        link.set_halign(gtk::Align::Start);
        link.set_tooltip_text(Some(e.folder()));
        let lr = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let g = kit::icons::glyph("folder-open", 14);
        g.set_valign(gtk::Align::Start);
        g.add_css_class("alles-pv-link-icon");
        lr.append(&g);
        // a break chance before every "/" (panes.js <wbr>)
        let shown = e.folder().replace('/', "\u{200b}/");
        let ll = controls::label(&shown, "alles-pv-link-text", true);
        lr.append(&ll);
        link.set_child(Some(&lr));
        let f = self.inner.on_folder.clone();
        let entry = e.clone();
        link.connect_clicked(move |_| f(&entry));
        loc.append(&link);
        props.append(&loc);
        body.append(&props);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("alles-pv-actions");
        let open = controls::button_with_glyph("open", t("Openen", "Open"), Variant::Accent);
        open.set_widget_name("alles-preview-open");
        let f = self.inner.on_open.clone();
        let entry = e.clone();
        open.connect_clicked(move |_| f(&entry));
        let folder = controls::button_with_glyph("folder-open", t("Map openen", "Open folder"), Variant::Standard);
        folder.set_widget_name("alles-preview-folder");
        let f = self.inner.on_folder.clone();
        let entry = e.clone();
        folder.connect_clicked(move |_| f(&entry));
        actions.append(&open);
        actions.append(&folder);
        body.append(&actions);
    }

    /// Several items selected (`size` None: not all of them known yet).
    pub fn show_many(&self, n: u64, size: Option<f64>) {
        *self.inner.current.borrow_mut() = None;
        self.inner.media.add_css_class("alles-empty-media");
        let g = kit::icons::glyph("copy", 40);
        g.add_css_class("alles-pv-glyph");
        self.media_center(&g);
        let body = &self.inner.body;
        controls::clear(body);
        body.append(&controls::label(&text::items_selected(n), "alles-pv-name", true));
        if let Some(size) = size {
            let props = gtk::Box::new(gtk::Orientation::Vertical, 10);
            props.add_css_class("alles-pv-props");
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            r.add_css_class("alles-pv-row");
            let dt = controls::label(t("Totale grootte", "Total size"), "alles-pv-dt", true);
            dt.set_size_request(96, -1);
            r.append(&dt);
            let dd = controls::label(&kit::format::bytes(size), "alles-pv-dd", true);
            dd.set_hexpand(true);
            r.append(&dd);
            props.append(&r);
            body.append(&props);
        }
    }
}

// -------------------------------------------------------------------------------------------------- tips

/// (example nl, example en, description nl, description en); one example for both languages when equal.
type Tip = (&'static str, &'static str, &'static str, &'static str);

const TIP_GROUPS: [(&str, &str, &[Tip]); 6] = [
    ("De basis", "The basics", &[
        ("foto", "photo", "Vindt alles met “foto” in de naam, ook “Vakantiefoto’s”.", "Finds everything with “photo” in the name, including “Holiday photos”."),
        ("vakantie 2024", "holiday 2024", "Spatie: alle woorden moeten erin staan.", "Space: all words must match."),
        ("\"mijn document\"", "\"my document\"", "Aanhalingstekens: precies deze woorden, in deze volgorde.", "Quotes: exactly these words, in this order."),
    ]),
    ("Combineren", "Combining", &[
        ("jpg|png", "jpg|png", "Streep: het een óf het ander.", "Bar: one or the other."),
        ("rapport !concept", "report !draft", "Uitroepteken: dit woord juist niet.", "Exclamation mark: not this word."),
        ("<foto|video> 2024", "<photo|video> 2024", "Punthaken groeperen een deel van je zoekopdracht.", "Angle brackets group part of your search."),
    ]),
    ("Jokertekens", "Wildcards", &[
        ("*.pdf", "*.pdf", "Sterretje: willekeurige tekens. Dit zijn alle pdf-bestanden.", "Asterisk: any characters. These are all PDF files."),
        ("IMG_00??.jpg", "IMG_00??.jpg", "Vraagteken: precies één teken.", "Question mark: exactly one character."),
    ]),
    ("Filters", "Filters", &[
        ("ext:mp3;flac", "ext:mp3;flac", "Bestanden met deze extensie(s).", "Files with these extension(s)."),
        ("size:>1gb", "size:>1gb", "Groter dan 1 GB. Ook: size:<10mb of size:10mb..100mb.", "Larger than 1 GB. Also: size:<10mb or size:10mb..100mb."),
        ("dm:vandaag", "dm:today", "Vandaag gewijzigd. Ook: gisteren, dezeweek, dezemaand, ditjaar.", "Changed today. Also: yesterday, thisweek, thismonth, thisyear."),
        ("dm:2026-10-01..2026-10-25", "dm:2026-10-01..2026-10-25", "Gewijzigd tussen twee datums.", "Changed between two dates."),
        ("folder:vakantie", "folder:holiday", "Alleen mappen (met “vakantie” in de naam).", "Folders only (with “holiday” in the name)."),
        ("file: notities", "file: notes", "Alleen bestanden.", "Files only."),
        ("pic:", "pic:", "Alleen afbeeldingen. Ook: audio:, video:, doc:, zip:, exe:.", "Pictures only. Also: audio:, video:, doc:, zip:, exe:."),
        ("empty:", "empty:", "Lege mappen.", "Empty folders."),
    ]),
    ("Plaatsen", "Places", &[
        ("parent:~/Downloads", "parent:~/Downloads", "Alles wat direct in deze map staat.", "Everything directly in this folder."),
        ("in:~/Documenten factuur", "in:~/Documents invoice", "Alles in deze map en de mappen daarbinnen.", "Everything in this folder and the folders inside it."),
        ("path:werk offerte", "path:work quote", "Zoek ook in mapnamen: bestanden in een map “werk”.", "Search folder names too: files in a folder “work”."),
    ]),
    ("Voor gevorderden", "Advanced", &[
        ("case:IMG", "case:IMG", "Let op hoofdletters.", "Match upper and lower case."),
        ("ww:map", "ww:folder", "Alleen het hele woord “map”, niet “mappen”.", "Only the whole word “folder”, not “folders”."),
        ("regex:^IMG_\\d+", "regex:^IMG_\\d+", "Reguliere expressie voor precieze patronen.", "Regular expression for exact patterns."),
    ]),
];

const KEYS: [(&str, &str, &str); 6] = [
    ("Enter", "Openen", "Open"),
    ("Ctrl+Enter", "Map met bestand openen", "Open containing folder"),
    ("Ctrl+C", "Pad kopiëren", "Copy path"),
    ("Ctrl+F", "Naar het zoekvak", "Go to the search box"),
    ("Alt+Enter", "Eigenschappen", "Properties"),
    ("Delete", "Naar Prullenbak", "Move to Recycle Bin"),
];

/// The search tips pane; `on_example(query)` tries an example, `on_close` closes the pane.
pub fn tips_pane(on_example: impl Fn(&str) + 'static, on_close: impl Fn() + 'static) -> gtk::Widget {
    let on_example: Rc<dyn Fn(&str)> = Rc::new(on_example);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("alles-pane");
    root.add_css_class("alles-tips");
    root.set_widget_name("alles-tips-pane");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    head.add_css_class("alles-tips-head");
    let title = controls::label(t("Zoektips", "Search tips"), "alles-tips-title", false);
    title.set_hexpand(true);
    head.append(&title);
    let close = gtk::Button::new();
    close.add_css_class("w11-icon-button");
    close.set_child(Some(&kit::icons::glyph("close", 14)));
    close.set_size_request(32, 32);
    close.set_valign(gtk::Align::Center);
    close.set_tooltip_text(Some(t("Sluiten", "Close")));
    close.set_widget_name("alles-tips-close");
    close.connect_clicked(move |_| on_close());
    head.append(&close);
    root.append(&head);
    let scroll_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scroll_body.add_css_class("alles-tips-scroll");
    scroll_body.append(&controls::label(
        t(
            "Typ gewoon een deel van een naam. Met deze tekens en woorden zoek je nog preciezer. Klik op een voorbeeld om het te proberen.",
            "Just type part of a name. These characters and words make your search more precise. Click an example to try it.",
        ),
        "alles-tips-intro",
        true,
    ));
    for (gnl, gen_, rows) in TIP_GROUPS.iter() {
        let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
        group.add_css_class("alles-tips-group");
        group.append(&controls::label(t(gnl, gen_), "alles-tips-heading", true));
        for (i, (exnl, exen, dnl, den)) in rows.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            row.add_css_class("alles-tips-row");
            if i == 0 {
                row.add_css_class("alles-tips-first");
            }
            let q = t(exnl, exen).to_owned();
            let chip = gtk::Button::with_label(&q);
            chip.add_css_class("alles-chip");
            chip.add_css_class("alles-chip-code");
            chip.set_halign(gtk::Align::Start);
            if let Some(l) = chip.child().and_downcast::<gtk::Label>() {
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            let f = on_example.clone();
            chip.connect_clicked(move |_| f(&q));
            row.append(&chip);
            row.append(&controls::label(t(dnl, den), "alles-tips-desc", true));
            group.append(&row);
        }
        scroll_body.append(&group);
    }
    let keys = gtk::Box::new(gtk::Orientation::Vertical, 0);
    keys.add_css_class("alles-tips-group");
    keys.append(&controls::label(t("Sneltoetsen", "Keyboard shortcuts"), "alles-tips-heading", true));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for (k, dnl, den) in KEYS {
        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let dt = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        dt.set_size_request(120, -1);
        for (i, part) in k.split('+').enumerate() {
            if i > 0 {
                dt.append(&controls::label("+", "alles-kbd-plus", false));
            }
            let kbd = gtk::Label::new(Some(part));
            kbd.add_css_class("alles-kbd");
            kbd.set_valign(gtk::Align::Center);
            dt.append(&kbd);
        }
        r.append(&dt);
        let dd = controls::label(t(dnl, den), "alles-tips-dd", true);
        dd.set_hexpand(true);
        r.append(&dd);
        list.append(&r);
    }
    keys.append(&list);
    scroll_body.append(&keys);
    let sc = gtk::ScrolledWindow::new();
    sc.set_hscrollbar_policy(gtk::PolicyType::Never);
    sc.set_vexpand(true);
    sc.set_child(Some(&scroll_body));
    root.append(&sc);
    root.upcast()
}
