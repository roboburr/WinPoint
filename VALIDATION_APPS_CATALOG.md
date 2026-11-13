# Rust-appcatalogus — versie 0.6.0

Gecontroleerd op 1 november 2026 in een eigen schermloze GNOME 50.1/Wayland-sessie met echte Rust/WebKit-processen. Alle **tien gegroepeerde integratiegevallen** slagen. De oorspronkelijke Start-interface toont de native catalogus in **Alle apps**; `preview-apps-catalog.png` is een echte schermopname met herkenbare testapps.

## Volledige metadata en taal

De publieke `apps.list`-antwoorden dragen `X-Win11-Service: rust-apps`. Het volledige resultaat, inclusief volgorde en alle velden, is in Engels en Nederlands vergeleken met de oorspronkelijke `AppCatalog` in de private Python-backend van hetzelfde venster. Na het toevoegen van een app is deze vergelijking voor twee onafhankelijke Rust-hosts herhaald. De private referentie heeft uitsluitend een testmethode die zijn bestaande cache ongeldig maakt; de metadataopbouw blijft de oorspronkelijke code.

De testcatalogus bevat 72 zichtbare apps bij de eerste meting. Getest zijn:

- Alle vijf ingebouwde apps, hun vaste categorieën, zoekwoorden en vertalingen; een geïnstalleerde eigen Verkenner-entry met een echt PNG-pictogram heeft de oorspronkelijke pictogramvoorkeur.
- `NoDisplay` en `Hidden`-entries komen niet in de zichtbare lijst. De afzonderlijke Rust-unittests controleren ook de eenvoudige metadata voor verborgen entries.
- Inline `Name[nl]`, `GenericName[nl]`, `Comment[nl]`, dubbele vertaalde zoekwoorden en de oorspronkelijke volgorde van extra Engelse zoekwoorden.
- De Nederlandse ingebouwde terugvaltabellen en de vaste naam `Schijven (GNOME)` voor GNOME Disks.
- Een gettext-entry zonder inline vertaling, met een bestaande Nederlandse `WebKitGTK-4.1.mo` als alleen-lezen bron. Naam en overige metadata zijn gelijk aan de oorspronkelijke Python-gettext-route.
- Stabiele accentongevoelige NFKD/casefold-sortering en numerieke naamdelen die groter zijn dan een 64-bits integer.

Gettext-vertalingen worden in Rust uit de MO-tabellen gelezen. Zeven tijdelijke Rust-unittests controleren daarnaast little-/big-endian MO, UTF-8 en ISO-8859-1, beschadigde offsets, taalterugval, domeinvoorrang, keywordduplicaten, verborgen metadata, natuurlijke sortering met cijfers uit andere schriften en de seen-registry.

## Installatie, wijzigingen en eerste waarneming

Alle aanwezige apps hebben bij een nieuwe registry `added: 0` en `installedAt: 0`. Een later toegevoegd eigen desktopbestand krijgt een positieve, gelijke waarde in beide velden. Twee onafhankelijke hosts lezen exact dezelfde eerste-waarnemingstijd.

Beide native eventstreams ontvangen `apps.changed` bij installatie en verwijdering; de lijsten nemen de nieuwe app op en verwijderen haar weer. Een taalwissel naar Nederlands en terug naar Engels maakt de catalogus ongeldig en levert eveneens de native gebeurtenis.

Een aanvullende proef schrijft twaalf nieuwe desktopbestanden met zes workers en vernieuwt beide hostcatalogi gelijktijdig. Alle twaalf registry-items blijven behouden, met dezelfde timestamps in beide catalogi en op schijf. Native schrijvers delen een bestandsvergrendeling en publiceren de registry atomair. De private compatibiliteitsbackends lezen de registry en laten het schrijven aan Rust over.

## Bestaande interface en native route

Een derde host gebruikt een testkopie waarin alle acht publieke Python-`apps.*`-methoden bewust fouten geven. `apps.list` blijft native werken en levert dezelfde volledige catalogus als het eerste venster.

Daarna opent een afzonderlijk Rust-Startproces de oorspronkelijke HTML/CSS-interface. De bestaande knop **All apps** wordt via zijn DOM-handler bediend. De lijst toont de correcte testnaam, alle vijf eigen apps en geen verborgen testentry; zij bevat na de twaalf toevoegingen 84 apps. Er worden geen apps uit de lijst gestart.

## Isolatie en grenzen

Alle desktopbestanden en mutaties staan onder de eigen werkmap. De processen gebruiken een eigen `HOME`, XDG-configuratie, data, cache, runtime, GSettings-keyfilebackend en testprullenbak. De actieve desktop en de echte applicatiebestanden zijn niet aangepast. De test registreert uitsluitend eigen launchers met `Exec=/bin/false` en start alleen de Rust-testvensters. De gettext-proef leest een bestaand systeemcatalogusbestand.

De ontwikkelharness staat in `work/apps-catalog-test/run.py`; het volledige resultaat is `results.json`. De compacte distributie-uitvoer staat in `validation/apps-catalog-results.json` en bevat geen tokens, proces-ID's, persoonlijke instellingen of bestandsinhoud. De opname bewijst de werkende bestaande Start-interface; volledige pixelvergelijking, fysieke invoerhardware en alle mogelijke apps, iconenthema's en talen zijn niet uitputtend getest. De bestaande window-class-index voor overige compatibiliteitsdiensten blijft buiten deze catalogusproef.
