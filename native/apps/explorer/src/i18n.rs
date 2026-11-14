//! Every user-facing text of the Explorer (vendor/ui/js/apps/explorer/i18n.js), Dutch and English, by key.
//! `s(key)` gives the text in the UI language, `f(key, &[("name", ..)])` fills `{name}` placeholders and
//! `n(key, count, &[..])` picks the `.one` / `.other` form and fills `{n}` too. Builders of ops.rs, dialogs.rs,
//! details.rs, picker.rs and transfer.rs use the same table, so the wording stays the WebKit app's.
use std::collections::HashMap;

/// (key, nl, en). Plural texts are two keys: "items.one" and "items.other".
const TEXTS: &[(&str, &str, &str)] = &[
    ("appName", "Verkenner", "File Explorer"),
    ("home", "Start", "Home"),
    ("gallery", "Galerij", "Gallery"),
    ("thisPC", "Deze pc", "This PC"),
    ("trash", "Prullenbak", "Recycle Bin"),
    ("desktop", "Bureaublad", "Desktop"),
    ("downloads", "Downloads", "Downloads"),
    ("documents", "Documenten", "Documents"),
    ("pictures", "Afbeeldingen", "Pictures"),
    ("music", "Muziek", "Music"),
    ("videos", "Video's", "Videos"),
    ("homeFolder", "Persoonlijke map", "Home folder"),
    ("root", "Systeemschijf", "System disk"),
    ("search", "Zoekresultaten", "Search results"),
    // tabs + title bar
    ("newTab", "Nieuw tabblad (Ctrl+T)", "New tab (Ctrl+T)"),
    ("closeTab", "Tabblad sluiten (Ctrl+W)", "Close tab (Ctrl+W)"),
    ("tabs", "Tabbladen", "Tabs"),
    ("tabMoveFailed", "Tabblad kon niet worden verplaatst", "Couldn\u{2019}t move the tab"),
    (
        "tabKeptChanged",
        "Het tabblad is in het andere venster geopend. De bron is behouden omdat die tijdens het verplaatsen veranderde.",
        "The tab was opened in the other window. The source was kept because it changed during the move.",
    ),
    ("tabRenaming", "Rond het hernoemen eerst af voordat je dit tabblad verplaatst.", "Finish renaming before moving this tab."),
    ("tabGone", "Dit tabblad bestaat niet meer.", "This tab no longer exists."),
    (
        "tabInvalid",
        "Dit tabblad kan niet worden verplaatst, omdat de gegevens ervan ongeldig zijn.",
        "This tab couldn\u{2019}t be moved because its data isn\u{2019}t valid.",
    ),
    ("tabLoadFailed", "Dit tabblad kon in dit venster niet worden geopend.", "This tab couldn\u{2019}t be opened in this window."),
    // address row
    ("back", "Terug (Alt+pijl-links)", "Back (Alt+Left Arrow)"),
    ("backTo", "Terug naar {name} (Alt+pijl-links)", "Back to {name} (Alt+Left Arrow)"),
    ("forward", "Vooruit (Alt+pijl-rechts)", "Forward (Alt+Right Arrow)"),
    ("forwardTo", "Vooruit naar {name} (Alt+pijl-rechts)", "Forward to {name} (Alt+Right Arrow)"),
    ("up", "Omhoog (Alt+pijl-omhoog)", "Up (Alt+Up Arrow)"),
    ("upTo", "Omhoog naar \u{2018}{name}\u{2019} (Alt+pijl-omhoog)", "Up to \u{201c}{name}\u{201d} (Alt+Up Arrow)"),
    ("refresh", "Vernieuwen (F5)", "Refresh (F5)"),
    ("address", "Adresbalk", "Address bar"),
    ("recentLocations", "Recente locaties", "Recent locations"),
    ("subfolders", "Submappen van {name}", "Subfolders of {name}"),
    ("noSubfolders", "Geen submappen", "No subfolders"),
    ("hiddenParts", "Meer locaties", "More locations"),
    ("searchIn", "Zoeken in {name}", "Search {name}"),
    ("searchEverywhere", "Overal zoeken met Alles", "Search everywhere with Alles"),
    (
        "cantFind",
        "Kan \u{2018}{path}\u{2019} niet vinden. Controleer de spelling en probeer het opnieuw.",
        "Can\u{2019}t find \u{201c}{path}\u{201d}. Check the spelling and try again.",
    ),
    // command bar
    ("new", "Nieuw", "New"),
    ("newHint", "Een nieuw item maken in de huidige locatie", "Create a new item in the current location"),
    ("newFolder", "Map", "Folder"),
    ("newText", "Tekstdocument", "Text Document"),
    ("cut", "Knippen (Ctrl+X)", "Cut (Ctrl+X)"),
    ("copy", "Kopi\u{eb}ren (Ctrl+C)", "Copy (Ctrl+C)"),
    ("paste", "Plakken (Ctrl+V)", "Paste (Ctrl+V)"),
    ("rename", "Naam wijzigen (F2)", "Rename (F2)"),
    ("share", "Delen", "Share"),
    ("delete", "Verwijderen (Del)", "Delete (Del)"),
    ("sort", "Sorteren", "Sort"),
    ("view", "Weergeven", "View"),
    ("ctxView", "Beeld", "View"),
    ("more", "Meer weergeven", "See more"),
    ("details", "Details", "Details"),
    ("detailsHint", "Het detailvenster weergeven of verbergen (Alt+Shift+P)", "Show or hide the details pane (Alt+Shift+P)"),
    ("emptyTrash", "Prullenbak leegmaken", "Empty Recycle Bin"),
    ("restoreAll", "Alle items terugzetten", "Restore all items"),
    ("restoreSel", "Geselecteerde items terugzetten", "Restore the selected items"),
    ("commandBar", "Opdrachten", "Commands"),
    // sort / group / view menus
    ("colName", "Naam", "Name"),
    ("colModified", "Gewijzigd op", "Date modified"),
    ("colType", "Type", "Type"),
    ("colSize", "Grootte", "Size"),
    ("colOrig", "Oorspronkelijke locatie", "Original location"),
    ("colDeleted", "Verwijderd op", "Date deleted"),
    ("colLocation", "Locatie", "Location"),
    ("colOpened", "Datum geopend", "Date accessed"),
    ("asc", "Oplopend", "Ascending"),
    ("desc", "Aflopend", "Descending"),
    ("groupBy", "Groeperen op", "Group by"),
    ("none", "(Geen)", "(None)"),
    ("xl", "Extra grote pictogrammen", "Extra large icons"),
    ("large", "Grote pictogrammen", "Large icons"),
    ("medium", "Middelgrote pictogrammen", "Medium icons"),
    ("small", "Kleine pictogrammen", "Small icons"),
    ("list", "Lijst", "List"),
    ("detailsView", "Details", "Details"),
    ("tiles", "Tegels", "Tiles"),
    ("content", "Inhoud", "Content"),
    ("compact", "Compacte weergave", "Compact view"),
    ("show", "Weergeven", "Show"),
    ("navPane", "Navigatievenster", "Navigation pane"),
    ("detailsPane", "Detailvenster", "Details pane"),
    ("previewPane", "Voorbeeldvenster", "Preview pane"),
    ("job_copy.one", "1 item kopi\u{eb}ren naar {name}", "Copying 1 item to {name}"),
    ("job_copy.other", "{n} items kopi\u{eb}ren naar {name}", "Copying {n} items to {name}"),
    ("job_move.one", "1 item verplaatsen naar {name}", "Moving 1 item to {name}"),
    ("job_move.other", "{n} items verplaatsen naar {name}", "Moving {n} items to {name}"),
    ("job_paste.one", "1 item plakken in {name}", "Pasting 1 item into {name}"),
    ("job_paste.other", "{n} items plakken in {name}", "Pasting {n} items into {name}"),
    ("job_compress", "Comprimeren tot ZIP-bestand\u{2026}", "Compressing to ZIP file\u{2026}"),
    ("job_extract", "Alles uitpakken\u{2026}", "Extracting all\u{2026}"),
    ("noPreview", "Er is geen voorbeeld beschikbaar.", "No preview available."),
    ("previewHint", "Selecteer het bestand waarvan je een voorbeeld wilt bekijken.", "Select a file to preview."),
    ("checkboxes", "Selectievakjes voor items", "Item check boxes"),
    ("extensions", "Bestandsnaamextensies", "File name extensions"),
    ("hidden", "Verborgen items", "Hidden items"),
    ("selectAll", "Alles selecteren", "Select all"),
    ("selectNone", "Niets selecteren", "Select none"),
    ("invert", "Selectie omkeren", "Invert selection"),
    ("properties", "Eigenschappen", "Properties"),
    ("options", "Opties", "Options"),
    // context menu
    ("open", "Openen", "Open"),
    ("openNewTab", "Openen in nieuw tabblad", "Open in new tab"),
    ("openWith", "Openen met", "Open with"),
    ("noApps", "Geen apps gevonden", "No apps found"),
    ("pin", "Vastmaken aan Snelle toegang", "Pin to Quick access"),
    ("unpin", "Losmaken van Snelle toegang", "Unpin from Quick access"),
    ("fav", "Toevoegen aan Favorieten", "Add to Favourites"),
    ("unfav", "Verwijderen uit Favorieten", "Remove from Favourites"),
    ("compress", "Comprimeren tot ZIP-bestand", "Compress to ZIP file"),
    ("extract", "Alles uitpakken\u{2026}", "Extract all\u{2026}"),
    ("copyPath", "Kopi\u{eb}ren als pad", "Copy as path"),
    ("terminal", "Openen in Terminal", "Open in Terminal"),
    ("openLocation", "Bestandslocatie openen", "Open file location"),
    ("restore", "Terugzetten", "Restore"),
    ("sortBy", "Sorteren op", "Sort by"),
    ("refreshMenu", "Vernieuwen", "Refresh"),
    ("pasteMenu", "Plakken", "Paste"),
    ("cutMenu", "Knippen", "Cut"),
    ("copyMenu", "Kopi\u{eb}ren", "Copy"),
    ("renameMenu", "Naam wijzigen", "Rename"),
    ("deleteMenu", "Verwijderen", "Delete"),
    ("newMenu", "Nieuw", "New"),
    ("columns", "Kolommen", "Columns"),
    // status bar
    ("items.one", "1 item", "1 item"),
    ("items.other", "{n} items", "{n} items"),
    ("selected.one", "1 item geselecteerd", "1 item selected"),
    ("selected.other", "{n} items geselecteerd", "{n} items selected"),
    ("statusDetails", "Details-weergave: informatie over elk item weergeven", "Details view: show information about each item"),
    ("statusLarge", "Voorbeelden van items weergeven met grote miniaturen", "Preview items with large thumbnails"),
    // groups
    ("gToday", "Vandaag", "Today"),
    ("gYesterday", "Gisteren", "Yesterday"),
    ("gThisWeek", "Eerder deze week", "Earlier this week"),
    ("gLastWeek", "Vorige week", "Last week"),
    ("gThisMonth", "Eerder deze maand", "Earlier this month"),
    ("gLastMonth", "Vorige maand", "Last month"),
    ("gThisYear", "Eerder dit jaar", "Earlier this year"),
    ("gOlder", "Lang geleden", "A long time ago"),
    ("gFuture", "Later", "Later"),
    ("gFolders", "Mappen", "Folders"),
    ("gEmpty", "Leeg", "Empty"),
    ("gTiny", "Zeer klein (0 - 16 kB)", "Tiny (0 - 16 KB)"),
    ("gSmall", "Klein (16 kB - 1 MB)", "Small (16 KB - 1 MB)"),
    ("gMedium", "Normaal (1 - 128 MB)", "Medium (1 - 128 MB)"),
    ("gLarge", "Groot (128 MB - 1 GB)", "Large (128 MB - 1 GB)"),
    ("gHuge", "Zeer groot (1 - 4 GB)", "Huge (1 - 4 GB)"),
    ("gGigantic", "Gigantisch (> 4 GB)", "Gigantic (> 4 GB)"),
    ("gUnknown", "Onbekend", "Unknown"),
    ("collapseGroup", "Groep samenvouwen", "Collapse group"),
    ("expandGroup", "Groep uitvouwen", "Expand group"),
    // home / this pc / gallery
    ("quickAccess", "Snelle toegang", "Quick access"),
    ("favorites", "Favorieten", "Favourites"),
    ("recent", "Recent", "Recent"),
    ("savedOnPC", "Opgeslagen op deze pc", "Stored locally"),
    ("savedOn", "Opgeslagen op {name}", "Stored on {name}"),
    ("pinned", "Vastgemaakt", "Pinned"),
    (
        "favEmpty",
        "Als je bestanden aan Favorieten toevoegt, zie je ze hier.",
        "After you\u{2019}ve added some files to Favourites, they\u{2019}ll show here.",
    ),
    ("recentEmpty", "Bestanden die je onlangs hebt geopend, zie je hier.", "Files you\u{2019}ve opened recently will show here."),
    (
        "recentOff",
        "Recente bestanden worden niet bijgehouden. Je kunt dit aanzetten bij Opties.",
        "Recent files aren\u{2019}t being tracked. You can turn this on in Options.",
    ),
    ("folders", "Mappen", "Folders"),
    ("devices", "Apparaten en stations", "Devices and drives"),
    ("freeOf", "{free} vrij van {total}", "{free} free of {total}"),
    ("almostFull", "Bijna vol", "Almost full"),
    ("noDrives", "Er zijn geen stations gevonden.", "No drives were found."),
    ("galleryEmpty", "Hier verschijnen je foto\u{2019}s", "Your photos will show here"),
    (
        "galleryEmptyMsg",
        "Zet foto\u{2019}s in de map Afbeeldingen en ze verschijnen automatisch in de Galerij.",
        "Put photos in your Pictures folder and they\u{2019}ll appear in the Gallery automatically.",
    ),
    ("galleryMore.one", "Alleen de nieuwste foto wordt weergegeven.", "Only the newest photo is shown."),
    ("galleryMore.other", "Alleen de nieuwste {n} foto\u{2019}s worden weergegeven.", "Only the newest {n} photos are shown."),
    ("openFolder", "Map openen", "Open folder"),
    // empty / loading / errors
    ("loading", "Bezig met laden\u{2026}", "Loading\u{2026}"),
    ("folderEmpty", "Deze map is leeg.", "This folder is empty."),
    ("trashEmpty", "De Prullenbak is leeg", "The Recycle Bin is empty"),
    (
        "trashEmptyMsg",
        "Verwijderde bestanden komen hier terecht. Je kunt ze terugzetten zolang je de Prullenbak niet leegmaakt.",
        "Deleted files end up here. You can restore them as long as you don\u{2019}t empty the Recycle Bin.",
    ),
    ("noMatch", "Er zijn geen items die overeenkomen met je zoekopdracht.", "No items match your search."),
    ("errOpenFolder", "Deze map kan niet worden geopend", "This folder can\u{2019}t be opened"),
    ("tryAgain", "Opnieuw proberen", "Try again"),
    ("goHome", "Naar Start", "Go to Home"),
    ("noTrash", "De Prullenbak is niet gevonden op deze computer.", "The Recycle Bin couldn\u{2019}t be found on this computer."),
    ("errTitle", "Dat lukte niet", "That didn\u{2019}t work"),
    ("errSome", "Dit ging mis bij: {names}", "There was a problem with: {names}"),
    ("dryRun", "Testmodus: er is niets echt gewijzigd.", "Test mode: nothing was actually changed."),
    // search
    ("searchResults", "Zoekresultaten in {name}", "Search results in {name}"),
    ("searchSub", "Ook in submappen zoeken", "Also search subfolders"),
    ("searching", "Bezig met zoeken\u{2026}", "Searching\u{2026}"),
    ("filtered", "Resultaten in {name}", "Results in {name}"),
    ("truncated", "Er zijn meer resultaten. Typ meer letters of zoek met Alles.", "There are more results. Type more letters or search with Alles."),
    ("clearSearch", "Zoekopdracht wissen", "Clear search"),
    // operations
    ("trashed.one", "\u{2018}{name}\u{2019} is naar de Prullenbak verplaatst", "\u{201c}{name}\u{201d} was moved to the Recycle Bin"),
    ("trashed.other", "{n} items zijn naar de Prullenbak verplaatst", "{n} items were moved to the Recycle Bin"),
    ("undo", "Ongedaan maken", "Undo"),
    ("undone", "Ongedaan gemaakt", "Undone"),
    ("restored.one", "\u{2018}{name}\u{2019} is teruggezet", "\u{201c}{name}\u{201d} was restored"),
    ("restored.other", "{n} items zijn teruggezet", "{n} items were restored"),
    ("pathCopied.one", "Pad gekopieerd", "Path copied"),
    ("pathCopied.other", "{n} paden gekopieerd", "{n} paths copied"),
    ("pasted.one", "1 item geplakt", "1 item pasted"),
    ("pasted.other", "{n} items geplakt", "{n} items pasted"),
    ("moved.one", "\u{2018}{name}\u{2019} is verplaatst naar {dest}", "\u{201c}{name}\u{201d} was moved to {dest}"),
    ("moved.other", "{n} items zijn verplaatst naar {dest}", "{n} items were moved to {dest}"),
    ("copied.one", "\u{2018}{name}\u{2019} is gekopieerd naar {dest}", "\u{201c}{name}\u{201d} was copied to {dest}"),
    ("copied.other", "{n} items zijn gekopieerd naar {dest}", "{n} items were copied to {dest}"),
    ("compressed", "\u{2018}{name}\u{2019} is gemaakt", "\u{201c}{name}\u{201d} was created"),
    ("extracted", "Uitgepakt naar \u{2018}{name}\u{2019}", "Extracted to \u{201c}{name}\u{201d}"),
    ("emptied", "De Prullenbak is leeggemaakt", "The Recycle Bin was emptied"),
    ("nameSlash", "Een naam mag geen / bevatten.", "A name can\u{2019}t contain /."),
    ("nameEmpty", "Typ een naam.", "Type a name."),
    ("nameBad", "Deze naam kun je niet gebruiken.", "You can\u{2019}t use this name."),
    ("nameHidden", "Een naam die met een punt begint, maakt het item verborgen.", "A name that starts with a dot makes the item hidden."),
    (
        "trashDelete",
        "Items in de Prullenbak kun je terugzetten. Wil je ruimte vrijmaken? Kies dan \u{2018}Prullenbak leegmaken\u{2019}.",
        "Items in the Recycle Bin can be restored. Want to free up space? Choose \u{201c}Empty Recycle Bin\u{201d}.",
    ),
    ("emptyTitle", "Prullenbak leegmaken?", "Empty Recycle Bin?"),
    (
        "emptyMsg.one",
        "Het item in de Prullenbak ({size}) wordt definitief verwijderd. Dit kun je niet ongedaan maken.",
        "The item in the Recycle Bin ({size}) will be permanently deleted. This can\u{2019}t be undone.",
    ),
    (
        "emptyMsg.other",
        "Alle {n} items in de Prullenbak ({size}) worden definitief verwijderd. Dit kun je niet ongedaan maken.",
        "All {n} items in the Recycle Bin ({size}) will be permanently deleted. This can\u{2019}t be undone.",
    ),
    ("emptyConfirm", "Leegmaken", "Empty"),
    ("cancel", "Annuleren", "Cancel"),
    ("ok", "OK", "OK"),
    ("apply", "Toepassen", "Apply"),
    ("close", "Sluiten", "Close"),
    ("moveTo", "Verplaatsen naar {name}", "Move to {name}"),
    ("copyTo", "Kopi\u{eb}ren naar {name}", "Copy to {name}"),
    ("trashTo", "Naar de Prullenbak verplaatsen", "Move to the Recycle Bin"),
    // properties
    ("propsTitle", "Eigenschappen van {name}", "{name} Properties"),
    ("general", "Algemeen", "General"),
    ("type", "Type", "Type"),
    ("location", "Locatie", "Location"),
    ("size", "Grootte", "Size"),
    ("sizeOnDisk", "Grootte op schijf", "Size on disk"),
    ("contains", "Bevat", "Contains"),
    ("containsVal", "{files} bestanden, {folders} mappen", "{files} files, {folders} folders"),
    ("created", "Gemaakt op", "Created"),
    ("modified", "Gewijzigd op", "Modified"),
    ("accessed", "Geopend op", "Accessed"),
    ("attributes", "Kenmerken", "Attributes"),
    ("readOnly", "Alleen-lezen", "Read-only"),
    ("hiddenAttr", "Verborgen", "Hidden"),
    ("owner", "Eigenaar", "Owner"),
    ("permissions", "Machtigingen", "Permissions"),
    ("bytesFull", "{h} ({b} bytes)", "{h} ({b} bytes)"),
    ("calculating", "Bezig met berekenen\u{2026}", "Calculating\u{2026}"),
    ("opensWith", "Wordt geopend met", "Opens with"),
    ("multiName.one", "{n} item", "{n} item"),
    ("multiName.other", "{n} items", "{n} items"),
    ("multiTypes", "Meerdere typen", "Multiple types"),
    ("allOfType", "Alles van het type {type}", "All of type {type}"),
    ("allIn", "Alles in {dir}", "All in {dir}"),
    ("variousFolders", "Diverse mappen", "Various folders"),
    ("mime", "MIME-type", "MIME type"),
    ("fullPath", "Volledig pad", "Full path"),
    ("link", "Snelkoppeling naar", "Shortcut to"),
    ("readOnlyHint", "Wie dit mag wijzigen, stel je in via de machtigingen.", "The permissions decide who can change this."),
    ("change", "Wijzigen\u{2026}", "Change\u{2026}"),
    ("driveTitle", "Eigenschappen van {name}", "{name} Properties"),
    ("fsType", "Bestandssysteem", "File system"),
    ("used", "Gebruikte ruimte", "Used space"),
    ("free", "Vrije ruimte", "Free space"),
    ("capacity", "Capaciteit", "Capacity"),
    ("mount", "Koppelpunt", "Mount point"),
    ("freeUp", "Ruimte vrijmaken", "Free up space"),
    ("removable", "Verwisselbaar station", "Removable drive"),
    ("localDisk", "Lokaal station", "Local disk"),
    ("trashTitle", "Eigenschappen van Prullenbak", "Recycle Bin Properties"),
    ("trashSize", "In de Prullenbak", "In the Recycle Bin"),
    // share / options
    ("shareTitle", "Delen", "Share"),
    ("copyPathBtn", "Pad kopi\u{eb}ren", "Copy path"),
    (
        "shareMsg",
        "Rechtstreeks delen kan nog niet vanuit Verkenner. Kopieer het pad of sleep het bestand naar een app, bijvoorbeeld je e-mail.",
        "Sharing directly isn\u{2019}t available in File Explorer yet. Copy the path, or drag the file into an app such as your email.",
    ),
    ("optionsTitle", "Mapopties", "Folder options"),
    ("optRecent", "Onlangs gebruikte bestanden weergeven op Start", "Show recently used files in Home"),
    ("optHint", "Deze opties gelden voor alle mappen.", "These options apply to all folders."),
    // details pane
    ("noSelection", "Selecteer een bestand om de details te zien.", "Select a file to see its details."),
    ("itemsInFolder.one", "1 item", "1 item"),
    ("itemsInFolder.other", "{n} items", "{n} items"),
    ("totalSize", "Totale grootte", "Total size"),
    ("dimensions", "Afmetingen", "Dimensions"),
    // misc
    ("splitter", "Breedte van het navigatievenster aanpassen", "Resize the navigation pane"),
    ("expand", "Uitvouwen", "Expand"),
    ("collapse", "Samenvouwen", "Collapse"),
    ("file", "Bestand", "File"),
    ("folder", "Bestandsmap", "File folder"),
    ("shortcut", "Snelkoppeling", "Shortcut"),
    ("extFile", "{ext}-bestand", "{ext} File"),
];

