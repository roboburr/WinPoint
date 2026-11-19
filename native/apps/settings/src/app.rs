//! The Settings window and its router (6ebf169f:vendor/ui/js/apps/settings/main.js): a Mica window with Back and the
//! title in a 48 px title bar, the 280 px navigation pane (user card, "Zoek een instelling", the colourful
//! categories) and the content: the page title or breadcrumb (fixed) above a scrolling column of cards.
//! Deep links (`--params {"page": ..}`, a relaunch), Back (history of 50), the search results page, the live
//! language switch and the automatic accent colour from the wallpaper live here.
use crate::common;
use crate::navicons;
use crate::pages::{self, Content, Page};
use crate::registry;
use crate::widgets::{self, Clamp};
use kit::controls::{self as c, Breadcrumb, NavItem, NavView, ProgressRing, SearchBox, SettingsCard, SettingsGroup, Suggestion};
use kit::frame::{AppWindow, WindowOptions};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// How `show` presents a page (main.js show options).
#[derive(Clone, Copy, Debug)]
pub struct Show {
    /// Put the current page on the Back history.
    pub push: bool,
    /// The enter animation.
    pub animated: bool,
    /// Keep the view and its scroll position, swapping the body when the new one is ready (a refresh).
    pub keep_scroll: bool,
}

impl Default for Show {
    fn default() -> Show {
        Show { push: true, animated: true, keep_scroll: false }
    }
}

enum Title {
    None,
    Label(gtk::Label),
    /// The breadcrumb and its (id, label) items: `set_title_text` replaces the last label.
    Crumb(Breadcrumb, Vec<(String, String)>),
}

/// The breadcrumb items with the last label replaced by `text` (a data page's item name, `Page::set_title`).
pub(crate) fn with_last_label(items: &[(String, String)], text: &str) -> Vec<(String, String)> {
    let mut out = items.to_vec();
    if let Some(last) = out.last_mut() {
        last.1 = text.to_owned();
    }
    out
}

fn set_crumbs(bc: &Breadcrumb, items: &[(String, String)]) {
    let refs: Vec<(&str, &str)> = items.iter().map(|(id, label)| (id.as_str(), label.as_str())).collect();
    bc.set_items(&refs);
}

struct View {
    root: gtk::Box,
    head: Clamp,
    head_box: gtk::Box,
    title: RefCell<Title>,
    page: Clamp,
    body: gtk::Box,
}

/// One-shot parameters and the window's own do not make the same page a new history entry (main.js ONE_SHOT).
const ONE_SHOT: &[&str] = &["add", "surface", "app", "token", "theme", "lang", "debug"];

