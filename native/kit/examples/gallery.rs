//! The kit's gallery: one Windows 11 app window with every control, for screenshots of the look.
//!
//!   cargo build --example gallery          (target/debug/examples/gallery)
//!   gallery [--light] [--en] [--backend]
//!
//! --light: the light theme (default dark), --en: English (default Dutch), --backend: start the backend as an
//! app would (default: none, so the gallery runs anywhere with a display). WIN11_THEME / WIN11_LANG work too.
//! For screenshots: --section buttons|toggles|inputs|cards|tabs|feedback|table|charts|misc shows that section
//! only, --open dialog opens the content dialog at start.
use kit::controls::{self as c, MenuItem, TopAction, Variant};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let light = args.iter().any(|a| a == "--light");
    let english = args.iter().any(|a| a == "--en");
    let backend = args.iter().any(|a| a == "--backend");
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let section = value("--section");
    let open = value("--open");
    let mut spec = kit::AppSpec::new("gallery", "io.github.win11shell.KitGallery", "Galerij", "Gallery")
        .icon("grid")
        .usage("gallery [--light] [--en] [--backend] [--section ID] [--open dialog]");
    if !backend {
        spec = spec.without_backend();
    }
    kit::run(spec, move |_ctx, _args, first| {
        if !first {
            if let Some(w) = kit::frame::AppWindow::active() {
                w.present();
            }
            return;
        }
        kit::theme::set_forced_dark(Some(!light));
        if english {
            kit::i18n::set_forced_lang(Some("en"));
        }
        build(section.clone(), open.clone());
    })
}

fn section(title: &str) -> (gtk::Box, gtk::Box) {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_margin_bottom(28);
    let l = c::label(title, "w11-subtitle", false);
    l.set_margin_bottom(4);
    b.append(&l);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.append(&body);
    (b, body)
}

fn row() -> gtk::Box {
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    r.set_halign(gtk::Align::Start);
    r
}

