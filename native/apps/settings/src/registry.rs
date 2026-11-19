//! The page registry (6ebf169f:vendor/ui/js/apps/settings/registry.js): every page with its parent, glyph, title,
//! description and search keywords in Dutch and English, the aliases deep links may use, and the search of the
//! navigation pane's "Zoek een instelling" (the same scoring as registry.js searchSettings).
//!
//! Pure data and its helpers. `icon` names are kit glyphs (icons.js); `parent` is the id of the parent page: a
//! top-level page, or a sub-page for the third level (Netwerk en internet › Geavanceerde netwerkinstellingen ›
//! Geavanceerde instellingen voor delen). The page tree, its owners and the rules for changing this file are in
//! notes/claude/settings-parity.md §2-3: after batch B0 only the integration step edits it (registry notes).
//!
//! [`NEEDS`] names the caps key (`caps.get`, host/src/services/caps.rs) a page needs: navigation, child links and the
//! search leave the page out while the key is false. [`DATA_PAGES`] need a parameter (the network, printer, app or
//! device they show): they are reached from their parent only, never from the search, Start's settings_pages.json,
//! Startpagina's recent list or an alias.
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

/// Every page, top-level pages in navigation order with their sub-pages after them (Windows 11's order).
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
            "vernieuwingsfrequentie", "hz", "beeldschermen", "tweede scherm", "hdr", "privacyscherm", "draaivergrendeling", "dupliceren", "uitbreiden", "hoofdbeeldscherm"],
        ["screen", "monitor", "brightness", "night light", "blue light", "resolution", "scale", "zoom", "refresh rate", "hz",
            "multiple displays", "hdr", "privacy screen", "rotation lock", "duplicate", "extend", "main display"]),
    page!("display-advanced", Some("display"), "display", "Geavanceerd beeldscherm", "Advanced display",
        "Beeldscherminformatie en vernieuwingsfrequentie", "Display information and refresh rate",
        ["geavanceerd beeldscherm", "vernieuwingsfrequentie", "verversingssnelheid", "hz", "beeldscherminformatie", "aanpassen voor tv", "wazige apps", "schaal voor apps herstellen", "dynamische vernieuwingsfrequentie", "vrr"],
        ["advanced display", "refresh rate", "hz", "display information", "adjust for tv", "fix scaling", "fix scaling for apps", "dynamic refresh rate", "vrr"]),
    page!("nightlight", Some("display"), "night-light", "Nachtlampje", "Night light",
        "Warmere kleuren in de avond, planning en sterkte", "Warmer colours in the evening, schedule and strength",
        ["nachtlampje", "nachtlicht", "blauw licht", "warme kleuren", "avond", "zonsondergang", "planning", "kleurtemperatuur", "nu uitschakelen"],
        ["night light", "blue light", "warm colours", "warm colors", "evening", "sunset", "schedule", "colour temperature", "color temperature", "turn off now"]),
    page!("colorprofile", Some("display"), "color", "Kleurprofiel", "Colour profile",
        "Kleurprofielen van beeldschermen en printers, kalibreren", "Colour profiles of displays and printers, calibrate",
        ["kleurprofiel", "kleurbeheer", "icc", "kalibreren", "kleurkalibratie"],
        ["colour profile", "color profile", "colour management", "color management", "icc", "calibrate"]),
    page!("sound", Some("system"), "sound", "Geluid", "Sound",
        "Volume, uitvoer en geluidsapparaten", "Volume, output and sound devices",
        ["geluid", "volume", "luidspreker", "speaker", "koptelefoon", "hoofdtelefoon", "audio", "dempen", "stil", "uitvoer", "invoer", "microfoon", "invoerapparaat", "geluidsschema", "opstartgeluid"],
        ["sound", "volume", "speaker", "headphones", "headset", "audio", "mute", "output", "input", "microphone", "input device", "sound scheme", "startup sound"]),
    page!("sound-device", Some("sound"), "volume", "Eigenschappen", "Properties",
        "Indeling, poort, balans en testen van een geluidsapparaat", "Format, port, balance and test of a sound device",
        ["eigenschappen", "geluidsapparaat", "balans", "poort", "linkerkanaal", "rechterkanaal", "indeling", "testen"],
        ["properties", "sound device", "balance", "port", "left channel", "right channel", "format", "test"]),
    page!("sound-devices", Some("sound"), "devices", "Alle geluidsapparaten", "All sound devices",
        "Alle invoer- en uitvoerapparaten, ook uitgeschakelde", "All input and output devices, also turned-off ones",
        ["alle geluidsapparaten", "geluidsapparaten", "geluidskaart", "uitgeschakeld apparaat"],
        ["all sound devices", "sound devices", "sound card", "disabled device"]),
    page!("volume-mixer", Some("sound"), "volume", "Volumemixer", "Volume mixer",
        "Volume per app en systeemgeluiden", "Volume per app and system sounds",
        ["volumemixer", "mixer", "volume per app", "systeemgeluiden", "app-volume"],
        ["volume mixer", "mixer", "app volume", "system sounds"]),
    page!("notifications", Some("system"), "notification", "Meldingen", "Notifications",
        "Waarschuwingen van apps en systeem, niet storen", "Alerts from apps and system, do not disturb",
        ["meldingen", "notificaties", "niet storen", "stil", "pop-up", "berichten", "geluid melding", "testmelding"],
        ["notifications", "alerts", "do not disturb", "dnd", "toast", "banner", "test notification"]),
    page!("app-notifications", Some("notifications"), "notification", "Meldingen van app", "App notifications",
        "Meldingen, banners en geluid van één app", "Notifications, banners and sound of one app",
        ["meldingen van app", "banners"],
        ["app notifications", "banners"]),
    page!("screentime", Some("system"), "timer", "Schermtijd", "Screen time",
        "Hoe lang je deze pc gebruikt, limieten en pauzes", "How long you use this PC, limits and breaks",
        ["schermtijd", "pauze", "pauzes", "herinnering", "limiet", "ogen", "bewegen", "welzijn", "dagelijkse limiet", "grijswaarden", "ouderlijk toezicht", "bewegingspauzes"],
        ["screen time", "break", "breaks", "reminder", "limit", "eyes", "movement", "wellbeing", "daily limit", "greyscale", "grayscale", "parental controls", "movement breaks"]),
    page!("power", Some("system"), "power", "Energie en batterij", "Power & battery",
        "Energiestand, scherm uitschakelen, automatisch slapen en batterij", "Power mode, screen timeout, automatic sleep and battery",
        ["energie", "energiestand", "energie-instellingen", "energiebesparing", "energie-efficiëntie", "prestaties", "gebalanceerd",
            "stroom", "batterij", "accu", "scherm uitschakelen", "scherm dimmen", "schermtimeout", "automatisch slapen",
            "slaapinstellingen", "inactiviteit", "afsluiten", "uitzetten", "opnieuw opstarten", "herstarten", "slaapstand",
            "vergrendelen", "afmelden", "uitloggen", "aan/uit-knop", "batterij opladen", "batterijpercentage", "batterijniveaus"],
        ["power", "power mode", "power settings", "energy", "energy settings", "power saver", "efficiency", "performance",
            "balanced", "battery", "screen timeout", "turn off screen", "dim screen", "automatic sleep", "sleep settings", "idle",
            "shut down", "turn off", "restart", "reboot", "sleep", "lock", "sign out", "log out", "power button", "battery charging", "battery percentage", "battery levels"]),
    page!("storage", Some("system"), "storage", "Opslag", "Storage",
        "Opslagruimte, stations, ruimte vrijmaken", "Storage space, drives, free up space",
        ["opslag", "schijf", "harde schijf", "ruimte", "vol", "vrijmaken", "opruimen", "stations", "usb", "disk", "ssd", "opslaginzicht"],
        ["storage", "disk", "drive", "space", "full", "free up", "clean up", "drives", "usb", "ssd", "storage sense"]),
    page!("storagesense", Some("storage"), "clean", "Opslaginzicht", "Storage Sense",
        "Tijdelijke bestanden en de Prullenbak automatisch opruimen", "Clean up temporary files and the Recycle Bin automatically",
        ["opslaginzicht", "opslagzin", "opruimen", "tijdelijke bestanden", "prullenbak", "automatisch", "prullenbak leegmaken", "tijdelijke bestanden verwijderen", "opslaginzicht nu uitvoeren"],
        ["storage sense", "clean up", "temporary files", "recycle bin", "automatically", "empty recycle bin", "delete temporary files", "run storage sense now"]),
    page!("multitasking", Some("system"), "snap", "Multitasking", "Multitasking",
        "Vensters uitlijnen, virtuele bureaubladen, Alt+Tab", "Snap windows, virtual desktops, Alt+Tab",
        ["multitasking", "vensters uitlijnen", "uitlijnen", "bureaubladen", "virtuele bureaubladen", "werkruimten", "alt tab", "actieve hoek", "aantal bureaubladen", "hoofdbeeldscherm", "taakweergave"],
        ["multitasking", "snap windows", "snap", "desktops", "virtual desktops", "workspaces", "alt tab", "hot corner", "number of desktops", "main display", "task view"]),
    page!("system-advanced", Some("system"), "code", "Geavanceerd", "Advanced",
        "Voor ontwikkelaars: OpenSSH-server en optionele functies", "For developers: OpenSSH server and optional features",
        ["geavanceerd", "ontwikkelaars", "ssh", "openssh", "optionele functies", "terminal"],
        ["advanced", "developers", "ssh", "openssh", "optional features", "terminal"]),
    page!("remotedesktop", Some("system"), "remote-desktop", "Extern bureaublad", "Remote Desktop",
        "Deze pc op afstand bedienen", "Control this PC remotely",
        ["extern bureaublad", "rdp", "op afstand", "bediening op afstand", "schermdeling"],
        ["remote desktop", "rdp", "remote", "remote control", "screen sharing"]),
    page!("clipboard", Some("system"), "clipboard", "Klembord", "Clipboard",
        "Knippen en kopiëren, klembordgeschiedenis", "Cut and copy history",
        ["klembord", "kopiëren", "plakken", "knippen", "geschiedenis", "windows v", "super v"],
        ["clipboard", "copy", "paste", "cut", "history", "win v", "super v"]),
    page!("about", Some("system"), "info", "Info", "About",
        "Apparaatspecificaties, processor, geheugen, versie", "Device specifications, processor, memory, version",
        ["info", "over", "specificaties", "processor", "cpu", "geheugen", "ram", "videokaart", "gpu", "versie", "ubuntu", "kernel",
            "computernaam", "pc-naam", "hostname", "gnome", "firmware", "wayland"],
        ["about", "specifications", "specs", "processor", "cpu", "memory", "ram", "graphics", "gpu", "version", "ubuntu", "kernel",
            "device name", "hostname", "pc name", "gnome", "firmware", "wayland"]),

    // ---- Bluetooth en apparaten
    page!("bluetooth", None, "bluetooth", "Bluetooth en apparaten", "Bluetooth & devices",
        "Bluetooth, koptelefoons, muizen en andere apparaten", "Bluetooth, headphones, mice and other devices",
        ["bluetooth", "apparaten", "koppelen", "verbinden", "koptelefoon", "muis", "toetsenbord", "oortjes", "speaker"],
        ["bluetooth", "devices", "pair", "connect", "headphones", "mouse", "keyboard", "earbuds", "speaker"]),
    page!("bluetooth-devices", Some("bluetooth"), "devices", "Apparaten", "Devices",
        "Muis, toetsenbord, pen, audio, beeldschermen en docks, overige apparaten", "Mouse, keyboard, pen, audio, displays and docks, other devices",
        ["apparaten", "gekoppeld", "koppelen", "verwijderen", "batterij van apparaat"],
        ["devices", "paired", "pair", "remove", "device battery"]),
    page!("printers", Some("bluetooth"), "print", "Printers en scanners", "Printers & scanners",
        "Printers en scanners toevoegen, de afdrukwachtrij", "Add printers and scanners, the print queue",
        ["printer", "printers", "scanner", "afdrukken", "printen", "afdrukwachtrij", "wachtrij"],
        ["printer", "printers", "scanner", "print", "print queue", "queue"]),
    page!("printer", Some("printers"), "print", "Printer", "Printer",
        "Wachtrij, voorkeuren en eigenschappen van een printer", "Queue, preferences and properties of a printer",
        ["printer", "afdrukwachtrij"],
        ["printer", "print queue"]),
    page!("mouse", Some("bluetooth"), "mouse", "Muis", "Mouse",
        "Primaire muisknop, aanwijzersnelheid, schuiven", "Primary mouse button, pointer speed, scrolling",
        ["muis", "muisknop", "linkshandig", "aanwijzer", "snelheid", "schuiven", "scrollen", "dubbelklikken"],
        ["mouse", "mouse button", "left-handed", "pointer", "speed", "scroll", "scrolling", "double-click"]),
    page!("touchpad", Some("bluetooth"), "touchpad", "Touchpad", "Touchpad",
        "Tikken, schuiven en zoomen, cursorsnelheid", "Taps, scroll and zoom, cursor speed",
        ["touchpad", "trackpad", "tikken", "schuiven", "twee vingers"],
        ["touchpad", "trackpad", "tap", "scroll", "two fingers"]),
    page!("pen", Some("bluetooth"), "pen", "Pen en tablet", "Pen & tablet",
        "Tekentablet, penknoppen en pendruk", "Drawing tablet, pen buttons and pen pressure",
        ["pen", "tablet", "wacom", "stylus", "tekentablet", "penknoppen"],
        ["pen", "tablet", "wacom", "stylus", "drawing tablet", "pen buttons"]),
    page!("autoplay", Some("bluetooth"), "play", "Automatisch afspelen", "AutoPlay",
        "Standaardinstellingen voor verwisselbare stations en geheugenkaarten", "Defaults for removable drives and memory cards",
        ["automatisch afspelen", "autoplay", "usb-stick", "cd", "dvd", "geheugenkaart"],
        ["autoplay", "usb drive", "cd", "dvd", "memory card"]),
    page!("usb", Some("bluetooth"), "usb", "USB", "USB",
        "USB4- en Thunderbolt-apparaten", "USB4 and Thunderbolt devices",
        ["usb", "usb4", "thunderbolt", "dock", "externe gpu"],
        ["usb", "usb4", "thunderbolt", "dock", "external gpu"]),

    // ---- Netwerk en internet
    page!("network", None, "globe", "Netwerk en internet", "Network & internet",
        "Wi-Fi, ethernet, vliegtuigmodus", "Wi-Fi, Ethernet, flight mode",
        ["netwerk", "internet", "verbinding", "online", "vliegtuigmodus", "offline"],
        ["network", "internet", "connection", "online", "flight mode", "airplane mode", "offline"]),
    page!("wifi", Some("network"), "wifi", "Wi-Fi", "Wi-Fi",
        "Draadloze netwerken, verbinden, wachtwoord", "Wireless networks, connect, password",
        ["wifi", "wi-fi", "wlan", "draadloos", "netwerk", "wachtwoord", "verbinden", "hotspot"],
        ["wifi", "wi-fi", "wlan", "wireless", "network", "password", "connect", "hotspot"]),
    page!("wifi-network", Some("wifi"), "wifi", "Wi-Fi-eigenschappen", "Wi-Fi properties",
        "Verbinding, IP-toewijzing en beveiliging van een netwerk", "Connection, IP assignment and security of a network",
        ["wi-fi-eigenschappen"],
        ["wi-fi properties"]),
    page!("wifi-known", Some("wifi"), "wifi", "Bekende netwerken beheren", "Manage known networks",
        "Opgeslagen Wi-Fi-netwerken vergeten of toevoegen", "Forget or add saved Wi-Fi networks",
        ["bekende netwerken", "opgeslagen netwerken", "vergeten", "netwerk toevoegen"],
        ["known networks", "saved networks", "forget", "add network"]),
    page!("ethernet", Some("network"), "ethernet", "Ethernet", "Ethernet",
        "Kabelverbinding en IP-adres", "Wired connection and IP address",
        ["ethernet", "kabel", "lan", "bekabeld", "ip-adres", "ip"],
        ["ethernet", "cable", "lan", "wired", "ip address", "ip"]),
    page!("mobile", Some("network"), "phone", "Mobiel netwerk", "Mobile network",
        "Simkaart, mobiele gegevens, roaming en APN", "SIM card, mobile data, roaming and APN",
        ["mobiel netwerk", "mobiele gegevens", "simkaart", "4g", "5g", "lte", "roaming", "apn", "cellulair"],
        ["mobile network", "mobile data", "sim card", "4g", "5g", "lte", "roaming", "apn", "cellular"]),
    page!("vpn", Some("network"), "vpn", "VPN", "VPN",
        "VPN-verbindingen toevoegen en verbinden", "Add and connect VPN connections",
        ["vpn", "openvpn", "wireguard", "tunnel", "beveiligde verbinding"],
        ["vpn", "openvpn", "wireguard", "tunnel", "secure connection"]),
    page!("hotspot", Some("network"), "hotspot", "Mobiele hotspot", "Mobile hotspot",
        "Je internetverbinding delen met andere apparaten", "Share your internet connection with other devices",
        ["mobiele hotspot", "hotspot", "tethering", "internet delen", "toegangspunt"],
        ["mobile hotspot", "hotspot", "tethering", "share internet", "access point"]),
    page!("flightmode", Some("network"), "airplane", "Vliegtuigmodus", "Flight mode",
        "Alle draadloze communicatie stoppen", "Stop all wireless communication",
        ["vliegtuigmodus", "vliegtuig", "draadloos", "offline"],
        ["flight mode", "airplane mode", "aeroplane mode", "wireless", "offline"]),
    page!("proxy", Some("network"), "globe", "Proxy", "Proxy",
        "Een proxyserver voor internet, automatisch of handmatig", "A proxy server for the internet, automatic or manual",
        ["proxy", "proxyserver", "installatiescript", "pac"],
        ["proxy", "proxy server", "setup script", "pac"]),
    page!("network-advanced", Some("network"), "network", "Geavanceerde netwerkinstellingen", "Advanced network settings",
        "Netwerkadapters en controle op internetverbinding", "Network adapters and connectivity checking",
        ["geavanceerd", "netwerkadapters", "adapter", "adapter uitschakelen", "internetverbinding controleren"],
        ["advanced", "network adapters", "adapter", "disable adapter", "connectivity check"]),
    page!("sharing-advanced", Some("network-advanced"), "share", "Geavanceerde instellingen voor delen", "Advanced sharing settings",
        "Mediastreaming en delen op het netwerk", "Media streaming and sharing on the network",
        ["delen", "mediastreaming", "dlna", "netwerkdetectie"],
        ["sharing", "media streaming", "dlna", "network discovery"]),

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
    page!("app-advanced", Some("installed"), "settings", "Geavanceerde opties", "Advanced options",
        "Machtigingen, opslag en standaardkoppelingen van een app", "Permissions, storage and default links of an app",
        ["geavanceerde opties", "app-machtigingen"],
        ["advanced options", "app permissions"]),
    page!("defaultapps", Some("apps"), "open", "Standaard-apps", "Default apps",
        "Welke app opent je webpagina’s, muziek en foto’s", "Which app opens your web pages, music and photos",
        ["standaard", "standaardapp", "browser", "openen met", "standaardbrowser", "mail", "muziekspeler"],
        ["default", "default apps", "browser", "open with", "default browser", "mail", "music player"]),
    page!("defaultapps-app", Some("defaultapps"), "open", "App", "App",
        "Standaardbestandstypen of -koppelingstypen instellen", "Set default file types or link types",
        [], []),
    page!("startup", Some("apps"), "startup", "Opstarten", "Startup",
        "Apps die starten wanneer je je aanmeldt", "Apps that start when you sign in",
        ["opstarten", "automatisch starten", "autostart", "aanmelden", "opstartprogramma"],
        ["startup", "autostart", "start automatically", "sign in"]),

    // ---- Accounts
    page!("accounts", None, "user", "Accounts", "Accounts",
        "Je account, naam en profielfoto", "Your account, name and picture",
        ["account", "gebruiker", "profiel", "naam", "foto", "wachtwoord", "aanmelden"],
        ["account", "user", "profile", "name", "picture", "password", "sign in"]),
    page!("yourinfo", Some("accounts"), "user", "Je info", "Your info",
        "Je foto en je naam", "Your picture and your name",
        ["je info", "foto", "profielfoto", "naam", "avatar"],
        ["your info", "picture", "profile picture", "name", "avatar"]),
    page!("email-accounts", Some("accounts"), "mail", "E-mail en accounts", "Email & accounts",
        "Online accounts voor e-mail, agenda en contactpersonen", "Online accounts for email, calendar and contacts",
        ["e-mail", "accounts", "online accounts", "google", "microsoft", "nextcloud", "agenda", "contactpersonen"],
        ["email", "accounts", "online accounts", "google", "microsoft", "nextcloud", "calendar", "contacts"]),
    page!("signin", Some("accounts"), "key", "Aanmeldingsopties", "Sign-in options",
        "Wachtwoord, vingerafdruk, automatisch aanmelden, vergrendelen", "Password, fingerprint, automatic sign-in, lock",
        ["aanmeldingsopties", "wachtwoord", "vingerafdruk", "automatisch aanmelden", "vergrendelen", "wachtwoord wijzigen"],
        ["sign-in options", "password", "fingerprint", "automatic sign-in", "lock", "change password"]),
    page!("otherusers", Some("accounts"), "people", "Andere gebruikers", "Other users",
        "Accounts van andere gebruikers op deze pc", "Accounts of other users on this PC",
        ["andere gebruikers", "gebruiker toevoegen", "account toevoegen", "administrator", "standaardgebruiker"],
        ["other users", "add user", "add account", "administrator", "standard user"]),

    // ---- Tijd en taal
    page!("time", None, "clock", "Tijd en taal", "Time & language",
        "Datum, tijd, tijdzone en taal", "Date, time, time zone and language",
        ["tijd", "taal", "datum", "klok"], ["time", "language", "date", "clock"]),
    page!("datetime", Some("time"), "calendar", "Datum en tijd", "Date & time",
        "Tijdzone, 24-uursnotatie, seconden, eerste dag van de week", "Time zone, 24-hour clock, seconds, first day of week",
        ["datum", "tijd", "klok", "tijdzone", "24 uur", "24-uurs", "am pm", "seconden", "week", "maandag", "zondag", "kalender", "ntp", "synchroniseren", "zomertijd"],
        ["date", "time", "clock", "time zone", "24 hour", "24-hour", "am pm", "seconds", "week", "monday", "sunday", "calendar", "ntp", "sync", "daylight"]),
    page!("language", Some("time"), "language", "Taal en regio", "Language & region",
        "Weergavetaal: Nederlands of English", "Display language: Dutch or English",
        ["taal", "nederlands", "engels", "english", "regio", "land", "vertaling", "weergavetaal", "taalpakket"],
        ["language", "dutch", "english", "region", "country", "translation", "display language", "language pack"]),
    page!("language-options", Some("language"), "keyboard", "Taalopties", "Language options",
        "Toetsenborden en invoermethoden", "Keyboards and input methods",
        ["taalopties", "toetsenbord", "toetsenbordindeling", "indeling", "invoermethode", "qwerty", "azerty"],
        ["language options", "keyboard", "keyboard layout", "layout", "input method", "qwerty", "azerty"]),
    page!("formats", Some("language"), "calendar", "Regionale notatie", "Regional format",
        "Datums, tijden, getallen en de eerste dag van de week", "Dates, times, numbers and the first day of the week",
        ["regionale notatie", "notaties", "datumnotatie", "tijdnotatie", "24 uur", "eerste dag van de week", "getallen", "valuta"],
        ["regional format", "formats", "date format", "time format", "24 hour", "first day of week", "numbers", "currency"]),
    page!("language-admin", Some("language"), "shield", "Beheerinstellingen voor taal", "Administrative language settings",
        "Taal en notaties van het aanmeldingsscherm", "Language and formats of the sign-in screen",
        ["beheerinstellingen", "aanmeldingsscherm", "systeemtaal", "landinstelling", "welkomstscherm"],
        ["administrative", "sign-in screen", "system language", "locale", "welcome screen"]),
    page!("typing", Some("time"), "keyboard", "Typen", "Typing",
        "Aanraaktoetsenbord, toetsenbordinstellingen en sneltoetsen", "Touch keyboard, keyboard settings and keyboard shortcuts",
        ["typen", "aanraaktoetsenbord", "schermtoetsenbord", "toetsenbord"],
        ["typing", "touch keyboard", "on-screen keyboard", "keyboard"]),
    page!("keyboard-advanced", Some("typing"), "keyboard", "Geavanceerde toetsenbordinstellingen", "Advanced keyboard settings",
        "Invoermethode, tekenherhaling, Windows-toets", "Input method, character repeat, Windows key",
        ["geavanceerd toetsenbord", "tekenherhaling", "herhalen", "vertraging", "samenstellingstoets", "alt gr", "windows-toets"],
        ["advanced keyboard", "character repeat", "repeat", "delay", "compose key", "alt gr", "windows key"]),
    page!("shortcuts", Some("typing"), "keyboard", "Sneltoetsen", "Keyboard shortcuts",
        "Sneltoetsen bekijken en wijzigen, aangepaste sneltoetsen", "View and change keyboard shortcuts, custom shortcuts",
        ["sneltoetsen", "sneltoets", "toetscombinatie", "aangepaste sneltoets"],
        ["keyboard shortcuts", "shortcut", "key combination", "custom shortcut"]),

    // ---- Toegankelijkheid
    page!("accessibility", None, "accessibility", "Toegankelijkheid", "Accessibility",
        "Tekstgrootte, visuele effecten, contrast", "Text size, visual effects, contrast",
        ["toegankelijkheid", "slechtziend", "leesbaarheid", "hulp"], ["accessibility", "low vision", "readability", "ease of access"]),
    page!("textsize", Some("accessibility"), "zoom-in", "Tekstgrootte", "Text size",
        "Tekst groter maken", "Make text bigger",
        ["tekst", "groter", "lettergrootte", "letters", "vergroten", "grootte", "lezen", "zoom", "tekstschaal"],
        ["text", "bigger", "font size", "larger", "enlarge", "size", "read", "zoom", "text scaling"]),
    page!("visualeffects", Some("accessibility"), "sparkle", "Visuele effecten", "Visual effects",
        "Schuifbalken, transparantie, animaties", "Scroll bars, transparency, animations",
        ["animatie", "animaties", "beweging", "transparantie", "effecten", "rustig"],
        ["animation", "animations", "motion", "transparency", "effects", "reduce motion"]),
    page!("mousepointer", Some("accessibility"), "mouse", "Muisaanwijzer en aanraken", "Mouse pointer and touch",
        "Grootte en zichtbaarheid van de muisaanwijzer", "Size and visibility of the mouse pointer",
        ["muisaanwijzer", "aanwijzer", "cursor", "grootte", "groter"],
        ["mouse pointer", "pointer", "cursor", "size", "bigger"]),
    page!("textcursor", Some("accessibility"), "text-cursor", "Tekstcursor", "Text cursor",
        "Knipperen en snelheid van de tekstcursor", "Blinking and speed of the text cursor",
        ["tekstcursor", "cursor", "knipperen", "invoegpositie"],
        ["text cursor", "cursor", "blink", "caret"]),
    page!("magnifier", Some("accessibility"), "magnifier", "Vergrootglas", "Magnifier",
        "Delen van het scherm vergroten", "Zoom in on parts of the screen",
        ["vergrootglas", "vergroten", "zoomen", "inzoomen", "loep", "slechtziend", "dradenkruis", "kleuren omkeren"],
        ["magnifier", "magnify", "zoom", "zoom in", "lens", "low vision", "crosshairs", "invert colours", "invert colors"]),
    page!("contrast", Some("accessibility"), "eye", "Contrastthema’s", "Contrast themes",
        "Meer contrast voor beter leesbare tekst", "More contrast for easier reading",
        ["contrast", "hoog contrast", "leesbaar", "kleurenblind"], ["contrast", "high contrast", "readable", "colour blind", "color blind"]),
    page!("narrator", Some("accessibility"), "narrator", "Verteller", "Narrator",
        "Schermlezer die voorleest wat er op het scherm staat", "Screen reader that reads out what’s on the screen",
        ["verteller", "schermlezer", "voorlezen", "orca", "blind"],
        ["narrator", "screen reader", "read aloud", "orca", "blind"]),
    page!("a11y-audio", Some("accessibility"), "volume", "Audio", "Audio",
        "Het scherm laten knipperen bij geluiden, volume", "Flash the screen for sounds, volume",
        ["audio", "geluid", "knipperen", "visuele melding", "slechthorend", "doof"],
        ["audio", "sound", "flash", "visual alert", "hard of hearing", "deaf"]),
    page!("a11y-keyboard", Some("accessibility"), "keyboard", "Toetsenbord", "Keyboard",
        "Plaktoetsen, filtertoetsen, wisseltoetsen", "Sticky keys, filter keys, toggle keys",
        ["plaktoetsen", "filtertoetsen", "wisseltoetsen", "trage toetsen", "schermtoetsenbord"],
        ["sticky keys", "filter keys", "toggle keys", "slow keys", "on-screen keyboard"]),
    page!("a11y-mouse", Some("accessibility"), "mouse", "Muis", "Mouse",
        "Muistoetsen, dubbelklikken, klikken door te zweven", "Mouse keys, double-click, hover click",
        ["muistoetsen", "dubbelkliksnelheid", "klikken door te zweven", "secundaire klik", "venster activeren", "muistoetsen snelheid"],
        ["mouse keys", "double-click speed", "hover click", "secondary click", "activate window", "mouse keys speed"]),

    // ---- Privacy en beveiliging
    page!("privacy", None, "shield", "Privacy en beveiliging", "Privacy & security",
        "Apparaatbeveiliging, app-machtigingen, activiteitengeschiedenis", "Device security, app permissions, activity history",
        ["privacy", "beveiliging", "recente bestanden", "geschiedenis", "wissen", "veilig", "locatie", "camera", "microfoon", "zoeken"],
        ["privacy", "security", "recent files", "history", "clear", "safe", "location", "camera", "microphone", "search"]),
    page!("security", Some("privacy"), "shield", "Apparaatbeveiliging", "Device security",
        "Beveiligingsniveau van het apparaat en Beveiligd opstarten", "Security level of the device and Secure Boot",
        ["apparaatbeveiliging", "beveiliging", "beveiligd opstarten", "secure boot", "tpm", "firmware"],
        ["device security", "security", "secure boot", "tpm", "firmware"]),
    page!("diagnostics", Some("privacy"), "chart", "Diagnostische gegevens en feedback", "Diagnostics & feedback",
        "Foutrapporten en systeemrapporten voor Canonical", "Error reports and system reports for Canonical",
        ["diagnostische gegevens", "feedback", "foutrapporten", "rapporten", "telemetrie"],
        ["diagnostic data", "feedback", "error reports", "reports", "telemetry"]),
    page!("activity", Some("privacy"), "history", "Activiteitengeschiedenis", "Activity history",
        "Recente bestanden bijhouden en wissen", "Keep track of recent files and clear them",
        ["activiteitengeschiedenis", "geschiedenis", "recente bestanden", "wissen"],
        ["activity history", "history", "recent files", "clear"]),
    page!("searchpermissions", Some("privacy"), "search", "Zoekmachtigingen", "Search permissions",
        "Zoekresultaten van apps in het overzicht", "Search results from apps in the overview",
        ["zoekmachtigingen", "zoeken", "zoekresultaten", "zoekproviders"],
        ["search permissions", "search", "search results", "search providers"]),
    page!("searching", Some("privacy"), "search", "Deze pc doorzoeken", "Searching this PC",
        "Welke mappen worden geïndexeerd voor zoeken", "Which folders are indexed for search",
        ["indexeren", "index", "zoeklocaties", "mappen doorzoeken"],
        ["indexing", "index", "search locations", "search folders"]),
    page!("location", Some("privacy"), "location", "Locatie", "Location",
        "Locatieservices en apps die je locatie gebruiken", "Location services and apps that use your location",
        ["locatie", "locatieservices", "gps", "plaats"],
        ["location", "location services", "gps", "place"]),
    page!("camera", Some("privacy"), "camera", "Camera", "Camera",
        "Cameratoegang voor apps", "Camera access for apps",
        ["camera", "webcam", "cameratoegang"],
        ["camera", "webcam", "camera access"]),
    page!("microphone", Some("privacy"), "mic", "Microfoon", "Microphone",
        "Microfoontoegang voor apps", "Microphone access for apps",
        ["microfoon", "mic", "microfoontoegang", "opnemen"],
        ["microphone", "mic", "microphone access", "record"]),

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