fn hist_key(params: &Value) -> String {
    let mut pairs: Vec<(String, String)> = params
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, _)| !ONE_SHOT.contains(&k.as_str()) && !k.starts_with("mock"))
                .map(|(k, v)| (k.clone(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    pairs.sort();
    serde_json::to_string(&pairs).unwrap_or_default()
}

struct User {
    avatar: gtk::Box,
    name: gtk::Label,
    kind: gtk::Label,
    real: RefCell<String>,
    image: RefCell<Option<String>>,
    /// AccountsService answered (users.self): system.info's user, the fallback, no longer overrides it
    accounts: Cell<bool>,
}

pub struct Router {
    win: AppWindow,
    nav: NavView,
    search: SearchBox,
    user: User,
    main: gtk::Box,
    view: RefCell<Option<Rc<View>>>,
    hist: RefCell<Vec<(&'static str, Value)>>,
    current: RefCell<Option<(&'static str, Value)>>,
    page: RefCell<Option<Page>>,
    seq: Cell<u64>,
    narrow: Cell<bool>,
    accent_timer: RefCell<Option<glib::SourceId>>,
    me: RefCell<Weak<Router>>,
}

impl Router {
    /// The sequence number of the page shown now (a Page is alive while its number is this).
    pub fn seq(&self) -> u64 {
        self.seq.get()
    }

    fn weak(&self) -> Weak<Router> {
        self.me.borrow().clone()
    }

    /// The window.
    pub fn window(&self) -> &AppWindow {
        &self.win
    }

    /// Show a page (main.js show): `raw` is an id, alias or "search".
    pub fn show(&self, raw: &str, params: Value, opts: Show) {
        let id = registry::resolve(raw).unwrap_or("home");
        let params = if params.is_object() { params } else { json!({}) };
        {
            let cur = self.current.borrow().clone();
            let same = cur.as_ref().is_some_and(|(cid, cp)| *cid == id && hist_key(cp) == hist_key(&params));
            if opts.push
                && let Some(c) = cur
                && !same
            {
                let mut h = self.hist.borrow_mut();
                h.push(c);
                if h.len() > 50 {
                    h.remove(0);
                }
            }
        }
        *self.current.borrow_mut() = Some((id, params.clone()));
        self.win.set_back_enabled(!self.hist.borrow().is_empty());
        if id != "search" {
            self.nav.select(registry::top_of(id));
            common::record_visit(id);
        } else {
            // no category is selected while showing search results
            self.nav.select("__search__");
        }
        self.update_window_title();

        // the old page goes
        if let Some(old) = self.page.borrow_mut().take() {
            old.finish();
        }
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        let page = Page::new(id, params.clone(), seq, self.weak(), self.win.clone());
        *self.page.borrow_mut() = Some(page.clone());

        let reuse = if opts.keep_scroll { self.view.borrow().clone() } else { None };
        let view = match reuse {
            Some(v) => {
                self.set_header(&v, id);
                v
            }
            None => {
                let v = self.build_view(id);
                if opts.animated {
                    // the title shows at once (like Windows); the body follows when its data is in
                    self.set_view(&v);
                    widgets::enter(&v.head, 12.0, 167);
                }
                v
            }
        };
        let immediate = opts.animated && !opts.keep_scroll;

        // a spinner when the page takes a moment
        let ready = Rc::new(Cell::new(false));
        let weak = self.weak();
        let v2 = view.clone();
        let ready2 = ready.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(180), move || {
            let Some(r) = weak.upgrade() else { return };
            if r.seq() != seq || ready2.get() {
                return;
            }
            if !opts.keep_scroll {
                c::clear(&v2.body);
                let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
                wrap.add_css_class("st-loading");
                let ring = ProgressRing::new(32, None);
                ring.widget().set_halign(gtk::Align::Center);
                ring.widget().set_tooltip_text(Some(common::ct::loading()));
                wrap.append(ring.widget());
                v2.body.append(&wrap);
            }
            r.set_view(&v2);
        });

        let weak = self.weak();
        glib::spawn_future_local(async move {
            let content: Content = if id == "search" {
                search_results(&page)
            } else {
                match pages::builder(id) {
                    Some(b) => b(page.clone()).await,
                    None => failed(&page),
                }
            };
            let Some(r) = weak.upgrade() else { return };
            if r.seq() != seq {
                return;
            }
            ready.set(true);
            c::clear(&view.body);
            for w in content {
                view.body.append(&w);
            }
            common::space(&view.body);
            r.set_view(&view);
            if opts.animated {
                if !immediate {
                    widgets::enter(&view.head, 12.0, 167);
                }
                widgets::enter(&view.page, 20.0, 250);
            }
        });
    }

    /// Build the page again in place (ctx.refresh): no history entry, no animation, the scroll kept.
    pub fn refresh(&self, id: &'static str, params: Value) {
        self.show(id, params, Show { push: false, animated: false, keep_scroll: true });
    }

    /// Back (frame.onBack).
    fn back(&self) {
        let prev = self.hist.borrow_mut().pop();
        if let Some((id, params)) = prev {
            self.show(id, params, Show { push: false, ..Show::default() });
        }
        self.win.set_back_enabled(!self.hist.borrow().is_empty());
    }

    /// The page shown now.
    fn current_id(&self) -> Option<&'static str> {
        self.current.borrow().as_ref().map(|(id, _)| *id)
    }

    /// The page shown now and its parameters (main.rs: the page a restart reopens).
    pub fn current(&self) -> Option<(&'static str, Value)> {
        self.current.borrow().clone()
    }

    fn update_window_title(&self) {
        let app = t("Instellingen", "Settings");
        let page = match self.current_id() {
            Some("search") => t("Zoekresultaten", "Search results"),
            Some(id) => registry::page(id).map(|p| p.title()).unwrap_or(""),
            None => "",
        };
        self.win.set_title(app);
        if !page.is_empty() {
            self.win.window().set_title(Some(&format!("{page} – {app}")));
        }
    }

    fn build_view(&self, id: &str) -> Rc<View> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("st-view");
        if self.narrow.get() {
            root.add_css_class("st-narrow");
        }
        let head_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        head_box.add_css_class("st-head");
        let head = Clamp::new(&head_box, 1080);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 22);
        body.add_css_class("st-body");
        let page_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page_box.add_css_class("st-page");
        page_box.append(&body);
        let page = Clamp::new(&page_box, 1080);
        let scroll = c::scrolled(&page);
        scroll.add_css_class("st-scroll");
        root.append(&head);
        root.append(&scroll);
        let weak = self.weak();
        page.connect_width(move |_| {
            // settings.css @media (max-width: 760px): the window's width
            if let Some(r) = weak.upgrade() {
                let w = r.win.window().width();
                r.set_narrow(w > 0 && w < 760);
            }
        });
        let v = Rc::new(View { root, head, head_box, title: RefCell::new(Title::None), page, body });
        self.set_header(&v, id);
        v
    }

    /// The fixed header: the title, or the breadcrumb of the parent chain, "Netwerk en internet › Geavanceerde
    /// netwerkinstellingen › Geavanceerde instellingen voor delen" (main.js header, N levels).
    fn set_header(&self, v: &View, id: &str) {
        c::clear(&v.head_box);
        let info = registry::page(id);
        let chain = registry::ancestors(id);
        if chain.len() > 1 {
            let bc = Breadcrumb::new(true);
            let items: Vec<(String, String)> = chain.iter().map(|p| (p.id.to_owned(), p.title().to_owned())).collect();
            set_crumbs(&bc, &items);
            bc.widget().add_css_class("st-title");
            let weak = self.weak();
            bc.connect_selected(move |pid, _| {
                if let Some(r) = weak.upgrade() {
                    let pid = pid.to_owned();
                    glib::idle_add_local_once(move || r.show(&pid, json!({}), Show::default()));
                }
            });
            v.head_box.append(bc.widget());
            *v.title.borrow_mut() = Title::Crumb(bc, items);
        } else {
            let text = if id == "search" { t("Zoekresultaten", "Search results") } else { info.map(|p| p.title()).unwrap_or("") };
            let l = c::label(text, "w11-title st-title", true);
            v.head_box.append(&l);
            *v.title.borrow_mut() = Title::Label(l);
        }
    }

    /// ctx.setTitle: replace the title text, on a sub-page the last crumb (a data page's item name: "Netwerk en
    /// internet › Wi-Fi › Thuis").
    pub fn set_title_text(&self, text: &str) {
        let Some(v) = self.view.borrow().clone() else { return };
        let mut title = v.title.borrow_mut();
        match &mut *title {
            Title::Label(l) => l.set_label(text),
            Title::Crumb(bc, items) => {
                *items = with_last_label(items, text);
                set_crumbs(bc, items);
            }
            Title::None => {}
        }
    }

    fn set_view(&self, v: &Rc<View>) {
        let shown = self.view.borrow().as_ref().is_some_and(|cur| Rc::ptr_eq(cur, v)) && v.root.parent().is_some();
        if shown {
            return;
        }
        c::clear(&self.main);
        v.root.set_vexpand(true);
        self.main.append(&v.root);
        *self.view.borrow_mut() = Some(v.clone());
    }

    fn set_narrow(&self, narrow: bool) {
        if self.narrow.replace(narrow) == narrow {
            return;
        }
        if let Some(v) = self.view.borrow().as_ref() {
            if narrow {
                v.root.add_css_class("st-narrow");
            } else {
                v.root.remove_css_class("st-narrow");
            }
        }
    }

    fn settings_changed(&self, s: &Value, path: Option<&str>) {
        let page = self.page.borrow().clone();
        if let Some(p) = page {
            p.dispatch_settings(s, path);
        }
        // automatic accent colour from the wallpaper
        if s["theme"]["accentFromWallpaper"] != true {
            return;
        }
        let relevant = match path {
            None => true,
            Some(p) => p.starts_with("wallpaper") || p == "theme.mode" || p == "theme.accentFromWallpaper",
        };
        if !relevant {
            return;
        }
        if let Some(id) = self.accent_timer.borrow_mut().take() {
            id.remove();
        }
        let weak = self.weak();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
            if let Some(r) = weak.upgrade() {
                r.accent_timer.borrow_mut().take();
                auto_accent();
            }
        });
        *self.accent_timer.borrow_mut() = Some(id);
    }

    // ---- the user card

    /// The card from AccountsService, as Je info shows the account (users.self: the name and the picture; Windows'
    /// second line "Lokaal account"); asked at the start and again after every users.changed. Without
    /// AccountsService the card keeps system.info's user.
    fn load_user(&self) {
        let weak = self.weak();
        kit::ctx().call("users.self", json!({}), move |r| {
            let (Some(r), Ok(u)) = (weak.upgrade(), r) else { return };
            if u["available"] != true {
                return;
            }
            let name = u["realName"].as_str().filter(|s| !s.trim().is_empty()).or(u["userName"].as_str()).unwrap_or("");
            if name.is_empty() {
                return;
            }
            r.user.accounts.set(true);
            *r.user.real.borrow_mut() = name.to_owned();
            *r.user.image.borrow_mut() = u["iconFile"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
            r.render_user();
        });
    }

    fn render_user(&self) {
        let real = self.user.real.borrow().clone();
        let name = if real.is_empty() { t("Gebruiker", "User").to_owned() } else { real };
        c::clear(&self.user.avatar);
        self.user.avatar.append(&common::avatar(&name, self.user.image.borrow().as_deref(), 60));
        self.user.name.set_label(&name);
        self.user.kind.set_label(t("Lokaal account", "Local account"));
    }

    fn relabel(&self) {
        for id in registry::top_ids() {
            if let Some(p) = registry::page(id) {
                self.nav.set_label(id, p.title());
            }
        }
        self.search.set_placeholder(t("Zoek een instelling", "Find a setting"));
        self.render_user();
        self.update_window_title();
        let cur = self.current.borrow().clone();
        if let Some((id, params)) = cur {
            if id != "search" {
                self.nav.select(registry::top_of(id));
            }
            self.refresh(id, params);
        }
    }
}

