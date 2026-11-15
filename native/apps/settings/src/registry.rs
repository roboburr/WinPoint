//! The page registry (6ebf169f:vendor/ui/js/apps/settings/registry.js): every page with its parent, glyph, title,
//! description and search keywords in Dutch and English, the aliases deep links may use, and the search of the
//! navigation pane's "Zoek een instelling" (the same scoring as registry.js searchSettings).
//!
//! Pure data and one helper. `icon` names are kit glyphs (icons.js); `parent` is the id of the top-level page.

/// One Settings page.
pub struct PageInfo {
    pub id: &'static str,
    pub parent: Option<&'static str>,
    pub icon: &'static str,
    pub title: (&'static str, &'static str),
    pub description: (&'static str, &'static str),
    pub keywords: (&'static [&'static str], &'static [&'static str]),
}

impl PageInfo {
    /// The title in the UI language.
    pub fn title(&self) -> &'static str {
        kit::t(self.title.0, self.title.1)
    }

    /// The description in the UI language.
    pub fn description(&self) -> &'static str {
        kit::t(self.description.0, self.description.1)
    }

    fn title_in(&self, en: bool) -> &'static str {
        if en { self.title.1 } else { self.title.0 }
    }

    fn description_in(&self, en: bool) -> &'static str {
        if en { self.description.1 } else { self.description.0 }
    }

    fn keywords_in(&self, en: bool) -> &'static [&'static str] {
        if en { self.keywords.1 } else { self.keywords.0 }
    }
}

macro_rules! page {
    ($id:expr, $parent:expr, $icon:expr, $nl:expr, $en:expr, $dnl:expr, $den:expr, [$($knl:expr),* $(,)?], [$($ken:expr),* $(,)?]) => {
        PageInfo {
            id: $id,
            parent: $parent,
            icon: $icon,
            title: ($nl, $en),
            description: ($dnl, $den),
            keywords: (&[$($knl),*], &[$($ken),*]),
        }
    };
}

