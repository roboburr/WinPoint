# Rust-bestandskern — versie 0.3.0

Gecontroleerd op 1 november 2026 in een eigen schermloze GNOME 50.1/Wayland-sessie. Alle 34 integratiegevallen slagen. Drie echte Rust/WebKit-Verkenners gebruikten hun eigen compatibiliteitsdiensten, een gedeelde private runtime en uitsluitend eigen tijdelijke testbestanden. De actieve desktopconfiguratie is niet gebruikt voor de test.

## Wat nu in Rust wordt uitgevoerd

De gemeten lokale paden lopen voor deze twaalf methoden door de nieuwe Rust-bestandskern:

`files.list`, `files.stat`, `files.desktop`, `files.createFolder`, `files.createFile`, `files.rename`, `files.copy`, `files.move`, `files.clipboardSet`, `files.clipboardGet`, `files.clipboardClear` en `files.paste`.

Native antwoorden zijn herkenbaar aan de HTTP-header `X-Win11-Service: rust-files`. Als extra bewijs gebruikte de derde Verkenner een **testkopie** van de Python-backend waarin de twaalf corresponderende methoden bewust een fout geven. De native methoden bleven werken. Dit schakelt de overige Python-diensten niet uit en verandert de gedistribueerde Python-bron niet.

Launchermetadata en lijsten waarin `.desktop`-items voorkomen blijven bewust bij de oorspronkelijke dienst, zodat de bestaande controle op vertrouwde snelkoppelingen, namen en pictogrammen behouden blijft. Ook onder andere de Prullenbak, eigenschappen, attributen, openen, archieven, zoeken en watchregistratie zijn nog compatibiliteitsfuncties. Dit is een gedeeltelijke backendmigratie.

## Geslaagde controles

- Metadata en natuurlijke sortering komen op dezelfde fixtures volledig overeen met de oorspronkelijke Python-resultaten, met en zonder verborgen bestanden. De fixture bevat namen met accenten en cijfers, `.hidden`, puntbestanden, namen met `~`, een ongeldige UTF-8-naam en een FIFO.
- Normale bestanden, symlinks naar bestanden en mappen, een gebroken symlink en de FIFO behouden het bestaande `FileItem`-contract. Paden, URI's, MIME, pictogrammen, typenamen, grootte en milliseconden-mtime zijn vergeleken.
- De desktoplijst, file-URI-decodering en lexicale padnormalisatie werken; ongeldige paden en een bestand dat als map wordt geopend geven de juiste foutcodes.
- De selectieve compatibiliteitsroute voor een onbetrouwbare `.desktop`-launcher levert precies de oorspronkelijke metadata op.
- Aanmaken behoudt exclusieve creatie, nummering en samengestelde extensies zoals `.tar.gz`. Nieuwe items zijn met equivalente Python-bewerkingen vergeleken; alleen de werkelijk verschillende aanmaaktijd is daarbij buiten de vergelijking gelaten.
- Acht gelijktijdige aanmaakverzoeken krijgen acht verschillende namen. Acht gelijktijdige hernoemverzoeken naar één naam leveren precies één winnaar op. De zeven andere bronnen en een bestaand doelbestand blijven behouden.
- Hernoemen overschrijft geen bestaand bestand of gebroken doelsymlink. Ongeldige namen behouden de oorspronkelijke foutcodes.
- Kopiëren en verplaatsen behouden dezelfde conflictbenaming, dezelfde-map-kopieën en dezelfde-map-verplaatsingen. Recursief kopiëren behoudt symlinks en weigert een map in zichzelf te kopiëren. Een gemengde kopie met een FIFO geeft bruikbare gedeeltelijke succesresultaten.
- Een echte kruisbestandssysteemverplaatsing gaat van de werkmap naar een eigen tijdelijke map op `/tmp` en terug. Inhoud en symlinks blijven behouden; een hardlinkdoel buiten de bronboom blijft intact.
- De gecombineerde eventstream levert voortgang met de oorspronkelijke `jobId`, velden en start/eindstatus.
- De HTTP-grens weigert schrijfacties via GET met 405, een vreemde Origin met 403, ongeldige JSON/niet-object-RPC-bodies met `bad_request` en niet-objectargumenten met `bad_args`. De bedoelde bestanden worden niet aangemaakt; de eigen Origin blijft werken.
- Kopiëren in de ene Verkenner is beschikbaar in de andere. Plakken via de oorspronkelijke Ctrl+V-handler maakt het juiste bestand aan en ververst de zichtbare inhoud.
- Knippen in het ene proces en plakken in het andere verplaatst het bestand en wist de gedeelde knipstatus. Een leeg klembord geeft de juiste fout.
- Een selectie van 700 echte bestanden overschrijdt de 60-KiB-datagramgrens. De volledige selectie verschijnt via SSE en Get in het andere proces; plakken maakt alle 700 bestanden met de juiste inhoud aan.
- De UI behoudt de nieuwste knipmarkering en een ingeschakelde Plakken-knop wanneer een ouder SSE-event, een legacy-event zonder versie, een vertraagd ouder Get-antwoord, een vertraagde Get-fout of een vertraagd ouder Set-antwoord wordt afgeleverd. De test gebruikt daadwerkelijk verkregen serverversies; er zijn geen toekomstige versienummers geïnjecteerd. De vier ordeningsgevallen zijn daarnaast gericht herhaald met een expliciete UI-barrière: de oude kopieerstatus is zichtbaar toegepast voordat het oude antwoord wordt vastgehouden en de nieuwe knipstatus wordt afgeleverd. Ook die herhaling slaagt.
- Sluiten via de oorspronkelijke captionknop tijdens een lopende kopie van 10.000 bestanden eindigt met exitcode 0 en alle 10.000 bron- en doelbestanden intact. Een antwoord of laatste voortgangsbericht aan het sluitende venster is geen blijvende taak-API-garantie; de waarneming staat apart in de resultaat-JSON.
- Aanmaken vanuit een ander Verkenner-proces levert een mapwijziging en automatische DOM-refresh op.