/// The automatic accent colour (main.js autoAccent): the wallpaper's dominant tone, or the solid colour.
fn auto_accent() {
    let ctx = kit::ctx();
    let s = ctx.settings();
    if s["theme"]["accentFromWallpaper"] != true {
        return;
    }
    let path = common::resolve_wallpaper_path(&s, kit::theme::dark());
    let apply = |hex: Option<String>| {
        let Some(hex) = hex else { return };
        let ctx = kit::ctx();
        let cur = ctx.settings()["theme"]["accent"].as_str().unwrap_or("").to_lowercase();
        if hex.to_lowercase() != cur {
            ctx.set_setting_then("theme.accent", json!(hex), |r| {
                if let Err(e) = r {
                    eprintln!("win11-gtk-settings: auto accent failed: {e}");
                }
            });
        }
    };
    if path.is_empty() {
        apply(s["wallpaper"]["color"].as_str().and_then(kit::theme::normalize_hex));
    } else {
        common::accent_from_image(&path, apply);
    }
}

/// The search results page (main.js renderSearch).
fn search_results(p: &Page) -> Content {
    let q = p.param("q").unwrap_or_default();
    let hits = registry::search_visible(&q, kit::i18n::en(), 40, &common::page_visible);
    let lead = c::label(&common::fill(t("Resultaten voor “{q}”", "Results for “{q}”"), &[("q", &q)]), "st-lead", true);
    if hits.is_empty() {
        let e = c::empty_state(
            "search",
            t("Geen resultaten", "No results"),
            t(
                "Probeer een ander woord, bijvoorbeeld “achtergrond”, “geluid” of “wifi”.",
                "Try another word, for example “background”, “sound” or “wifi”.",
            ),
            None,
        );
        return vec![lead.upcast(), e.upcast()];
    }
    let group = SettingsGroup::new("", "");
    for page in hits {
        let chain = registry::ancestors(page.id);
        let prefix: String = chain[..chain.len().saturating_sub(1)].iter().map(|pp| format!("{} › ", pp.title())).collect();
        let card = SettingsCard::clickable(page.icon, page.title(), &format!("{prefix}{}", page.description()), c::Chevron::Right);
        if let Some(b) = card.button() {
            b.set_widget_name(&format!("result-{}", page.id));
        }
        let (pp, id) = (p.clone(), page.id);
        card.connect_clicked(move || pp.go(id));
        group.add(card.widget());
    }
    vec![lead.upcast(), group.widget().clone().upcast()]
}

