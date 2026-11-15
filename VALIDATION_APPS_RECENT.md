# Native recente bestanden — versie 0.6.0

> **Historisch (D-G72, 14 november 2026):** gemeten met de HTML-interface in WebKitGTK van toen. Die is verwijderd: Verkenner, Instellingen, Taakbeheer en de andere apps zijn nu native GTK 4-programma's, en Win11 voor GNOME bevat geen WebKit en geen GTK 3-host meer. De Rust-diensten die dit rapport controleert, bestaan nog. De laatste commit met WebKit is `6ebf169f`; zie [native/APPS.md](native/APPS.md).

Gecontroleerd op 1 november 2026 in een eigen GNOME 50.1/Wayland-sessie. Alle achttien gegroepeerde integratiegevallen slagen met twee echte Rust/WebKit-Verkenners en een afzonderlijk Rust-Startvenster. Dezelfde XML-fixtures zijn vergeleken met de oorspronkelijke Python-dienst in een private compatibiliteitsbackend.

`apps.recent` en `apps.clearRecent` worden native afgehandeld en antwoorden met `X-Win11-Service: rust-apps`. Het Startvenster gebruikte bovendien een testkopie waarin alle openbare Python-`apps.*`-methoden bewust fouten geven. Recente bestanden ophalen en wissen bleven via de Rust-host werken. De overige compatibiliteitsdiensten zijn hierdoor niet verwijderd.

## Geslaagde vergelijking met het origineel

- De volledige resultaatvelden en volgorde komen overeen: `path`, `uri`, `name`, `mime`, `icon`, `modified`, `isDir` en `app`. De fixtures omvatten bestanden, een map, een bestandslink, een gebroken link, verdwenen bestanden, ontbrekende/ongeldige MIME, lokale URI's en een item in de eigen XDG-prullenbak dat wordt uitgesloten.
- Dubbele bladwijzers worden per gedecodeerd pad samengevoegd. Alleen een nieuwere duplicaat vervangt de eerdere; gelijke tijden behouden de eerdere URI en stabiele volgorde. De tijd gebruikt het maximum van de oorspronkelijke tijdvelden en applicatietijden. De applicatienaam behoudt de oorspronkelijke betekenis: de laatste niet-lege XML-applicatienaam, ook wanneer diens datum ouder is.
- Limieten behouden de oorspronkelijke standaard, begrenzing en typeconversie, inclusief floats, booleans, ongeldige waarden, Unicode-cijfers en toegestane underscores.
- Alleen niet-gekwalificeerde `bookmark`-elementen tellen mee; MIME en applicaties gebruiken juist hun exacte namespaces. CDATA, interne entiteiten, UTF-8 met BOM, UTF-16 en gedeclareerde ISO-8859-1 zijn vergeleken. Beschadigde XML, ongeldige UTF-8, recursieve entiteiten en externe entiteitsreferenties leveren dezelfde lege resultaten als de referentie op. Er wordt geen externe XML-bron opgehaald.
- Zeventien datumvormen zijn vergeleken met de aanwezige Python 3.14-referentie, waaronder ISO-offsets met seconden/fracties, basis- en weekdatums, een Unicode-scheidingsteken, datum zonder tijd, korte tijd, 24:00, negatieve tijdstempels en beide zomertijdovergangen in Europe/Amsterdam. Ontbrekende tijden behouden de mtime-terugval.
- Een wijziging van `privacy.recentFiles` via de native instellingen-API blokkeert de lijst in beide Rust-hosts en bereikt de oorspronkelijke Python-geheugentoestand. Opnieuw inschakelen herstelt de lijst zonder het geschiedenisbestand te wijzigen.

## Wissen, gebeurtenissen en echte Start-bediening

Wissen bewaart de exacte oorspronkelijke bytes in `.bak` en publiceert daarna de oorspronkelijke lege XBEL-inhoud. Zowel back-up als nieuwe geschiedenis gebruiken modus `0600`. Dit is gecontroleerd met ongeldige XML/binaire bytes en bestaande alleen-lezen bron- en back-upbestanden. Een back-uppad dat een map is en een niet-schrijfbare bovenliggende map leveren de oorspronkelijke vriendelijke fout op; de bron blijft intact en tijdelijke bestanden blijven niet achter.

Een geslaagde native clear levert `apps.changed {}` aan alle drie de host-eventstreams. Zeven eigen geschiedenisitems maakten de oorspronkelijke knop Meer beschikbaar in Start. De test opende die echte weergave, wiste de geschiedenis vanuit een ander proces en controleerde dat de recente bestandsrijen uit zowel de hoofdweergave als Meer verdwenen. Met de echte GNOME-vensterbridge ingeschakeld sluit Start daarna bij focusverlies naar Verkenner en eindigt het proces met exitcode 0.

De meting vond een bestaande UI-fout: bij een lege geschiedenis werd alleen het hoofdrooster geleegd; het rooster voor Meer behield oude rijen. Beide roosters worden nu geleegd zonder wijziging van de vormgeving. Een tweede bevinding betrof XML-conformiteit: de gebruikte XML-parser accepteerde een letterlijk procentteken in de waarde van een interne ENTITY-declaratie, waar Python Expat dit weigert. Een gerichte validatie bewaart ook voor dat geval de oorspronkelijke uitkomst.

## Implementatie, isolatie en grenzen

De geschiedenislogica draait in Rust met `roxmltree` en GLib voor charsetconversie; zij start geen Python-parser of externe opdracht. Het cachecontract gebruikt het pad, de wijzigingstijd en grootte. De module heeft dertien gerichte fixturetests, waaronder entiteitsaanvallen, charsetconversie, back-upfalen, symlinks en een FIFO. Een FIFO kan lezen of afsluiten niet laten wachten: uitsluitend reguliere bestanden worden gelezen. De complete centrale Rust-suite met de overige modules slaagt met 140 tests.

Alle geschiedenisaanroepen gebruikten expliciet `WIN11SHELL_RECENT_FILE` naar de eigen fixture. HOME, XDG-paden, desktop, prullenbak, GSettings-keyfilebackend en D-Bus-sessie waren privé. Er is geen echte gebruikersgeschiedenis gelezen, gewist of overgeschreven, en er zijn geen externe gebruikersapps gestart. De testkopie met geblokkeerde Python-methoden staat uitsluitend in de ontwikkelmap.

De XML-lezer begrenst invoer op 64 MiB en de boom op een miljoen knopen en gebruikt de ingebouwde begrenzing tegen entiteitsexpansie. Schema's en externe DTD-bronnen worden niet opgehaald. De suite bewijst de genoemde vormen en contracten; zij omvat niet iedere XML/DTD-variant, voortdurende externe schrijfraces, stroomuitval, alle bestandssystemen of fysieke invoerhardware. De eerste extra focusproef miste de productie-vensterbridge; na het correct inrichten van de testsessie slaagt die proef zonder wijziging van venstercode.

De reproduceerbare ontwikkelharness staat in `work/apps-recent-test/run.py`, met `launch.py`, `results.json`, `run.log` en de echte opname `apps-recent-native.png`. Het compacte distributiebewijs is `validation/apps-recent-results.json`, zonder tokens, persoonlijke geschiedenis, proces-ID's of bestandsinhoud.