/// The caps key a page needs (notes/claude/settings-parity.md §2, §6.2).
pub static NEEDS: &[(&str, &str)] = &[
    ("display-advanced", "gnome"),
    ("nightlight", "gnome"),
    ("screentime", "gnome"),
    ("multitasking", "gnome"),
    ("remotedesktop", "gnome"),
    ("touchpad", "touchpad"),
    ("pen", "tablet"),
    ("usb", "thunderbolt"),
    ("mobile", "modem"),
    ("shortcuts", "gnome"),
    ("magnifier", "gnome"),
    ("narrator", "gnome"),
];

/// The keys of `caps.get` a page may need (host/src/services/caps.rs KEYS without "session"; tests/i18n/test_native.py
/// compares the two lists).
#[cfg_attr(not(test), allow(dead_code))]
pub static KNOWN_NEEDS: &[&str] = &[
    "gnome", "mouse", "touchpad", "pointingstick", "tablet", "touchscreen", "thunderbolt", "modem", "ambientLight",
    "accelerometer", "privacyScreen", "backlight", "battery", "hibernate", "fingerprint", "colorSensor", "geoclue",
    "tracker", "usbguard", "sshServer", "userShare", "malcontent", "realmd", "rygel", "tecla", "orca", "simpleScan",
    "securityCenter", "insights", "whoopsie",
];

