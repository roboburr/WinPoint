# Verkenner: lokale desktopfuncties — versie 0.5.0

> **Historisch (D-G72, 14 november 2026):** gemeten met de HTML-interface in WebKitGTK van toen. Die is verwijderd: Verkenner, Instellingen, Taakbeheer en de andere apps zijn nu native GTK 4-programma's, en Win11 voor GNOME bevat geen WebKit en geen GTK 3-host meer. De Rust-diensten die dit rapport controleert, bestaan nog. De laatste commit met WebKit is `6ebf169f`; zie [native/APPS.md](native/APPS.md).

Op 1 november 2026 slagen twaalf integratiegroepen met echte Rust/WebKit-Verkenners in een private GNOME 50.1/Wayland-sessie. De definitieve build is opnieuw gecontroleerd. De acht laatste oorspronkelijke Files-methoden zijn geporteerd: `files.places`, `files.find`, `files.watch`, `files.unwatch`, `files.open`, `files.openWith`, `files.openTerminal` en `files.trustLauncher`. Daarmee zijn alle 28 openbare methoden uit de oorspronkelijke Files-service native afgehandeld. De volledige shell en haar andere diensten zijn hiermee nog niet volledig naar Rust omgezet.

## Gemeten uitkomsten

- Standaardlocaties komen volledig overeen met de referentie, inclusief volgorde, pad, naam, pictogram en bestaan-vlag.
- Zoeken komt overeen voor Unicode, meerwoordtermen, globklassen, verborgen namen, `.hidden`, MIME-prefix, limiet, ongeldige invoer en symlinks zonder recursie naar hun doelen. De aanvullende 166 zoekgevallen staan in `VALIDATION_SEARCH.md`.
- Extern schrijven veroorzaakt native SSE-meldingen. Twee watch-aanroepen delen een monitor; één unwatch houdt die actief en de laatste verwijdert de monitor. De desktop blijft automatisch en vast gevolgd. De echte UI-verversing, tabwisselingen en lokale berichtvolgorde staan in `VALIDATION_WATCH.md`.
- GIO opent gewone bestanden met een eigen testrecorder als standaardapp en als expliciet gekozen app. De bestandsnaam met spaties en shelltekens blijft één letterlijk argument. Private host-omgevingssleutels ontbreken.
- Een onbetrouwbare launcher start niet en geeft exact het oorspronkelijke foutobject. Na expliciet toestaan worden GIO-vertrouwen en uitvoerrecht gebruikt, en start de bedoelde recorder. Ontbrekende bestanden en apps behouden de oorspronkelijke foutobjecten.
- Terminalkeuze volgt de private `xdg-terminals.list`. Een eigen recorder met de naam `kgx` ontvangt exact `--working-directory=<map>`; het proces draait ook werkelijk in die map.
- Een map opent in een nieuw echt Rust-Verkenner-proces met de juiste inhoud; het venster wordt daarna via de native close-handler gesloten.
- Een tweede testkopie blokkeert alle acht corresponderende Python-methoden. Alle native routes blijven werken en antwoorden met `X-Win11-Service: rust-files`. De verspreide Python-bron wordt hiervoor niet aangepast.

## Isolatie en grenzen

HOME, XDG-configuratie, data, runtime, cache, GSettings en de expliciet ingestelde Trash zijn testmappen. App- en terminalassociaties gelden uitsluitend daar; echte gebruikersapps worden niet gestart. De actieve desktopconfiguratie en gebruikersbestanden zijn niet aangepast. De interfacebestanden zijn in 0.5.0 niet gewijzigd.

De eerste twee ontwikkelpogingen corrigeerden alleen harness-aannames over de naam van de eventstreamhelper en GIO’s lokale bestandsargumenten. De definitieve herhaling slaagt. Dit is geen volledige controle van alle dialoogbediening met fysieke invoer, ieder bestandssysteem, remote handler, terminalmerk of iedere race. Het optionele oude cgroup-prioriteitsbeleid is nog niet aangesloten. Een geslaagde GIO-aanroep bewijst niet de werking van willekeurige externe programma’s.

De reproduceerbare ontwikkelharness en het volledige resultaat staan in `work/files-desktop-test/`; compact distributiebewijs: `validation/files-desktop-results.json`. De oudere 0.4.0-suites voor prullenbak, archieven, eigenschappen en instellingen zijn deze ronde niet integraal herhaald; hun Rust-unitgevallen slagen wel opnieuw. De 34 actuele bestandsregressies en taakbalkmeting staan in `VALIDATION_REGRESSIONS_0.5.md`.