/// Every page, top-level pages in navigation order with their sub-pages after them.
pub static PAGES: &[PageInfo] = &[
    page!("home", None, "home", "Startpagina", "Home",
        "Aanbevolen instellingen, apparaten en thema in één oogopslag", "Recommended settings, devices and theme at a glance",
        ["start", "overzicht", "begin", "aanbevolen"], ["home", "overview", "start", "recommended"]),

    // ---- Systeem
    page!("system", None, "pc", "Systeem", "System",
        "Beeldscherm, geluid, meldingen, energie, opslag", "Display, sound, notifications, power, storage",
        ["computer", "pc", "apparaat", "systeem"], ["computer", "pc", "device", "system"]),
    page!("display", Some("system"), "display", "Beeldscherm", "Display",
        "Beeldschermen, helderheid, nachtlampje, schaal en resolutie", "Monitors, brightness, night light, scale and resolution",
        ["scherm", "monitor", "helderheid", "licht", "nachtlampje", "nachtlicht", "blauw licht", "resolutie", "schaal", "zoom",
            "vernieuwingsfrequentie", "hz", "beeldschermen", "tweede scherm"],
        ["screen", "monitor", "brightness", "night light", "blue light", "resolution", "scale", "zoom", "refresh rate", "hz",
            "multiple displays"]),
    page!("sound", Some("system"), "sound", "Geluid", "Sound",
        "Volume, uitvoer en geluidsapparaten", "Volume, output and sound devices",
        ["geluid", "volume", "luidspreker", "speaker", "koptelefoon", "hoofdtelefoon", "audio", "dempen", "stil", "uitvoer"],
        ["sound", "volume", "speaker", "headphones", "headset", "audio", "mute", "output"]),
    page!("notifications", Some("system"), "notification", "Meldingen", "Notifications",
        "Waarschuwingen van apps en systeem, niet storen", "Alerts from apps and system, do not disturb",
        ["meldingen", "notificaties", "niet storen", "stil", "pop-up", "berichten", "geluid melding", "testmelding"],
        ["notifications", "alerts", "do not disturb", "dnd", "toast", "banner", "test notification"]),
    page!("power", Some("system"), "power", "Energie en batterij", "Power & battery",
        "Energiestand, scherm uitschakelen, automatisch slapen en batterij", "Power mode, screen timeout, automatic sleep and battery",
        ["energie", "energiestand", "energie-instellingen", "energiebesparing", "energie-efficiëntie", "prestaties", "gebalanceerd",
            "stroom", "batterij", "accu", "scherm uitschakelen", "scherm dimmen", "schermtimeout", "automatisch slapen",
            "slaapinstellingen", "inactiviteit", "afsluiten", "uitzetten", "opnieuw opstarten", "herstarten", "slaapstand",
            "vergrendelen", "afmelden", "uitloggen"],
        ["power", "power mode", "power settings", "energy", "energy settings", "power saver", "efficiency", "performance",
            "balanced", "battery", "screen timeout", "turn off screen", "dim screen", "automatic sleep", "sleep settings", "idle",
            "shut down", "turn off", "restart", "reboot", "sleep", "lock", "sign out", "log out"]),
    page!("storage", Some("system"), "storage", "Opslag", "Storage",
        "Opslagruimte, stations, ruimte vrijmaken", "Storage space, drives, free up space",
        ["opslag", "schijf", "harde schijf", "ruimte", "vol", "vrijmaken", "opruimen", "stations", "usb", "disk", "ssd"],
        ["storage", "disk", "drive", "space", "full", "free up", "clean up", "drives", "usb", "ssd"]),
    page!("clipboard", Some("system"), "clipboard", "Klembord", "Clipboard",
        "Knippen en kopiëren, klembordgeschiedenis", "Cut and copy history",
        ["klembord", "kopiëren", "plakken", "knippen", "geschiedenis", "windows v", "super v"],
        ["clipboard", "copy", "paste", "cut", "history", "win v", "super v"]),
    page!("about", Some("system"), "info", "Info", "About",
        "Apparaatspecificaties, processor, geheugen, versie", "Device specifications, processor, memory, version",
        ["info", "over", "specificaties", "processor", "cpu", "geheugen", "ram", "videokaart", "gpu", "versie", "ubuntu", "kernel",
            "computernaam", "pc-naam"],
        ["about", "specifications", "specs", "processor", "cpu", "memory", "ram", "graphics", "gpu", "version", "ubuntu", "kernel",
            "device name"]),

    // ---- Bluetooth
    page!("bluetooth", None, "bluetooth", "Bluetooth en apparaten", "Bluetooth & devices",
        "Bluetooth, koptelefoons, muizen en andere apparaten", "Bluetooth, headphones, mice and other devices",
        ["bluetooth", "apparaten", "koppelen", "verbinden", "koptelefoon", "muis", "toetsenbord", "oortjes", "speaker"],
        ["bluetooth", "devices", "pair", "connect", "headphones", "mouse", "keyboard", "earbuds", "speaker"]),

    // ---- Netwerk
    page!("network", None, "globe", "Netwerk en internet", "Network & internet",
        "Wi-Fi, ethernet, vliegtuigmodus", "Wi-Fi, Ethernet, flight mode",
        ["netwerk", "internet", "verbinding", "online", "vliegtuigmodus", "offline"],
        ["network", "internet", "connection", "online", "flight mode", "airplane mode", "offline"]),
    page!("wifi", Some("network"), "wifi", "Wi-Fi", "Wi-Fi",
        "Draadloze netwerken, verbinden, wachtwoord", "Wireless networks, connect, password",
        ["wifi", "wi-fi", "wlan", "draadloos", "netwerk", "wachtwoord", "verbinden", "hotspot"],
        ["wifi", "wi-fi", "wlan", "wireless", "network", "password", "connect", "hotspot"]),
    page!("ethernet", Some("network"), "ethernet", "Ethernet", "Ethernet",
        "Kabelverbinding en IP-adres", "Wired connection and IP address",
        ["ethernet", "kabel", "lan", "bekabeld", "ip-adres", "ip"],
        ["ethernet", "cable", "lan", "wired", "ip address", "ip"]),

    // ---- Persoonlijke instellingen
    page!("personalization", None, "personalize", "Persoonlijke instellingen", "Personalisation",
        "Achtergrond, kleuren, thema’s, Start, taakbalk", "Background, colours, themes, Start, taskbar",
        ["persoonlijk", "aanpassen", "uiterlijk", "personaliseren"], ["personalize", "personalise", "customize", "appearance"]),
    page!("background", Some("personalization"), "image", "Achtergrond", "Background",
        "Bureaubladachtergrond, foto of effen kleur", "Desktop background, picture or solid colour",
        ["achtergrond", "bureaublad", "wallpaper", "foto", "afbeelding", "plaatje", "effen kleur", "behang"],
        ["background", "desktop", "wallpaper", "picture", "photo", "image", "solid colour", "solid color"]),
    page!("colors", Some("personalization"), "color", "Kleuren", "Colours",
        "Licht of donker, accentkleur, transparantie", "Light or dark, accent colour, transparency",
        ["kleur", "kleuren", "donker", "licht", "donkere modus", "lichte modus", "nachtmodus", "accent", "accentkleur",
            "transparantie", "doorzichtig", "vensterknoppen", "sluitknop", "minimaliseren", "maximaliseren", "thema voor apps",
            "gtk-thema", "yaru", "fluent"],
        ["colour", "color", "colours", "colors", "dark", "light", "dark mode", "light mode", "accent", "transparency",
            "caption buttons", "close button", "minimize", "maximize", "app theme", "gtk theme", "yaru", "fluent"]),
    page!("themes", Some("personalization"), "theme", "Thema’s", "Themes",
        "Achtergrond, kleur en modus in één keer", "Background, colour and mode in one go",
        ["thema", "thema's", "stijl", "uiterlijk"], ["theme", "themes", "style", "look"]),
    page!("start", Some("personalization"), "start-menu", "Start", "Start",
        "Indeling van Start, recente apps, mappen", "Start layout, recent apps, folders",
        ["start", "startmenu", "menu start", "vastgemaakt", "aanbevolen", "mappen", "recente apps"],
        ["start", "start menu", "pinned", "recommended", "folders", "recent apps"]),
    page!("taskbar", Some("personalization"), "taskbar", "Taakbalk", "Taskbar",
        "Uitlijning, zoeken, widgets, automatisch verbergen", "Alignment, search, widgets, auto-hide",
        ["taakbalk", "balk", "links", "midden", "gecentreerd", "uitlijning", "verbergen", "zoekvak", "widgets", "taakweergave",
            "klok", "seconden", "systeemvak", "systeemvakpictogrammen", "meldingsgebied", "verborgen pictogrammen", "pijltje",
            "overloop", "tray"],
        ["taskbar", "left", "centre", "center", "alignment", "hide", "auto-hide", "search box", "widgets", "task view", "clock",
            "seconds", "system tray", "tray icons", "notification area", "hidden icons", "hidden icon menu", "overflow"]),
    page!("desktopicons", Some("personalization"), "desktop", "Bureaubladpictogrammen", "Desktop icons",
        "Deze pc, Prullenbak en Persoonlijke map op het bureaublad", "This PC, Recycle Bin and user folder on the desktop",
        ["bureaublad", "pictogrammen", "iconen", "deze pc", "prullenbak", "persoonlijke map"],
        ["desktop", "icons", "this pc", "recycle bin", "trash", "home folder"]),

    // ---- Apps
    page!("apps", None, "apps", "Apps", "Apps",
        "Geïnstalleerde apps, standaard-apps, opstarten", "Installed apps, default apps, startup",
        ["apps", "programma", "programma’s", "toepassingen", "software"], ["apps", "programs", "applications", "software"]),
    page!("installed", Some("apps"), "apps", "Geïnstalleerde apps", "Installed apps",
        "Alle apps op deze pc, openen en vastmaken", "All apps on this PC, open and pin",
        ["geïnstalleerd", "apps", "programma", "vastmaken", "lijst", "verwijderen"],
        ["installed", "apps", "programs", "pin", "list", "uninstall"]),
    page!("defaultapps", Some("apps"), "open", "Standaard-apps", "Default apps",
        "Welke app opent je webpagina’s, muziek en foto’s", "Which app opens your web pages, music and photos",
        ["standaard", "standaardapp", "browser", "openen met", "standaardbrowser", "mail", "muziekspeler"],
        ["default", "default apps", "browser", "open with", "default browser", "mail", "music player"]),
    page!("startup", Some("apps"), "startup", "Opstarten", "Startup",
        "Apps die starten wanneer je je aanmeldt", "Apps that start when you sign in",
        ["opstarten", "automatisch starten", "autostart", "aanmelden", "opstartprogramma"],
        ["startup", "autostart", "start automatically", "sign in"]),

    // ---- Accounts
    page!("accounts", None, "user", "Accounts", "Accounts",
        "Je account, naam en profielfoto", "Your account, name and picture",
        ["account", "gebruiker", "profiel", "naam", "foto", "wachtwoord", "aanmelden"],
        ["account", "user", "profile", "name", "picture", "password", "sign in"]),

    // ---- Tijd en taal
    page!("time", None, "clock", "Tijd en taal", "Time & language",
        "Datum, tijd, tijdzone en taal", "Date, time, time zone and language",
        ["tijd", "taal", "datum", "klok"], ["time", "language", "date", "clock"]),
    page!("datetime", Some("time"), "calendar", "Datum en tijd", "Date & time",
        "Tijdzone, 24-uursnotatie, seconden, eerste dag van de week", "Time zone, 24-hour clock, seconds, first day of week",
        ["datum", "tijd", "klok", "tijdzone", "24 uur", "24-uurs", "am pm", "seconden", "week", "maandag", "zondag", "kalender"],
        ["date", "time", "clock", "time zone", "24 hour", "24-hour", "am pm", "seconds", "week", "monday", "sunday", "calendar"]),
    page!("language", Some("time"), "language", "Taal en regio", "Language & region",
        "Weergavetaal: Nederlands of English", "Display language: Dutch or English",
        ["taal", "nederlands", "engels", "english", "regio", "land", "vertaling"],
        ["language", "dutch", "english", "region", "country", "translation"]),

    // ---- Toegankelijkheid
    page!("accessibility", None, "accessibility", "Toegankelijkheid", "Accessibility",
        "Tekstgrootte, visuele effecten, contrast", "Text size, visual effects, contrast",
        ["toegankelijkheid", "slechtziend", "leesbaarheid", "hulp"], ["accessibility", "low vision", "readability", "ease of access"]),
    page!("textsize", Some("accessibility"), "zoom-in", "Tekstgrootte", "Text size",
        "Tekst groter maken", "Make text bigger",
        ["tekst", "groter", "lettergrootte", "letters", "vergroten", "grootte", "lezen", "zoom"],
        ["text", "bigger", "font size", "larger", "enlarge", "size", "read", "zoom"]),
    page!("visualeffects", Some("accessibility"), "sparkle", "Visuele effecten", "Visual effects",
        "Animaties en transparantie", "Animations and transparency",
        ["animatie", "animaties", "beweging", "transparantie", "effecten", "rustig"],
        ["animation", "animations", "motion", "transparency", "effects", "reduce motion"]),
    page!("contrast", Some("accessibility"), "eye", "Contrastthema’s", "Contrast themes",
        "Meer contrast voor beter leesbare tekst", "More contrast for easier reading",
        ["contrast", "hoog contrast", "leesbaar", "kleurenblind"], ["contrast", "high contrast", "readable", "colour blind", "color blind"]),

    // ---- Privacy
    page!("privacy", None, "shield", "Privacy en beveiliging", "Privacy & security",
        "Recente bestanden, beveiliging", "Recent files, security",
        ["privacy", "beveiliging", "recente bestanden", "geschiedenis", "wissen", "veilig"],
        ["privacy", "security", "recent files", "history", "clear", "safe"]),

    // ---- Update en herstel
    page!("update", None, "update", "Update en herstel", "Update & recovery",
        "Updates, back-up van instellingen, alles terugzetten", "Updates, settings backup, reset",
        ["update", "updates", "bijwerken", "software-updater", "back-up", "backup", "exporteren", "importeren", "terugzetten",
            "herstellen", "resetten", "versie"],
        ["update", "updates", "upgrade", "software updater", "backup", "export", "import", "reset", "restore", "recovery",
            "version"]),
    page!("update-history", Some("update"), "history", "Updategeschiedenis", "Update history",
        "Eerdere updates en Ubuntu-versies", "Earlier updates and Ubuntu versions",
        ["updategeschiedenis", "geschiedenis", "eerdere updates", "logboek"], ["update history", "history", "past updates", "log"]),
    page!("update-advanced", Some("update"), "settings", "Geavanceerde opties", "Advanced options",
        "Melding bij opnieuw opstarten, datalimiet, Software-updater van Ubuntu", "Restart notifications, metered connections, Ubuntu Software Updater",
        ["geavanceerd", "datalimiet", "software-updater", "automatisch bijwerken", "beveiligingsupdates", "snap"],
        ["advanced", "metered", "software updater", "automatic updates", "security updates", "snap"]),
];