/// Pages that show one item and need its parameter (amendment 26).
pub static DATA_PAGES: &[&str] = &["sound-device", "app-notifications", "printer", "wifi-network", "app-advanced", "defaultapps-app"];

/// Old or alternative ids that deep links may use (registry.js SETTINGS_ALIASES) and Windows' ms-settings: names.
/// None points at a data page.
pub static ALIASES: &[(&str, &str)] = &[
    ("brightness", "display"), ("monitors", "display"), ("volume", "sound"), ("audio", "sound"),
    ("battery", "power"), ("energy", "power"), ("disk", "storage"), ("info", "about"), ("devices", "bluetooth"),
    ("internet", "network"), ("wallpaper", "background"), ("accent", "colors"), ("personalize", "personalization"),
    ("default-apps", "defaultapps"), ("defaults", "defaultapps"), ("autostart", "startup"), ("user", "accounts"),
    ("clock", "datetime"), ("date", "datetime"), ("lang", "language"), ("updates", "update"), ("backup", "update"),
    ("reset", "update"), ("recovery", "update"), ("ease", "accessibility"), ("text", "textsize"), ("animations", "visualeffects"),
    ("taskmgr", "storage"), ("theme", "themes"),
    // ids other modules (shell, desktop, flyouts, host health actions) or Windows' ms-settings: names may use
    ("personalisation", "personalization"), ("colours", "colors"), ("colour", "colors"), ("color", "colors"),
    ("lockscreen", "personalization"), ("fonts", "personalization"), ("start-menu", "start"), ("startmenu", "start"),
    ("taskbar-settings", "taskbar"), ("notification", "notifications"), ("meldingen", "notifications"),
    ("date-time", "datetime"), ("dateandtime", "datetime"), ("datum-tijd", "datetime"), ("region", "language"),
    ("regionlanguage", "language"), ("apps-features", "installed"), ("appsfeatures", "installed"),
    ("installed-apps", "installed"), ("installedapps", "installed"), ("programs", "installed"),
    ("appsfeatures-app", "installed"), ("startup-apps", "startup"), ("startupapps", "startup"),
    ("your-info", "yourinfo"), ("account", "accounts"), ("wireless", "wifi"), ("wi-fi", "wifi"), ("wired", "ethernet"),
    ("network-status", "network"), ("network-wifi", "wifi"), ("network-ethernet", "ethernet"),
    ("network-wifisettings", "wifi-known"), ("network-advancedsettings", "network-advanced"),
    ("network-vpn", "vpn"), ("network-proxy", "proxy"), ("network-mobilehotspot", "hotspot"),
    ("network-airplanemode", "flightmode"), ("airplane", "flightmode"), ("network-cellular", "mobile"),
    ("bluetoothdevices", "bluetooth-devices"), ("connecteddevices", "bluetooth-devices"),
    ("mousetouchpad", "mouse"), ("devices-touchpad", "touchpad"), ("power-sleep", "power"), ("powersleep", "power"),
    ("sleep", "power"), ("batterysaver", "power"), ("storage-sense", "storagesense"), ("storagepolicies", "storagesense"),
    ("clipboardhistory", "clipboard"), ("about-pc", "about"), ("pc", "about"), ("windows-update", "update"),
    ("windowsupdate", "update"), ("privacy-security", "privacy"), ("privacy-general", "privacy"),
    ("text-size", "textsize"), ("visual-effects", "visualeffects"), ("contrast-themes", "contrast"),
    ("desktop-icons", "desktopicons"), ("night-light", "nightlight"),
    ("apps-volume", "volume-mixer"), ("developers", "system-advanced"), ("optionalfeatures", "system-advanced"),
    ("emailandaccounts", "email-accounts"), ("signinoptions", "signin"), ("regionformatting", "formats"),
    ("keyboard", "keyboard-advanced"), ("easeofaccess-display", "textsize"),
    ("easeofaccess-visualeffects", "visualeffects"), ("easeofaccess-mousepointer", "mousepointer"),
    ("easeofaccess-cursor", "textcursor"), ("easeofaccess-magnifier", "magnifier"),
    ("easeofaccess-highcontrast", "contrast"), ("easeofaccess-narrator", "narrator"),
    ("easeofaccess-audio", "a11y-audio"), ("easeofaccess-keyboard", "a11y-keyboard"),
    ("easeofaccess-mouse", "a11y-mouse"), ("privacy-location", "location"), ("privacy-webcam", "camera"),
    ("privacy-microphone", "microphone"), ("privacy-feedback", "diagnostics"),
    ("privacy-activityhistory", "activity"), ("search-permissions", "searchpermissions"),
    ("cortana-windowssearch", "searching"), ("windowsdefender", "security"),
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

/// The top-level page of a page (itself when it has no parent): the navigation pane's item.
pub fn top_of(id: &str) -> &'static str {
    match ancestors(id).first() {
        Some(p) => p.id,
        None => "home",
    }
}