## Grenzen en eerdere bevindingen

De bediening gebruikt geauthenticeerde HTTP-aanroepen en DOM-toetsenbordevents in echte WebKit-vensters. Fysieke muis- of toetsenbordhardware is in deze meting niet gebruikt. Deze suite bewijst geen volledige ondersteuning van alle bestandssystemen, netwerkshares, mountwisselingen of alle oorspronkelijke bestandsfuncties.

De eerste proef vond dat kleine SSE-berichten door de standaard chunkbuffer van de HTTP-server bleven wachten; expliciet doorspoelen van headers en events heeft dit verholpen. Een tweede proef vond een verschil tussen een lege MIME-databuffer en een ontbrekende buffer: enkele JSON- en symlink-items werden ten onrechte als leeg document weergegeven. De bestandnaamdetectie gebruikt nu dezelfde ontbrekende buffer als het origineel; de uiteindelijke vergelijking slaagt.

De vormgeving is behouden. `main.js` bevat een aanvullende klembordversie-adapter die verouderde antwoorden en events negeert. De API en events voegen daarvoor het optionele veld `_revision` toe; de visuele vormgeving blijft behouden. De vorige rapporten over vensterknoppen, taakbalk, Start en taboverdracht beschrijven hun eigen afzonderlijke metingen; deze suite herhaalt die volledige bediening niet.

## Bewijs

De reproduceerbare ontwikkelharness staat in `work/files-native-test/run.py`; de uitgebreide samenvatting staat in `work/files-native-test/results.json`. De gerichte ordeningsherhaling staat in `run-ordering.py` met `ordering-results.json` in dezelfde ontwikkelmap. Deze ontwikkelmap maakt geen onderdeel uit van de installatie. Het distributierapport `validation/files-results.json` bevat de geslaagde gevallen zonder tokens, persoonlijke instellingen, proces-ID's of bestandsinhoud.