/// Old or alternative ids that deep links may use (registry.js SETTINGS_ALIASES).
pub static ALIASES: &[(&str, &str)] = &[
    ("nightlight", "display"), ("brightness", "display"), ("monitors", "display"), ("volume", "sound"), ("audio", "sound"),
    ("battery", "power"), ("energy", "power"), ("disk", "storage"), ("info", "about"), ("devices", "bluetooth"),
    ("internet", "network"), ("wallpaper", "background"), ("accent", "colors"), ("personalize", "personalization"),
    ("default-apps", "defaultapps"), ("defaults", "defaultapps"), ("autostart", "startup"), ("user", "accounts"),
    ("clock", "datetime"), ("date", "datetime"), ("lang", "language"), ("updates", "update"), ("backup", "update"),
    ("reset", "update"), ("recovery", "update"), ("ease", "accessibility"), ("text", "textsize"), ("animations", "visualeffects"),
    ("security", "privacy"), ("taskmgr", "storage"), ("theme", "themes"),
    // ids other modules (shell, desktop, flyouts, host health actions) or Windows' ms-settings: names may use
    ("personalisation", "personalization"), ("colours", "colors"), ("colour", "colors"), ("color", "colors"),
    ("lockscreen", "personalization"), ("fonts", "personalization"), ("start-menu", "start"), ("startmenu", "start"),
    ("taskbar-settings", "taskbar"), ("notification", "notifications"), ("meldingen", "notifications"),
    ("date-time", "datetime"), ("dateandtime", "datetime"), ("datum-tijd", "datetime"), ("region", "language"),
    ("apps-features", "installed"), ("appsfeatures", "installed"), ("installed-apps", "installed"), ("installedapps", "installed"),
    ("programs", "installed"), ("startup-apps", "startup"), ("startupapps", "startup"), ("your-info", "accounts"),
    ("yourinfo", "accounts"), ("account", "accounts"), ("wireless", "wifi"), ("wi-fi", "wifi"), ("wired", "ethernet"),
    ("network-status", "network"), ("bluetoothdevices", "bluetooth"), ("sound-devices", "sound"), ("power-sleep", "power"),
    ("powersleep", "power"), ("sleep", "power"), ("storage-sense", "storage"), ("storagesense", "storage"),
    ("clipboardhistory", "clipboard"), ("about-pc", "about"), ("pc", "about"), ("windows-update", "update"),
    ("windowsupdate", "update"), ("privacy-security", "privacy"), ("text-size", "textsize"), ("visual-effects", "visualeffects"),
    ("contrast-themes", "contrast"), ("desktop-icons", "desktopicons"), ("night-light", "display"),
    // Windows Update subpages (ms-settings:windowsupdate-history, -options, -action)
    ("windowsupdate-history", "update-history"), ("updatehistory", "update-history"), ("windowsupdate-options", "update-advanced"),
    ("windowsupdate-action", "update"),
];