/// The page and its parents, the top-level page first (the breadcrumb); empty for an unknown id. A chain is at most
/// three deep (a test checks it), so the walk stops there.
pub fn ancestors(id: &str) -> Vec<&'static PageInfo> {
    let mut chain = Vec::new();
    let mut cur = page(id);
    while let Some(p) = cur {
        if chain.len() >= 4 || chain.iter().any(|c: &&PageInfo| c.id == p.id) {
            break;
        }
        chain.push(p);
        cur = p.parent.and_then(page);
    }
    chain.reverse();
    chain
}

/// The direct sub-pages of a page, in navigation order (data pages included: the caller decides).
pub fn children(parent: &str) -> Vec<&'static PageInfo> {
    PAGES.iter().filter(|p| p.parent == Some(parent)).collect()
}

/// The caps key a page needs, if any.
pub fn needs(id: &str) -> Option<&'static str> {
    NEEDS.iter().find(|(p, _)| *p == id).map(|(_, k)| *k)
}

/// Whether a page shows one item and needs its parameter.
pub fn is_data(id: &str) -> bool {
    DATA_PAGES.contains(&id)
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
/// substring), keywords and description in the language, weaker in the other language; every word must match. Data
/// pages are never results.
#[cfg_attr(not(test), allow(dead_code))]
pub fn search(query: &str, en: bool, limit: usize) -> Vec<&'static PageInfo> {
    search_visible(query, en, limit, &|_| true)
}

