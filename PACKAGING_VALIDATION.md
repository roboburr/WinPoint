# Controle van installatie en verpakking

> **Historisch (D-G72, 14 november 2026):** deze controles gingen over pakketten met de HTML-apps in WebKitGTK en minder programma's. Een pakket bevat nu 19 programma's (de host zonder WebKit of GTK 3, en de GTK 4-shell, -agenda en -apps); `scripts/package.py` en `tests/installer/run.sh` controleren dat. De laatste commit met WebKit heeft de tag `last-webkit`.

Uitgevoerd op 1 november 2026 in tijdelijke mappen; geen installatie in de actieve desktop of activering uitgevoerd. De scriptcontrole gebruikt vier onschadelijke testprogramma's. De aanvullende archiefcontrole gebruikt de echte meegeleverde Rust-programma's.

## Actuele archiefcontrole: 0.7.0

Het definitieve 0.7.0-archief wordt vergeleken met alle huidige bronbestanden, rapporten, UI-hashes en zes geteste ELF-programma’s. Installeren en verwijderen gebeuren in een private tijdelijke prefix, met een onafhankelijke configuratiesentinel. De controle start geen GUI-apps en wijzigt de actieve desktop niet.

De huidige UI blijft 162 bestanden, met zes aangepaste JavaScript-modules en één toevoeging; de zesde wijziging is het op verzoek aangepaste ronde Startlogo; alle oorspronkelijke CSS-hashes blijven gelijk. De correcties voor Start-hoeken, sneltoetsen en appactivering zitten in de host en GNOME-koppeling. De traycorrectie zit in de bestaande compatibiliteitsdienst. Het pakket bevat de desktop-, achtergrond-, activerings-, indexer- en geheugengroeprapporten van 0.7.0 naast de herkenbaar eerdere sneltoets-, tray- en hoekencontroles. `validation/regressions-0.7.0.json` bundelt het actuele bewijs.

De acht desktop-items worden gecontroleerd met `desktop-file-validate`. Het lokale extensieschema wordt strikt gecompileerd; de geïnstalleerde Ctrl+Alt+Delete-standaard wordt gelezen. Alle eigen extensie- en schemafiles moeten na verwijderen verdwenen zijn; de configuratiesentinel blijft bestaan. De externe `.sha256` hoort bij het definitieve archief. Ontwikkelbewijs: `work/packaging-audit-0.7.0/archive-results.json`.

## Eerdere archiefcontrole: 0.6.1

Het definitieve `win11-gnome-0.6.1-linux-x86_64.tar.gz` wordt uitgepakt, met de huidige bron-/rapportbestanden en alle vier geteste ELF-binaries vergeleken, en uitsluitend in een private prefix geïnstalleerd en verwijderd. De externe `.sha256` hoort bij het definitieve archief. Deze verpakkingscontrole start geen GUI en herhaalt de historische API-suites niet.

De huidige bewijzen zijn 140 Rust-tests, 26 GNOME-controles, 13 Start-groepen en elf Alles-DOM-groepen. De UI telt 162 bestanden tegenover de baseline van 161: vijf bestaande JavaScript-modules zijn aangepast en één module is toegevoegd. Alle CSS-hashes blijven gelijk aan de oorspronkelijke baseline. Het archief bevat de drie nieuwe specialistische rapporten en `validation/regressions-0.6.1.json`, zonder persoonlijke configuratie, tokens, bouwcache of testgegevens. De drie geïnstalleerde desktop-items worden met `desktop-file-validate` gecontroleerd; verwijderen bewaart een onafhankelijke configuratiesentinel.

Ontwikkelbewijs van de definitieve archiefcontrole: `work/packaging-audit-0.6.1/archive-results.json`. De beperkte bekende runtimeproblemen blijven in de functionele rapporten vermeld.

## Installatiescripts

Geslaagd:

- Shellsyntaxis van de bouw-, installatie-, verpakkings- en verwijderingsscripts.
- Python-syntaxis van de installatiescripts.
- Installatie van vier programma's: `win11-start`, `win11-explorer`, `win11-panel` en `win11-app`.
- Installatie van de huidige UI, de oorspronkelijke en huidige hashmanifesten, de wijzigingsbeschrijving en de Python-compatibiliteitsbackend naar een tijdelijke prefix.
- Installatie en verwijdering van `extension/panelBridge.js` samen met de andere eigen extensiebestanden.
- Validatie van de drie `.desktop`-bestanden voor Start, Verkenner en het verborgen paneel-item met `desktop-file-validate`.
- Werkelijk starten van de onschadelijke Start-, Verkenner- en paneelprogramma's via GIO uit een pad met spaties, aanhalingstekens, backslash, dollar, backtick en procentteken.
- Rechtstreeks starten van het geïnstalleerde onschadelijke `win11-app`-programma met behoud van `--app settings` als argumenten. Dit controleert de installatie en argumentoverdracht, niet de Rust-app-parser of Instellingen zelf.
- Correct doorgeven van een lokaal bestandspad met spaties en procentteken vanuit een file-URI naar Verkenner.
- Bijwerken van een gemarkeerde installatie.
- Verwijderen van de vier eigen programma's met behoud van instellingen buiten de pakketmap.
- Weigeren om een bestaande niet-gemarkeerde pakketmap te overschrijven of te verwijderen.
- Integriteitscontrole van de huidige 162 UI-bestanden tegenover het distributiemanifest; de baseline van 161 bestanden blijft afzonderlijk beschikbaar. Het verschil bestaat uit drie gewijzigde modules en één toegevoegde module.

## Eerdere archiefcontrole: 0.6.0

Gecontroleerd: `win11-gnome-0.6.0-linux-x86_64.tar.gz`, met vier geteste Cargo-devbinaries zonder debug-symbolen. De controle pakt het echte archief uit en installeert/verwijdert uitsluitend in een eigen tijdelijke prefix. De externe `.sha256` hoort bij het definitieve archief.

Geslaagd:

- Alle vier uitvoerbare ELF-programma's hebben de hashes van de definitieve build; Cargo-bronversie en archiefnaam komen overeen.
- Native catalogus, gettext/data, standaardapps, recente bestanden/datumparser, URI-/tabparameters, lifecycle-flag en bestaande Files/Settings-bronnen zijn aanwezig en gelijk aan hun definitieve bronbestanden.
- De drie publieke native routes (`rust-files`, `rust-settings`, `rust-apps`) staan vóór de compatibiliteitsproxy; het huidige eventstreamfilter is meegeleverd.
- De huidige 162 UI-bestanden voldoen aan hun distributiemanifest, met behoud van de oorspronkelijke baseline. Vier JavaScript-modules verschillen bewust van die baseline en één module is toegevoegd; alle CSS-hashes zijn nog gelijk aan het origineel. De Start-Meer-correctie is in de meegeleverde bron aanwezig.
- Rapporten voor 140 geslaagde Rust-tests en 46 Apps-integratiegroepen, de actuele Files/Watch/paneelregressies en vier preview-PNG's zijn aanwezig. Historische rapporten blijven herkenbaar apart. De archiefcontrole leest dit bewijs en herhaalt de GUI-proeven niet.
- Het uitgepakte `bin/` installeert rechtstreeks; programmacontrolesommen blijven gelijk. UI, compatibiliteitsbackend en GNOME-extensie worden meegeïnstalleerd. Drie desktop-items slagen voor `desktop-file-validate`; de echte `win11-app --help` werkt.
- Verwijderen wist alleen de gemarkeerde tijdelijke installatie en behoudt een onafhankelijke instellingen-sentinel.
- Bouwcache, Python-bytecode, Git-metadata en persoonlijke configuratiemappen ontbreken in het pakket.

De actieve desktop is niet gewijzigd. De ongeklaarde eerdere testopstartuitval blijft als alpha-beperking in `VALIDATION_APPS.md` en `validation/regressions-0.6.json` vermeld. De archiefcontrole bewijst geen aanvullende GUI-stabiliteit. Ontwikkelbewijs: `work/packaging-audit/archive-results.json`.

## Eerdere archiefcontrole: 0.5.0

Gecontroleerd: `win11-gnome-0.5.0-linux-x86_64.tar.gz`, met vier geteste Cargo-devbinaries zonder debug-symbolen. De controle pakt het echte archief uit, installeert en verwijdert uitsluitend in een eigen tijdelijke prefix. De checksum van het definitieve archief staat in de externe `.sha256`.

Geslaagd:

- Alle vier uitvoerbare ELF-binaries hebben exact de hashes van de definitieve build. Cargo-bronversie en pakketnaam komen overeen.
- De native modules voor standaardlocaties, zoeken, openen en mapbewaking zitten samen met de eerdere bestanden en instellingen in het pakket; alle gecontroleerde bronbestanden komen overeen met de definitieve werkbestanden.
- De oorspronkelijke en huidige UI-manifesten zijn meegeleverd; alle 162 huidige UI-bestanden voldoen aan hun distributiemanifest. De interface is deze ronde niet gewijzigd.
- De native Files- en Settings-routes en de lokale/bestaande samengevoegde eventstream zijn aanwezig. De JSON-rapporten tonen alle 28 native Files-methoden, 104 geslaagde Rust-tests, twaalf nieuwe desktopgroepen, 166 zoekgevallen en acht watch/UI-groepen. Historische 0.3/0.4-rapporten blijven afzonderlijk herkenbaar.
- De actuele rapporten, bestandsregressiegegevens en drie previews zijn aanwezig en gelijk aan de definitieve bronbestanden. De archiefcontrole leest dit bewijs; zij herhaalt de GUI-suites niet.
- Installeren vanuit het uitgepakte `bin/` bewaart de programmacontrolesommen en installeert de UI, compatibiliteitsbackend en eigen GNOME-extensie. Alle drie desktop-items slagen voor `desktop-file-validate`; `win11-app --help` werkt.
- Verwijderen wist uitsluitend de gemarkeerde tijdelijke installatie; een onafhankelijke instellingen-sentinel blijft behouden.
- Bouwcache, Python-bytecode, Git-metadata en persoonlijke configuratiemappen ontbreken in het archief.

De actieve desktop is niet gewijzigd. Ontwikkelbewijs: `work/packaging-audit/archive-results.json`; de externe checksum geldt steeds voor de definitieve archieffile. De uitvoerige functionele grenzen staan in de afzonderlijke 0.5.0-validatierapporten.

## Eerdere archiefcontrole: 0.4.0

Gecontroleerd: `win11-gnome-0.4.0-linux-x86_64.tar.gz`, met vier geteste Cargo-devbinaries zonder debug-symbolen. De controle pakt het echte archief uit en installeert/verwijdert uitsluitend in een eigen tijdelijke prefix. De externe `.sha256` hoort bij het definitieve archief.

Geslaagd:

- Archiefchecksum en de hashes van alle vier uitvoerbare ELF-programma's komen overeen met de definitieve build.
- De oorspronkelijke en huidige UI-manifesten worden meegeleverd; alle 162 huidige UI-bestanden voldoen aan hun manifest.
- De nieuwe Rust-prullenbak-, archief-, eigenschappen- en instellingenmodules, launchervertalingen, standaardinstellingen en compatibiliteitsadoptie komen overeen met de definitieve bronbestanden.
- Beide native API-routes (`rust-files`, `rust-settings`) staan vóór de compatibiliteitsproxy; de samengevoegde eventstream is aanwezig.
- De meegeleverde resultaten bevatten 78 geslaagde Rust-tests, 22 nieuwe bestandsintegratiegevallen, twaalf instellingenintegratiegroepen en de actuele regressiegegevens. De controle leest dit bewijs; zij herhaalt de GUI-suites niet.
- Alle drie preview-PNG's, actuele en historische rapporten zijn aanwezig en komen overeen met hun definitieve bestanden.
- Installeren vanuit het uitgepakte `bin/` behoudt alle programmacontrolesommen en levert UI, compatibiliteitsbackend en GNOME-extensie mee. De drie desktop-items slagen voor `desktop-file-validate`; de echte `win11-app --help` werkt.
- Verwijderen wist alleen de gemarkeerde tijdelijke installatie; een onafhankelijk instellingen-sentinel blijft behouden.
- Bouwcache, Python-bytecode, Git-metadata en persoonlijke configuratiemappen ontbreken in het pakket.

De actieve desktop is niet gewijzigd. Ontwikkelbewijs: `work/packaging-audit/archive-results.json`. De afzonderlijke GUI-resultaten en hun grenzen staan in `VALIDATION_FILES_COMPLETE.md`, `VALIDATION_SETTINGS.md` en `VALIDATION.md`.

## Eerdere archiefcontrole: 0.3.0

Gecontroleerd: `win11-gnome-0.3.0-linux-x86_64.tar.gz`, met de geteste Cargo-devbuild zonder debug-symbolen. De externe `.sha256`-file bevat de checksum van het bijbehorende archief. Deze controle installeert alleen in een eigen tijdelijke prefix.

Geslaagd:

- De archiefchecksum klopt; alle vier uitvoerbare programma's zijn ELF-binaries en hebben exact dezelfde hashes als de definitief te verpakken programma's.
- Alle 162 UI-bestanden voldoen aan het huidige distributiemanifest. De oorspronkelijke baseline en wijzigingsbeschrijving zitten ook in het pakket.
- `VALIDATION_FILES.md`, `validation/files-results.json`, de overige validatierapporten en beide preview-PNG's zijn aanwezig. De meegeleverde rapporten en previews komen overeen met de definitieve ontwikkelbestanden.
- De vier nieuwe Rust-bestandsmodules, de eventmodule en de typenamentabel zijn aanwezig en hebben dezelfde hashes als hun bronbestanden.
- De meegeleverde serverbron bevat de native `X-Win11-Service: rust-files`-header en behandelt native bestandsaanroepen vóór de compatibiliteitsproxy. De samengevoegde eventstream en native eventbron zijn aanwezig.
- Het bestandsrapport bevat 34 geslaagde functionele gevallen voor twaalf native methoden en vier geslaagde gerichte klembordordeningsherhalingen. Het bevat ook de apart uitgevoerde proef waarin de equivalente Python-methoden geblokkeerd waren. De archiefcontrole controleert dit meegeleverde bewijs en voert deze GUI-proef niet opnieuw uit.
- Het uitgepakte pakket installeert rechtstreeks vanuit zijn eigen `bin/`. De vier geïnstalleerde programma's behouden hun hashes; de compatibiliteitsbron, UI-manifesten en GNOME-koppeling worden meegeïnstalleerd.
- De drie desktop-items slagen voor `desktop-file-validate`; `win11-app --help` werkt vanuit het uitgepakte pakket.
- Verwijderen vanuit het uitgepakte pakket wist de eigen programma's en pakketmap en behoudt een onafhankelijk instellingenbestand.
- Bouwcache, Python-bytecode, Git-metadata en persoonlijke configuratiemappen ontbreken in het archief.

Deze archiefcontrole opent geen GUI-apps. De werking van de bestandskern en de echte vensters is afzonderlijk gemeten zoals beschreven in `VALIDATION_FILES.md`, `VALIDATION.md` en `VALIDATION_TABS.md`.

## Eerdere archiefcontrole: 0.2.0

De onderstaande resultaten horen bij het eerder gemaakte 0.2.0-archief. Ze zijn historische onderbouwing en vervangen de bovenstaande 0.3.0-controle niet.

Gecontroleerd: `win11-gnome-0.2.0-linux-x86_64.tar.gz`, met de geteste Cargo-devbuild zonder debug-symbolen. De externe `.sha256`-file bevat de checksum van het bijbehorende archief.

Geslaagd:

- De archiefchecksum komt overeen met de externe checksumfile.
- De vier uitvoerbare ELF-programma's hebben dezelfde hashes als de geteste, te verpakken programma's.
- Alle 162 uitgepakte UI-bestanden voldoen aan het distributiemanifest.
- Het archief bevat de oorspronkelijke en huidige UI-manifesten, `panelBridge.js`, de drie validatierapporten en de compacte `validation/results.json`.
- Beide preview-PNG's zijn aanwezig en hebben dezelfde hashes als de bronafbeeldingen.
- Bouwcache, Python-bytecode, Git-metadata en persoonlijke configuratiemappen ontbreken.
- Het uitgepakte pakket installeert rechtstreeks vanuit zijn eigen `bin/` in een tijdelijke prefix; de vier geïnstalleerde programma's behouden hun hashes.
- De drie geïnstalleerde desktop-items voldoen aan `desktop-file-validate`.
- `win11-app --help` werkt vanuit het uitgepakte pakket.
- Verwijderen vanuit het uitgepakte pakket wist de eigen programma's en payload en behoudt een onafhankelijk instellingenbestand.

Deze archiefcontrole opent geen GUI-apps. De werking van de echte vensters is afzonderlijk gemeten zoals beschreven in `VALIDATION.md` en `VALIDATION_TABS.md`.

Deze controles zeggen niets over de werking van vensterknoppen, bestandsacties, taboverdracht of de GNOME-taakbalk. Daarvoor gelden de afzonderlijke functionele validatierapporten.

Testscripts: `work/packaging-audit/test_install.py` en `work/packaging-audit/check_archive.py` in de ontwikkelwerkmap. Het laatste resultaat staat daar in `archive-results.json`; het distributiearchief bevat de compacte samenvatting zonder zelfverwijzende archiefchecksum.