/// A page by id.
pub fn page(id: &str) -> Option<&'static PageInfo> {
    PAGES.iter().find(|p| p.id == id)
}

/// The top-level page ids in navigation order.
pub fn top_ids() -> Vec<&'static str> {
    PAGES.iter().filter(|p| p.parent.is_none()).map(|p| p.id).collect()
}

/// The page id a deep link means: "search", a page id or an alias (any case); None when unknown.
pub fn resolve(id: &str) -> Option<&'static str> {
    let s = id.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if s == "search" {
        return Some("search");
    }
    if let Some(p) = page(&s) {
        return Some(p.id);
    }
    ALIASES.iter().find(|(a, _)| *a == s).map(|(_, target)| *target)
}

/// The top-level page of a page (itself when it has no parent).
pub fn top_of(id: &str) -> &'static str {
    match page(id) {
        Some(p) => p.parent.unwrap_or(p.id),
        None => "home",
    }
}

/// Lower case, no accents, no apostrophes (registry.js norm).
fn norm(s: &str) -> String {
    let lower = s.to_lowercase();
    let nfd = gtk::glib::normalize(&lower, gtk::glib::NormalizeMode::Default);
    nfd.chars()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c) && *c != '’' && *c != '\'')
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The best matching pages for a query, best first (registry.js searchSettings): title (exact, prefix, word,
/// substring), keywords and description in the language, weaker in the other language; every word must match.
pub fn search(query: &str, en: bool, limit: usize) -> Vec<&'static PageInfo> {
    let q = norm(query);
    if q.is_empty() {
        return Vec::new();
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut scored: Vec<(&'static PageInfo, u32)> = Vec::new();
    for page in PAGES {
        let mut total = 0u32;
        for w in &words {
            let mut best = 0u32;
            let title = norm(page.title_in(en));
            if title == *w {
                best = 100;
            } else if title.starts_with(w) {
                best = 80;
            } else if title.split(|c: char| c.is_whitespace() || c == '-').any(|x| x.starts_with(w)) {
                best = 65;
            } else if title.contains(w) {
                best = 50;
            }
            for k in page.keywords_in(en) {
                let nk = norm(k);
                if nk == *w {
                    best = best.max(60);
                } else if nk.starts_with(w) {
                    best = best.max(45);
                } else if nk.contains(w) && w.chars().count() > 2 {
                    best = best.max(30);
                }
            }
            if best == 0 && w.chars().count() > 2 && norm(page.description_in(en)).contains(w) {
                best = 20;
            }
            if best == 0 {
                if norm(page.title_in(!en)).starts_with(w) {
                    best = 25;
                } else if page.keywords_in(!en).iter().any(|k| norm(k).starts_with(w)) {
                    best = 15;
                }
            }
            if best == 0 {
                total = 0;
                break;
            }
            total += best;
        }
        if total > 0 {
            scored.push((page, total + if page.parent.is_some() { 0 } else { 2 }));
        }
    }
    scored.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| kit::format::compare_natural(&a.0.title_in(en).to_lowercase(), &b.0.title_in(en).to_lowercase()))
    });
    scored.into_iter().take(limit).map(|(p, _)| p).collect()
}
