# Rust-instellingen — versie 0.4.0

Gecontroleerd op 1 november 2026 in een eigen schermloze GNOME 50.1/Wayland-sessie met drie echte Rust/WebKit-vensters van de oorspronkelijke Instellingen-app. Alle twaalf gegroepeerde integratiegevallen slagen: elf voor instellingen en één aanvullende proef van de productieprullenbakroute. De bestaande HTML/CSS-interface is behouden; de schermafbeelding staat in `preview-settings.png`.

## Native bereik en vergelijking

`settings.get`, `settings.set`, `settings.patch`, `settings.reset`, `settings.export`, `settings.import` en `settings.backupFile` worden door Rust afgehandeld. Hun antwoorden dragen `X-Win11-Service: rust-settings`. Het derde venster gebruikte een testkopie van de Python-dienst waarin alle zeven openbare instellingenmethoden bewust fouten geven. Alle zeven bleven via de Rust-host werken.

De referentie was een bevroren kopie van de oorspronkelijke Python-`SettingsStore` en methoderegistratie, met een afzonderlijk instellingenbestand. De vergelijking controleerde volledige resultaten en foutobjecten, niet alleen enkele velden:

- Standaardwaarden samenvoegen, onbekende sleutels behouden, Unicode, arraypaden en het onderscheid tussen puntpaden en letterlijke punten in een arraysegment.
- Ongeldige paden, alleen-lezen `version`, booleans tegenover getallen, ongeldige subbomen, vereiste lijsten van strings en de oorspronkelijke betekenis van lege/false waarden bij `patch`.
- Een gedeeltelijk vervangen `theme`-object blijft daadwerkelijk gedeeltelijk, ook in een ander Rust-proces en diens Python-geheugen. Export behoudt de oorspronkelijke JSON-opmaak en sleutelvolgorde.
- Import als object en tekst, foutieve import, typefiltering, behoud van onbekende instellingen, geforceerde schemaversie en reset van bekende/onbekende paden en alle instellingen.
- Back-ups bevatten dezelfde bytes als de referentie, gebruiken modus `0600`, behouden de antwoordvelden en krijgen afzonderlijke namen bij opeenvolgende verzoeken.

## Gebeurtenissen, bestaande diensten en bediening

Beide Rust-eventstreams en beide bijbehorende Python-eventstreams ontvingen de nieuwe instellingen en de juiste `changed`-paden. Het schrijvende venster behoudt het initiating-pad; een herlaadgebeurtenis kan `path: null` gebruiken. De private Python-backends namen de exacte native toestand over. Hun bestaande `files.list` gebruikte daarna aantoonbaar de nieuwe `explorer.showHidden`-waarde uit het geheugen.

De oorspronkelijke schakelaar voor transparantie werd in een echt WebKit-venster aangeklikt via zijn bestaande DOM-handler. Dit schreef de native instelling en werkte de HTML-themastatus in beide vensters bij. De afbeelding is een echte GNOME-opname van die vensters, geen nagemaakte interface.

Twaalf gelijktijdige workers schreven 24 verschillende sleutels via twee onafhankelijke Rust-hosts. Een tweede proef schreef nog 24 sleutels met Rust-verzoeken en rechtstreekse verzoeken aan beide eigen Python-backends door elkaar. Alle sleutels bleven aanwezig op schijf en bereikten beide native snapshots en beide Python-geheugentoestanden. De gedeelde bestandsvergrendeling voorkomt hierbij verloren wijzigingen.

De openbare opslag, validatie, import, export en back-up draaien native. De resterende systeemthema- en GTK-listeners blijven de oorspronkelijke compatibiliteitscode gebruiken en worden met de nieuwe toestand bijgewerkt. Deze meting beweert geen volledige Rust-migratie van alle systeeminstellingenpagina's, hardwarediensten of thema-effecten.

## Aanvullende GIO-prullenbakproef

Zonder `WIN11SHELL_TRASH_DIR` werd precies één eigen regulier bestand naar de echte GIO-prullenbakroute gestuurd. Vooraf is gecontroleerd dat de bron en de private `XDG_DATA_HOME` hetzelfde apparaatnummer hebben. Het native antwoord wees naar de eigen `data/Trash/files`, de bijbehorende `.trashinfo` en herstelnaam werden teruggevonden, en native herstellen bracht de oorspronkelijke inhoud terug. De eigen prullenbakdata en metadata waren daarna verdwenen.

Deze proef gebruikte geen productieprullenbaklijst of permanente verwijdering en geen acties op gemounte prullenbakken. De afzonderlijke suite in `VALIDATION_FILES_COMPLETE.md` controleert legen uitsluitend met een expliciete testprullenbak.

## Isolatie en grenzen

Alle appprocessen gebruikten een eigen `HOME`, XDG-configuratie, data, cache, runtime en de GSettings-keyfilebackend. De actieve desktopinstellingen en gebruikersbestanden zijn niet aangepast. De Python-referentie had bovendien uitgeschakelde systeemmutaties. De tests gebruiken geauthenticeerde HTTP, SSE en bestaande DOM-handlers; fysieke muis- en toetsenbordhardware, iedere instellingenpagina, alle bestandssystemen en langdurige foutscenario's zijn niet volledig getest. De opname bewijst geen pixelvergelijking van alle schermen.

De ontwikkelharness staat in `work/settings-native-test/run.py`, met de bevroren referentie onder `oracle/` en het volledige resultaat in `results.json`. Het compacte distributiebewijs is `validation/settings-results.json`, zonder tokens, persoonlijke instellingen, proces-ID's of bestandsinhoud.