fn build(only: Option<String>, open: Option<String>) {
    let win = kit::frame::AppWindow::new(kit::frame::WindowOptions {
        key: "gallery".into(),
        width: 1280,
        height: 900,
        back: true,
        ..Default::default()
    });
    let search = c::SearchBox::new(None);
    search.widget().set_size_request(360, -1);
    search.widget().set_halign(gtk::Align::Center);
    search.connect_changed({
        let s = search.clone();
        move |text| {
            let items: Vec<c::Suggestion> = kit::icons::glyph_names()
                .into_iter()
                .filter(|n| !text.is_empty() && n.contains(text))
                .take(8)
                .map(|n| c::Suggestion { value: n.into(), label: n.into(), description: t("Glyph", "Glyph").into(), glyph: n.into() })
                .collect();
            s.set_suggestions(&items, !text.is_empty());
        }
    });
    win.set_titlebar_content(Some(search.widget()));

    let nav = c::NavView::new(vec![
        c::NavItem::new("buttons", t("Knoppen", "Buttons"), "app"),
        c::NavItem::new("toggles", t("Schakelaars", "Toggles"), "check"),
        c::NavItem::new("inputs", t("Invoer", "Input"), "edit"),
        c::NavItem::new("cards", t("Instellingenkaarten", "Settings cards"), "settings").children(vec![
            c::NavItem::new("cards-plain", t("Gewoon", "Plain"), "document"),
            c::NavItem::new("cards-expander", t("Uitvouwen", "Expander"), "chevron-down"),
        ]),
        c::NavItem::separator(),
        c::NavItem::heading(t("Meer", "More")),
        c::NavItem::new("feedback", t("Meldingen", "Feedback"), "info").badge("3"),
        c::NavItem::new("table", t("Tabel", "Table"), "list"),
        c::NavItem::new("charts", t("Grafieken", "Charts"), "chart"),
        c::NavItem::new("misc", t("Overig", "Other"), "more"),
    ]);
    nav.set_compact_below(760);
    nav.select("buttons");

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("w11-page");
    page.append(&c::page_header(t("Galerij", "Gallery"), t("Alle bedieningselementen van win11-kit.", "Every control of win11-kit.")));
    let crumbs = c::Breadcrumb::new(true);
    crumbs.set_items(&[("home", t("Systeem", "System")), ("display", t("Beeldscherm", "Display")), ("scale", t("Schaal", "Scale"))]);
    crumbs.widget().set_margin_bottom(20);
    page.append(crumbs.widget());

    let mut anchors: Vec<(&'static str, gtk::Box)> = Vec::new();

    // ---- buttons
    let (s, body) = section(t("Knoppen", "Buttons"));
    let r = row();
    r.append(&c::button(t("Standaard", "Standard"), Variant::Standard));
    r.append(&c::button(t("Accent", "Accent"), Variant::Accent));
    r.append(&c::button(t("Subtiel", "Subtle"), Variant::Subtle));
    r.append(&c::button(t("Verwijderen", "Delete"), Variant::Danger));
    let dis = c::button(t("Uitgeschakeld", "Disabled"), Variant::Standard);
    dis.set_sensitive(false);
    r.append(&dis);
    body.append(&r);
    let r = row();
    let small = c::button(t("Klein", "Small"), Variant::Standard);
    small.add_css_class("w11-small");
    r.append(&small);
    let large = c::button(t("Groot", "Large"), Variant::Accent);
    large.add_css_class("w11-large");
    r.append(&large);
    r.append(&c::button_with_glyph("folder-open", t("Openen", "Open"), Variant::Standard));
    r.append(&c::button_with_glyph("refresh", "", Variant::Standard));
    r.append(&c::icon_button("more", t("Meer", "More"), 32));
    r.append(&c::icon_button("settings", t("Instellingen", "Settings"), 32));
    r.append(&c::hyperlink(t("Meer informatie", "Learn more"), Some("external")));
    body.append(&r);
    let r = row();
    let dd = c::DropDownButton::new(Some("sort"), t("Sorteren", "Sort"), Variant::Standard, || {
        vec![
            MenuItem::new(t("Naam", "Name"), || {}).radio(true),
            MenuItem::new(t("Gewijzigd op", "Date modified"), || {}).radio(false),
            MenuItem::separator(),
            MenuItem::new(t("Oplopend", "Ascending"), || {}).checked(true),
        ]
    });
    r.append(dd.widget());
    let split = c::SplitButton::new(Some("new"), t("Nieuw", "New"), Variant::Accent, || {
        vec![MenuItem::new(t("Map", "Folder"), || {}).glyph("folder"), MenuItem::new(t("Tekstdocument", "Text document"), || {}).glyph("document")]
    });
    r.append(split.widget());
    body.append(&r);
    page.append(&s);
    anchors.push(("buttons", s));

    // ---- toggles
    let (s, body) = section(t("Schakelaars", "Toggles"));
    let r = row();
    r.set_spacing(24);
    let sw = c::ToggleSwitch::new(true);
    sw.switch().set_widget_name("gallery-switch");
    r.append(sw.widget());
    r.append(c::ToggleSwitch::new(false).widget());
    let sd = c::ToggleSwitch::new(true);
    sd.set_sensitive(false);
    r.append(sd.widget());
    body.append(&r);
    let r = row();
    r.set_spacing(24);
    let cb = c::checkbox(t("Bestandsextensies", "File name extensions"));
    cb.set_active(true);
    r.append(&cb);
    r.append(&c::checkbox(t("Verborgen items", "Hidden items")));
    let cbd = c::checkbox(t("Uitgeschakeld", "Disabled"));
    cbd.set_sensitive(false);
    r.append(&cbd);
    r.append(&c::checkbox_desc(t("Met uitleg", "With a description"), t("Een tweede regel onder het label.", "A second line below the label.")));
    body.append(&r);
    let r = row();
    r.set_spacing(48);
    let rg = c::RadioGroup::new(&[("light", t("Licht", "Light")), ("dark", t("Donker", "Dark")), ("custom", t("Aangepast", "Custom"))], Some("dark"), Some(t("Kies je modus", "Choose your mode")), false);
    r.append(rg.widget());
    let rh = c::RadioGroup::new(&[("left", t("Links", "Left")), ("center", t("Midden", "Centre"))], Some("center"), Some(t("Uitlijning", "Alignment")), true);
    r.append(rh.widget());
    body.append(&r);
    page.append(&s);
    anchors.push(("toggles", s));

    // ---- inputs
    let (s, body) = section(t("Invoer", "Input"));
    let r = row();
    r.set_spacing(16);
    let tb = c::text_box(t("Naam van de map", "Folder name"));
    tb.set_size_request(240, -1);
    r.append(&c::field(t("Naam", "Name"), &tb, Some(t("Gebruik geen / of \\.", "Don't use / or \\."))));
    let nb = c::number_box(0.0, 100.0, 1.0, 42.0, 0);
    r.append(&c::field(t("Aantal", "Count"), &nb, None));
    let dropdown = c::Dropdown::new(
        vec![
            c::DropdownOption::new("100", "100%").description(t("Aanbevolen", "Recommended")),
            c::DropdownOption::new("125", "125%"),
            c::DropdownOption::new("150", "150%"),
            c::DropdownOption::new("files", t("Bestanden", "Files")).glyph("folder"),
        ],
        Some("125"),
    );
    r.append(&c::field(t("Schaal", "Scale"), dropdown.widget(), None));
    let empty_dd = c::Dropdown::simple(&[("a", "A"), ("b", "B")], None);
    r.append(&c::field(t("Leeg", "Empty"), empty_dd.widget(), None));
    body.append(&r);
    let slider = c::Slider::new(0.0, 100.0, 1.0, 60.0);
    slider.set_format(|v| kit::format::percent(v, 0));
    slider.widget().set_size_request(320, -1);
    slider.widget().set_halign(gtk::Align::Start);
    body.append(&c::field(t("Helderheid", "Brightness"), slider.widget(), None));
    let (area, _tv) = c::text_area(80);
    area.set_size_request(420, -1);
    area.set_halign(gtk::Align::Start);
    body.append(&c::field(t("Notities", "Notes"), &area, None));
    page.append(&s);
    anchors.push(("inputs", s));

    // ---- cards
    let (s, body) = section(t("Instellingenkaarten", "Settings cards"));
    let group = c::SettingsGroup::new(t("Beeldscherm", "Display"), t("Helderheid, nachtlamp en meer", "Brightness, night light and more"));
    let card = c::SettingsCard::new("brightness", t("Nachtlamp", "Night light"), t("Gebruik warmere kleuren om je te helpen slapen", "Use warmer colours to help block blue light"));
    card.set_control(c::ToggleSwitch::new(false).widget());
    group.add(card.widget());
    let click = c::SettingsCard::clickable("display", t("Beeldscherm", "Display"), t("Monitoren, helderheid, nachtlamp, beeldschermprofiel", "Monitors, brightness, night light, display profile"), c::Chevron::Right);
    click.widget().set_widget_name("gallery-card");
    group.add(click.widget());
    let ext = c::SettingsCard::clickable("globe", t("Online hulp", "Online help"), "", c::Chevron::External);
    group.add(ext.widget());
    let valued = c::SettingsCard::clickable("storage", t("Opslag", "Storage"), "", c::Chevron::Right);
    valued.set_control(&c::label("412 GB", "", false));
    group.add(valued.widget());
    let exp = c::SettingsExpander::new("personalize", t("Kleuren", "Colours"), t("Accentkleur, transparantie-effecten", "Accent colour, transparency effects"));
    let mode = c::Dropdown::simple(&[("dark", t("Donker", "Dark")), ("light", t("Licht", "Light"))], Some("dark"));
    exp.header().set_control(mode.widget());
    let n1 = c::SettingsCard::new("", t("Transparantie-effecten", "Transparency effects"), "");
    n1.set_control(c::ToggleSwitch::new(true).widget());
    exp.add(n1.widget());
    let n2 = c::SettingsCard::clickable("", t("Accentkleur", "Accent colour"), "", c::Chevron::Right);
    exp.add(n2.widget());
    exp.add(c::ColorSwatches::new(None, Some("#0078d4"), 8, 40).widget());
    exp.set_expanded(true);
    group.add(exp.widget());
    let generic = c::Expander::new("info", t("Over dit apparaat", "About this device"), "");
    generic.set_content(&c::label(t("Processor: 8 kernen. Geheugen: 16 GB.", "Processor: 8 cores. Memory: 16 GB."), "", true));
    group.add(generic.widget());
    body.append(group.widget());
    page.append(&s);
    anchors.push(("cards", s));

    // ---- navigation bits
    let (s, body) = section(t("Tabbladen", "Tabs"));
    let pivot = c::Pivot::new(&[("all", t("Alles", "All"), ""), ("apps", "Apps", ""), ("docs", t("Documenten", "Documents"), ""), ("web", "Web", "")], Some("all"));
    body.append(pivot.widget());
    let seg = c::Segmented::new(&[("day", t("Dag", "Day"), "clock"), ("week", t("Week", "Week"), "calendar"), ("month", t("Maand", "Month"), "grid")], Some("week"));
    body.append(seg.widget());
    let bc = c::Breadcrumb::new(false);
    bc.set_items(&[("pc", t("Deze pc", "This PC")), ("home", "robin"), ("docs", t("Documenten", "Documents"))]);
    body.append(bc.widget());
    page.append(&s);
    anchors.push(("tabs", s));

    // ---- feedback
    let (s, body) = section(t("Meldingen", "Feedback"));
    let ib = c::InfoBar::new(c::Severity::Info, t("Bijwerken", "Update"), t("Er is een update beschikbaar.", "An update is available."));
    ib.set_action(Some((t("Nu installeren", "Install now"), Rc::new(|| {}))));
    body.append(ib.widget());
    body.append(c::InfoBar::new(c::Severity::Success, t("Klaar", "Done"), t("De schijf is geformatteerd.", "The disk was formatted.")).widget());
    body.append(c::InfoBar::new(c::Severity::Warning, t("Weinig ruimte", "Low space"), t("Er is nog 2 GB vrij op deze schijf.", "There is 2 GB left on this disk.")).widget());
    body.append(c::InfoBar::new(c::Severity::Error, t("Mislukt", "Failed"), t("De service reageert niet.", "The service doesn't respond.")).widget());
    let pr = row();
    pr.set_spacing(24);
    let p1 = c::ProgressBar::new();
    p1.set_fraction(0.4);
    p1.widget().set_size_request(200, -1);
    pr.append(p1.widget());
    let p2 = c::ProgressBar::new();
    p2.set_indeterminate(true);
    p2.widget().set_size_request(200, -1);
    pr.append(p2.widget());
    let p3 = c::ProgressBar::new();
    p3.set_fraction(0.7);
    p3.set_state(c::ProgressState::Paused);
    p3.widget().set_size_request(120, -1);
    pr.append(p3.widget());
    let p4 = c::ProgressBar::new();
    p4.set_fraction(0.3);
    p4.set_state(c::ProgressState::Error);
    p4.widget().set_size_request(120, -1);
    pr.append(p4.widget());
    pr.append(c::ProgressRing::new(32, Some(0.65)).widget());
    pr.append(c::ProgressRing::new(32, None).widget());
    body.append(&pr);
    let br = row();
    let dlg = c::button(t("Dialoogvenster", "Dialog"), Variant::Standard);
    dlg.set_widget_name("gallery-dialog");
    dlg.connect_clicked(|b| {
        let entry = c::text_box(t("Nieuwe naam", "New name"));
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.append(&c::label(t("Geef de map een nieuwe naam.", "Give the folder a new name."), "w11-dialog-text", true));
        body.append(&entry);
        c::Dialog::new(t("Naam wijzigen", "Rename"))
            .body(&body)
            .primary(t("Opslaan", "Save"))
            .secondary(t("Niet opslaan", "Don't save"))
            .show(b, |_| {});
    });
    br.append(&dlg);
    let danger = c::button(t("Bevestigen", "Confirm"), Variant::Standard);
    danger.connect_clicked(|b| {
        c::confirm(b, t("Permanent verwijderen?", "Delete permanently?"), t("Dit kan niet ongedaan worden gemaakt.", "This can't be undone."), t("Verwijderen", "Delete"), true, |_| {});
    });
    br.append(&danger);
    let toast = c::button(t("Melding", "Toast"), Variant::Standard);
    toast.connect_clicked(|b| {
        c::Toast::new(t("Gekopieerd", "Copied")).message(t("3 items naar het klembord.", "3 items to the clipboard.")).severity(c::Severity::Success).action(t("Ongedaan maken", "Undo"), || {}).show(b);
    });
    br.append(&toast);
    let menu_btn = c::button(t("Contextmenu", "Context menu"), Variant::Standard);
    menu_btn.connect_clicked(|b| {
        c::show_menu_with_top(
            b,
            8.0,
            b.height() as f64,
            vec![
                TopAction::new("cut", t("Knippen", "Cut"), || {}),
                TopAction::new("copy", t("Kopiëren", "Copy"), || {}),
                TopAction::new("rename", t("Naam wijzigen", "Rename"), || {}),
                TopAction::new("delete", t("Verwijderen", "Delete"), || {}).danger(),
            ],
            vec![
                MenuItem::new(t("Openen", "Open"), || {}).glyph("open").accel("Enter"),
                MenuItem::submenu(t("Openen met", "Open with"), vec![MenuItem::new("Firefox", || {}).glyph("globe"), MenuItem::new(t("Teksteditor", "Text Editor"), || {}).glyph("document")]).glyph("app"),
                MenuItem::separator(),
                MenuItem::header(t("Weergeven", "View")),
                MenuItem::new(t("Details", "Details"), || {}).glyph("list").radio(true),
                MenuItem::new(t("Grote pictogrammen", "Large icons"), || {}).glyph("grid").radio(false),
                MenuItem::separator(),
                MenuItem::new(t("Eigenschappen", "Properties"), || {}).glyph("properties").accel("Alt+Enter").description(t("Grootte, locatie, rechten", "Size, location, permissions")),
                MenuItem::new(t("Niet beschikbaar", "Unavailable"), || {}).glyph("lock").enabled(false),
            ],
        );
    });
    br.append(&menu_btn);
    body.append(&br);
    page.append(&s);
    anchors.push(("feedback", s));

    // ---- table
    let (s, body) = section(t("Tabel", "Table"));
    let table = kit::table::Table::new(
        vec![
            kit::table::Column::new("name", t("Naam", "Name")).expand().icon_text("icon", kit::table::IconKind::Glyph, 16),
            kit::table::Column::new("status", t("Status", "Status")).width(110),
            kit::table::Column::new("cpu", "CPU").width(90).align(kit::table::Align::End).format(|v| kit::format::percent(v["cpu"].as_f64().unwrap_or(0.0), 1)),
            kit::table::Column::new("mem", t("Geheugen", "Memory")).width(110).align(kit::table::Align::End).format(|v| kit::format::bytes(v["mem"].as_f64().unwrap_or(0.0))),
        ],
        "id",
        kit::table::Selection::Multi,
    );
    table.view().set_widget_name("gallery-table");
    table.widget().set_size_request(-1, 320);
    let make_rows = |seed: u64| -> Vec<Value> {
        let names = ["firefox", "nautilus", "gnome-shell", "win11-shell", "code", "pipewire", "Xwayland", "evolution", "tracker", "systemd"];
        let glyphs = ["globe", "folder", "desktop", "taskbar", "code", "sound", "window", "mail", "search", "settings"];
        (0..200u64)
            .map(|i| {
                let k = (i as usize) % names.len();
                let x = (i * 7919 + seed * 104729) % 1000;
                json!({"id": i, "name": format!("{} {}", names[k], i), "icon": glyphs[k], "status": if x % 9 == 0 { t("Opgeschort", "Suspended") } else { "" },
                    "cpu": (x % 250) as f64 / 10.0, "mem": ((x + 1) * 1_048_576 * 3) as f64})
            })
            .collect()
    };
    table.set_rows(make_rows(1));
    table.set_sort(Some("cpu"), false);
    table.connect_context_menu(|ids, _, _| {
        let n = ids.len();
        Some((Vec::new(), vec![MenuItem::new(&format!("{} ({n})", t("Taak beëindigen", "End task")), || {}).glyph("close"), MenuItem::new(t("Details", "Details"), || {}).glyph("info")]))
    });
    body.append(table.widget());
    let refresh = c::button_with_glyph("refresh", t("Vernieuwen", "Refresh"), Variant::Standard);
    refresh.set_halign(gtk::Align::Start);
    let seed = Rc::new(std::cell::Cell::new(1u64));
    let tb2 = table.clone();
    refresh.connect_clicked(move |_| {
        seed.set(seed.get() + 1);
        tb2.set_rows(make_rows(seed.get()));
    });
    body.append(&refresh);
    page.append(&s);
    anchors.push(("table", s));

    // ---- charts
    let (s, body) = section(t("Grafieken", "Charts"));
    let r = row();
    r.set_spacing(24);
    let line = kit::chart::LineChart::new(kit::chart::LineOptions::default());
    line.widget().set_size_request(360, 160);
    let line2 = kit::chart::LineChart::new(kit::chart::LineOptions { color: kit::chart::colors::MEMORY.into(), ..Default::default() });
    line2.widget().set_size_request(240, 160);
    for i in 0..60 {
        let v = 30.0 + 25.0 * ((i as f64) / 6.0).sin() + (i % 7) as f64;
        line.push(&[v]);
        line2.push(&[55.0 + (i % 5) as f64]);
    }
    r.append(line.widget());
    r.append(line2.widget());
    let spark = kit::chart::Sparkline::new(kit::chart::colors::NETWORK);
    spark.widget().set_size_request(120, 40);
    spark.update((0..30).map(|i| ((i * 37) % 23) as f64).collect());
    spark.widget().set_valign(gtk::Align::Center);
    r.append(spark.widget());
    let donut = kit::chart::Donut::new(120, 12.0);
    donut.update(
        vec![kit::chart::Segment::new(310.0, "chart-1", t("Gebruikt", "Used")), kit::chart::Segment::new(102.0, "chart-3", t("Systeem", "System"))],
        Some(512.0),
        "412 GB",
        t("van 512 GB", "of 512 GB"),
    );
    r.append(donut.widget());
    body.append(&r);
    let stacked = kit::chart::StackedBar::new(8, true);
    stacked.set_format(|v| kit::format::bytes(v * 1_073_741_824.0));
    stacked.update(
        vec![
            kit::chart::Segment::new(120.0, "chart-1", "Apps"),
            kit::chart::Segment::new(80.0, "chart-2", t("Documenten", "Documents")),
            kit::chart::Segment::new(40.0, "chart-5", t("Tijdelijke bestanden", "Temporary files")),
        ],
        Some(512.0),
    );
    stacked.widget().set_size_request(480, -1);
    stacked.widget().set_halign(gtk::Align::Start);
    body.append(stacked.widget());
    let tick_chart = line.clone();
    let n = Rc::new(std::cell::Cell::new(60u32));
    glib::timeout_add_seconds_local(1, move || {
        n.set(n.get() + 1);
        let i = n.get() as f64;
        tick_chart.push(&[30.0 + 25.0 * (i / 6.0).sin() + (n.get() % 7) as f64]);
        glib::ControlFlow::Continue
    });
    page.append(&s);
    anchors.push(("charts", s));

    // ---- misc
    let (s, body) = section(t("Overig", "Other"));
    let r = row();
    r.set_spacing(12);
    r.append(&c::badge("3", "attention"));
    r.append(&c::badge("12", "critical"));
    r.append(&c::badge("", "success"));
    r.append(&c::badge("!", "caution"));
    r.append(&c::avatar("Robin Burr", 32));
    r.append(&c::avatar("", 32));
    body.append(&r);
    let list = c::misc::list_box();
    for (g, title, desc) in [("folder", t("Documenten", "Documents"), "/home/robin/Documents"), ("image", t("Afbeeldingen", "Pictures"), "/home/robin/Pictures"), ("music", t("Muziek", "Music"), "/home/robin/Music")] {
        let chev = kit::icons::glyph("chevron-right", 12);
        list.append(&c::list_item(g, title, desc, Some(chev.upcast_ref())));
    }
    list.set_size_request(420, -1);
    list.set_halign(gtk::Align::Start);
    body.append(&list);
    body.append(&c::empty_state("search", t("Geen resultaten", "No results"), t("Probeer een ander zoekwoord of zoek in een andere map.", "Try another search term or search another folder."), Some(c::button(t("Opnieuw zoeken", "Search again"), Variant::Standard).upcast_ref())));
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(30);
    flow.set_column_spacing(4);
    flow.set_row_spacing(4);
    for name in kit::icons::glyph_names() {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        cell.set_size_request(28, 28);
        let g = kit::icons::glyph(name, 16);
        g.set_hexpand(true);
        g.set_vexpand(true);
        g.set_tooltip_text(Some(name));
        g.set_can_target(true);
        cell.append(&g);
        flow.insert(&cell, -1);
    }
    body.append(&flow);
    let fmt = c::label(
        &format!(
            "{} · {} · {} · {} · {} · {} · {}",
            kit::format::number(1234.5, 1),
            kit::format::bytes(1_288_490_188.0),
            kit::format::rate(3_565_158.0),
            kit::format::duration(7500.0),
            kit::format::uptime(93784.0),
            kit::format::date(kit::format::now_ms(), kit::format::DateFormat::Long),
            kit::format::rel_time(kit::format::now_ms() - 300_000.0, kit::format::now_ms()),
        ),
        "w11-text-2",
        true,
    );
    body.append(&fmt);
    page.append(&s);
    anchors.push(("misc", s));

    if let Some(only) = &only {
        let keep: Vec<gtk::Box> = anchors.iter().filter(|(k, _)| k == only).map(|(_, w)| w.clone()).collect();
        let mut child = page.first_child();
        while let Some(w) = child {
            child = w.next_sibling();
            let is_section = anchors.iter().any(|(_, a)| a.upcast_ref::<gtk::Widget>() == &w) || w.has_css_class("gallery-section");
            if is_section && !keep.iter().any(|k| k.upcast_ref::<gtk::Widget>() == &w) {
                page.remove(&w);
            }
        }
        nav.select(only);
    }
    let scroller = c::scrolled(&page);
    let anchors = Rc::new(anchors);
    let sc = scroller.clone();
    nav.connect_selected(move |id| {
        let key = id.split('-').next().unwrap_or(id);
        if let Some((_, w)) = anchors.iter().find(|(k, _)| *k == key)
            && let Some(p) = w.compute_point(&sc.child().unwrap_or_else(|| sc.clone().upcast()), &gtk::graphene::Point::new(0.0, 0.0))
        {
            sc.vadjustment().set_value(p.y() as f64);
        }
    });
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(nav.widget());
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("w11-content");
    layer.add_css_class("w11-layer");
    layer.set_hexpand(true);
    layer.append(&scroller);
    body.append(&layer);
    win.set_content(&body);
    win.set_back_enabled(true);
    win.connect_back(|| {});
    let s2 = search.clone();
    win.add_shortcut("<Control>f", move || {
        s2.focus();
        true
    });
    if open.as_deref() == Some("dialog") {
        let w = win.clone();
        {
            let entry = c::text_box(t("Nieuwe naam", "New name"));
            entry.set_text("Projecten 2026");
            let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
            body.append(&c::label(t("Geef de map een nieuwe naam.", "Give the folder a new name."), "w11-dialog-text", true));
            body.append(&entry);
            c::Dialog::new(t("Naam wijzigen", "Rename"))
                .body(&body)
                .primary(t("Opslaan", "Save"))
                .secondary(t("Niet opslaan", "Don't save"))
                .show(w.window(), |_| {});
        }
    }
    win.present();
}