thread_local! {
    static INDEX: HashMap<&'static str, (&'static str, &'static str)> =
        TEXTS.iter().map(|(k, nl, en)| (*k, (*nl, *en))).collect();
}

/// The text of `key` in the UI language (the key itself when there is none, so a typo shows).
pub fn s(key: &str) -> &'static str {
    INDEX.with(|i| match i.get(key) {
        Some((nl, en)) => kit::t(nl, en),
        None => {
            eprintln!("win11-gtk-explorer: no text for {key}");
            Box::leak(key.to_owned().into_boxed_str())
        }
    })
}

/// `s(key)` with `{name}` style placeholders filled.
pub fn f(key: &str, args: &[(&str, &str)]) -> String {
    fill(s(key), args)
}

/// The plural form of `key` (`key.one` for 1, else `key.other`) with `{n}` and the other placeholders filled.
pub fn n(key: &str, count: usize, args: &[(&str, &str)]) -> String {
    let form = if count == 1 { format!("{key}.one") } else { format!("{key}.other") };
    let count_text = count.to_string();
    let mut all: Vec<(&str, &str)> = vec![("n", count_text.as_str())];
    all.extend_from_slice(args);
    fill(s(&form), &all)
}

/// "1 file, 2 folders" in English (i18n.js containsVal is a function there); the Dutch text as it is.
#[allow(dead_code)] // the API of builders B and C (see main.rs)
pub fn contains_val(files: &str, folders: &str) -> String {
    if kit::i18n::en() {
        let fw = if files == "1" { "file" } else { "files" };
        let dw = if folders == "1" { "folder" } else { "folders" };
        format!("{files} {fw}, {folders} {dw}")
    } else {
        f("containsVal", &[("files", files), ("folders", folders)])
    }
}

fn fill(text: &str, args: &[(&str, &str)]) -> String {
    if !text.contains('{') {
        return text.to_owned();
    }
    let mut out = text.to_owned();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}