/// [`search`] among the pages `visible` keeps (the app leaves out pages whose `needs` caps key is false).
pub fn search_visible(query: &str, en: bool, limit: usize, visible: &dyn Fn(&PageInfo) -> bool) -> Vec<&'static PageInfo> {
    let q = norm(query);
    if q.is_empty() {
        return Vec::new();
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut scored: Vec<(&'static PageInfo, u32)> = Vec::new();
    for page in PAGES.iter().filter(|p| !is_data(p.id) && visible(p)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_a_builder_and_a_short_acyclic_parent_chain() {
        let mut ids = std::collections::HashSet::new();
        for p in PAGES {
            assert!(ids.insert(p.id), "{} twice", p.id);
            assert!(crate::pages::builder(p.id).is_some(), "{} has no builder", p.id);
            let chain = ancestors(p.id);
            assert!(!chain.is_empty() && chain.len() <= 3, "{}: {} levels", p.id, chain.len());
            assert_eq!(chain.last().unwrap().id, p.id);
            assert!(chain[0].parent.is_none(), "{}: the chain ends at a top-level page", p.id);
            if let Some(parent) = p.parent {
                assert!(page(parent).is_some(), "{}: unknown parent {parent}", p.id);
            }
            assert!(!p.title.0.is_empty() && !p.title.1.is_empty() && !p.description.0.is_empty(), "{}", p.id);
            assert!(!p.title.0.contains('—') && !p.description.0.contains('—') && !p.description.1.contains('—'), "{}: no em-dash", p.id);
        }
        assert_eq!(PAGES.len(), 92);
    }

    #[test]
    fn needs_and_data_pages_and_aliases_are_consistent() {
        for (id, key) in NEEDS {
            assert!(page(id).is_some(), "NEEDS names {id}");
            assert!(KNOWN_NEEDS.contains(key), "{id} needs {key}, which caps.get does not report");
        }
        for id in DATA_PAGES {
            assert!(page(id).is_some_and(|p| p.parent.is_some()), "{id}");
        }
        for (alias, target) in ALIASES {
            assert!(page(target).is_some(), "{alias} → {target}");
            assert!(!is_data(target), "{alias} points at the data page {target}");
            assert!(page(alias).is_none(), "{alias} is a page id itself");
        }
        assert_eq!(resolve("nightlight"), Some("nightlight"));
        assert_eq!(resolve("night-light"), Some("nightlight"));
        assert_eq!(resolve("sound-devices"), Some("sound-devices"));
        assert_eq!(resolve("ms-settings-is-not-an-id"), None);
        assert_eq!(resolve("NETWORK-VPN"), Some("vpn"));
        assert_eq!(resolve("easeofaccess-narrator"), Some("narrator"));
    }

    #[test]
    fn breadcrumbs_and_the_navigation_item_follow_the_parent_chain() {
        let ids = |id: &str| ancestors(id).iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(ids("sharing-advanced"), ["network", "network-advanced", "sharing-advanced"]);
        assert_eq!(ids("shortcuts"), ["time", "typing", "shortcuts"]);
        assert_eq!(ids("app-advanced"), ["apps", "installed", "app-advanced"]);
        assert_eq!(ids("defaultapps-app"), ["apps", "defaultapps", "defaultapps-app"]);
        assert_eq!(ids("printer"), ["bluetooth", "printers", "printer"]);
        assert_eq!(ids("display"), ["system", "display"]);
        assert_eq!(ids("home"), ["home"]);
        assert!(ancestors("nope").is_empty());
        assert_eq!(top_of("sharing-advanced"), "network");
        assert_eq!(top_of("nightlight"), "system");
        assert_eq!(top_of("system"), "system");
        assert_eq!(top_of("nope"), "home");
        let system: Vec<&str> = children("system").iter().map(|p| p.id).collect();
        assert_eq!(
            system,
            ["display", "sound", "notifications", "screentime", "power", "storage", "multitasking", "system-advanced", "remotedesktop", "clipboard", "about"]
        );
    }

    fn first(q: &str, en: bool) -> &'static str {
        search(q, en, 8).first().map(|p| p.id).unwrap_or("")
    }

    #[test]
    fn search_finds_the_new_pages_and_hides_data_and_missing_ones() {
        assert_eq!(first("muis", false), "mouse");
        assert_eq!(first("mouse", true), "mouse");
        assert_eq!(first("touchpad", false), "touchpad");
        assert_eq!(first("printer", true), "printers");
        assert_eq!(first("vpn", false), "vpn");
        assert_eq!(first("proxy", true), "proxy");
        assert_eq!(first("nachtlampje", false), "nightlight");
        assert_eq!(first("opslaginzicht", false), "storagesense");
        for q in ["toetsenbord", "sneltoetsen", "vergrootglas", "verteller", "vliegtuigmodus", "hotspot"] {
            assert!(!search(q, false, 8).is_empty(), "{q}");
        }
        for q in ["printer", "eigenschappen", "wi-fi-eigenschappen", "meldingen van app"] {
            assert!(search(q, false, 40).iter().all(|p| !is_data(p.id)), "{q} finds a data page");
        }
        // a page whose caps key is false is hidden
        let no_touchpad = |p: &PageInfo| needs(p.id) != Some("touchpad");
        assert!(search_visible("touchpad", false, 8, &no_touchpad).iter().all(|p| p.id != "touchpad"));
        // the integration step's keywords (notes/claude/settings-i.md, package host-settings H10)
        assert_eq!(first("hdr", false), "display");
        assert_eq!(first("hdr", true), "display");
        assert_eq!(first("ntp", false), "datetime");
        assert_eq!(first("ntp", true), "datetime");
        assert_eq!(first("dradenkruis", false), "magnifier");
        assert_eq!(first("storage sense", true), "storagesense");
        assert_eq!(first("prullenbak leegmaken", false), "storagesense");
        assert_eq!(first("grijswaarden", false), "screentime");
        assert_eq!(first("weergavetaal", false), "language");
        assert_eq!(first("welkomstscherm", false), "language-admin");
        assert_eq!(first("wayland", true), "about");
        assert!(search("microfoon", false, 8).iter().any(|p| p.id == "privacy"));
        // Deze pc doorzoeken needs no caps key: without Tracker it still shows WinPoint's own index rows
        assert_eq!(needs("searching"), None);
        assert!(search("app", false, 40).iter().all(|p| p.id != "defaultapps-app"), "a data page");
        // the existing queries still find their pages
        assert_eq!(first("wifi", false), "wifi");
        assert_eq!(first("achtergrond", false), "background");
        assert_eq!(first("geluid", false), "sound");
        assert_eq!(first("bluetooth", true), "bluetooth");
    }
}