/// "Deze pagina kon niet worden geladen" with Opnieuw proberen.
fn failed(p: &Page) -> Content {
    let retry = c::button(common::ct::retry(), c::Variant::Standard);
    let pp = p.clone();
    retry.connect_clicked(move |_| pp.refresh());
    let e = c::empty_state(
        "warning",
        t("Deze pagina kon niet worden geladen", "This page couldn’t be loaded"),
        t("Er ging iets mis. Probeer het opnieuw.", "Something went wrong. Try again."),
        Some(retry.upcast_ref()),
    );
    vec![e.upcast()]
}

/// Build the window and show the first page.
pub fn build(start: &str, params: Value) -> Rc<Router> {
    let win = AppWindow::new(WindowOptions {
        key: "main".into(),
        title: t("Instellingen", "Settings").into(),
        icon: "settings".into(),
        width: 1100,
        height: 740,
        min_width: 640,
        min_height: 480,
        titlebar_height: 48,
        back: true,
    });
    win.window().add_css_class("st-app");

    // ---- navigation pane header: user card and search
    let avatar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    avatar.add_css_class("st-user-avatar");
    avatar.set_valign(gtk::Align::Center);
    let name = c::label("", "st-user-name", false);
    let kind = c::label("", "st-user-kind", false);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    text.append(&name);
    text.append(&kind);
    let card_box = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    card_box.append(&avatar);
    card_box.append(&text);
    let user_card = gtk::Button::new();
    user_card.add_css_class("st-user");
    user_card.set_widget_name("settings-user");
    user_card.set_child(Some(&card_box));
    let search = SearchBox::new(Some(t("Zoek een instelling", "Find a setting")));
    search.widget().add_css_class("st-search");
    search.widget().set_hexpand(true);
    search.text_widget().set_widget_name("settings-search");
    search.set_debounce(150);
    let navhead = gtk::Box::new(gtk::Orientation::Vertical, 14);
    navhead.add_css_class("st-navhead");
    navhead.append(&user_card);
    navhead.append(search.widget());

    let items: Vec<NavItem> = registry::top_ids()
        .into_iter()
        .filter_map(registry::page)
        .map(|p| NavItem::new(p.id, p.title(), ""))
        .collect();
    let nav = NavView::new(items);
    nav.widget().set_widget_name("settings-nav");
    nav.set_header(Some(&navhead));
    nav.set_compact_below(760);
    for id in registry::top_ids() {
        // the colourful category icons in the item's 16 px icon slot, 14 px before the label (settings.css)
        if let Some(b) = nav.item_button(id)
            && let Some(body) = b.child().and_downcast::<gtk::Overlay>().and_then(|o| o.child()).and_downcast::<gtk::Box>()
        {
            body.set_spacing(14);
            if let Some(slot) = body.first_child().and_downcast::<gtk::Box>() {
                slot.append(&navicons::cat_icon(id, 16));
            }
        }
    }

    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.add_css_class("st-main");
    main.set_hexpand(true);
    main.set_vexpand(true);
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.add_css_class("st-frame-body");
    body.append(nav.widget());
    body.append(&main);
    win.set_content(&body);

    let router = Rc::new(Router {
        win: win.clone(),
        nav: nav.clone(),
        search: search.clone(),
        user: User { avatar, name, kind, real: RefCell::new(String::new()), image: RefCell::new(None), accounts: Cell::new(false) },
        main,
        view: RefCell::new(None),
        hist: RefCell::new(Vec::new()),
        current: RefCell::new(None),
        page: RefCell::new(None),
        seq: Cell::new(0),
        narrow: Cell::new(false),
        accent_timer: RefCell::new(None),
        me: RefCell::new(Weak::new()),
    });
    *router.me.borrow_mut() = Rc::downgrade(&router);
    let info = common::shell_info();
    let initial = info["user"]["realName"].as_str().filter(|s| !s.is_empty()).or(info["user"]["name"].as_str()).unwrap_or("");
    *router.user.real.borrow_mut() = initial.to_owned();
    router.render_user();
    let weak = Rc::downgrade(&router);
    kit::ctx().call("system.info", json!({}), move |r| {
        let (Some(r), Ok(si)) = (weak.upgrade(), r) else { return };
        let u = &si["user"];
        if !u.is_object() || r.user.accounts.get() {
            return;
        }
        let name = u["realName"].as_str().filter(|s| !s.is_empty()).or(u["name"].as_str()).unwrap_or("").to_owned();
        if !name.is_empty() {
            *r.user.real.borrow_mut() = name;
        }
        *r.user.image.borrow_mut() = u["avatar"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        r.render_user();
    });
    router.load_user();
    let weak = Rc::downgrade(&router);
    // a rename or a new picture in Je info (or in GNOME Settings, or another program), as Windows' pane follows it
    kit::ctx().on("users.changed", move |_, _| {
        if let Some(r) = weak.upgrade() {
            r.load_user();
        }
    });

    // ---- behaviour
    let weak = Rc::downgrade(&router);
    user_card.connect_clicked(move |_| {
        if let Some(r) = weak.upgrade() {
            r.show("accounts", json!({}), Show::default());
        }
    });
    let weak = Rc::downgrade(&router);
    search.connect_changed(move |q| {
        let Some(r) = weak.upgrade() else { return };
        if q.trim().is_empty() {
            r.search.set_suggestions(&[], false);
            return;
        }
        let list: Vec<Suggestion> = registry::search_visible(q, kit::i18n::en(), 8, &common::page_visible)
            .into_iter()
            .map(|p| Suggestion {
                value: p.id.to_owned(),
                label: p.title().to_owned(),
                description: p.parent.and_then(registry::page).map(|pp| pp.title().to_owned()).unwrap_or_default(),
                glyph: p.icon.to_owned(),
            })
            .collect();
        r.search.set_suggestions(&list, true);
    });
    let weak = Rc::downgrade(&router);
    search.connect_suggestion(move |id| {
        if let Some(r) = weak.upgrade() {
            r.search.set_text("");
            r.show(id, json!({}), Show::default());
        }
    });
    let weak = Rc::downgrade(&router);
    search.connect_submit(move |q| {
        if let Some(r) = weak.upgrade()
            && !q.trim().is_empty()
        {
            r.show("search", json!({ "q": q.trim() }), Show::default());
        }
    });
    // navigation pane: top-level pages
    let weak = Rc::downgrade(&router);
    nav.connect_selected(move |id| {
        if let Some(r) = weak.upgrade()
            && r.current_id() != Some(id)
        {
            r.show(id, json!({}), Show::default());
        }
    });
    // clicking the selected category while on one of its sub-pages returns to the category page
    for id in registry::top_ids() {
        if let Some(b) = nav.item_button(id) {
            let weak = Rc::downgrade(&router);
            b.connect_clicked(move |_| {
                if let Some(r) = weak.upgrade()
                    && let Some(cur) = r.current_id()
                    && cur != id
                    && registry::top_of(cur) == id
                {
                    r.show(id, json!({}), Show::default());
                }
            });
        }
    }
    let weak = Rc::downgrade(&router);
    win.connect_back(move || {
        if let Some(r) = weak.upgrade() {
            r.back();
        }
    });
    // Ctrl+F / F3: the search box (like Windows Settings)
    for accel in ["<Control>f", "F3"] {
        let s = search.clone();
        win.add_shortcut(accel, move || {
            s.focus();
            s.text_widget().select_region(0, -1);
            true
        });
    }
    // live language switch
    let weak = Rc::downgrade(&router);
    kit::i18n::on_change(move || {
        if let Some(r) = weak.upgrade() {
            r.relabel();
        }
    });
    // settings changes: the page's listeners, the automatic accent colour
    let weak = Rc::downgrade(&router);
    kit::ctx().on_settings(move |s, path| {
        if let Some(r) = weak.upgrade() {
            r.settings_changed(s, path);
        }
    });

    let start_params = if start == "search" { json!({ "q": params["q"].as_str().unwrap_or("") }) } else { params };
    router.show(start, start_params, Show { push: false, animated: false, keep_scroll: false });
    win.present();
    router
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_page_names_its_item_in_the_last_crumb() {
        // review of B0, finding 7: Page::set_title on a sub-page (every data page) replaces the last crumb
        let chain: Vec<(String, String)> = registry::ancestors("wifi-network").iter().map(|p| (p.id.to_owned(), p.title().to_owned())).collect();
        assert_eq!(chain.len(), 3, "Netwerk en internet › Wi-Fi › Wi-Fi-eigenschappen");
        let named = with_last_label(&chain, "Thuis");
        assert_eq!(named.len(), 3);
        assert_eq!(named[..2], chain[..2], "the parents stay, with their ids");
        assert_eq!(named[2], ("wifi-network".to_owned(), "Thuis".to_owned()));
        assert!(with_last_label(&[], "x").is_empty());
    }
}
