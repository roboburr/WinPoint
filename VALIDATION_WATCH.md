# Native mapbewaking — versie 0.5.0

Gecontroleerd op 1 november 2026 in een eigen GNOME 50.1/Wayland-sessie. Alle acht gegroepeerde integratiegevallen slagen met twee echte Rust/WebKit-Verkenners. De oorspronkelijke HTML/CSS en gebeurtenishandlers zijn behouden.

`files.watch` en `files.unwatch` worden native afgehandeld en antwoorden met `X-Win11-Service: rust-files`. Beide vensters gebruikten een testkopie van de Python-dienst waarin juist deze twee methoden bewust fouten geven. Automatisch verversen bleef werken met gewone bestandswijzigingen buiten de bestandsdienst.

## Geslaagde controles

- Een extern aangemaakt bestand verschijnt vanzelf in beide oorspronkelijke bestandslijsten. Beide eventstreams krijgen precies één mapmelding voor deze wijziging. De monitoren sturen lokaal, zodat twee bewakende hosts hun meldingen niet onderling vermenigvuldigen.
- Extern hernoemen en verwijderen worden automatisch zichtbaar, zonder handmatig vernieuwen.
- Een map blijft bewaakt terwijl een ander tabblad actief is. Bij terugschakelen toont het oorspronkelijke tabblad de bijgewerkte inhoud.
- Sluiten van het laatste tabblad voor map A roept de native `unwatch` aan. Een latere wijziging in A bereikt vervolgens uitsluitend de tweede host, die A nog open heeft. Dit controleert zowel afmelden als de lokale gebeurtenisroute.
- Twee aanmeldingen en één afmelding laten één referentie actief. Zes kort opeenvolgende bestandscreaties leveren één gebundelde melding op. Na de laatste afmelding veroorzaken nieuwe wijzigingen geen melding meer.
- De desktopmap wordt vanaf het starten gevolgd, zonder openbare aanmelding. Extern aanmaken levert zowel `files.dirChanged` als `files.desktopChanged` op.
- Een extern toegevoegd item in de expliciete testprullenbak verschijnt vanzelf in het oorspronkelijke prullenbaktabblad. Native cache-invalidation, `files.trashChanged` en het `trash:///`-mappad zijn waargenomen. Extern verwijderen van dezelfde eigen fixture ververst de lijst opnieuw.

## Threadmodel en levensduur

Eén eigen workerthread bezit de GLib-context, hoofdloop, GIO-monitoren en timers. Alleen gewone opdrachten, paden en antwoorden gaan tussen threads; GIO-objecten worden niet kunstmatig `Send` gemaakt. De bewaking behoudt de referentietellingen, maximaal 64 actieve monitoren, LRU-vervanging en de vastgehouden desktopregistratie. Meldingen worden per map 300 ms gebundeld. Afsluiten annuleert monitoren en geplande timers en wacht tot de thread stopt.

De module bevat daarnaast zeven gerichte fixturetests voor echte GIO-gebeurtenissen in een eigen GLib-context, bundeling, referentietellingen, afmelden, desktopregistratie, de LRU-grens, prullenbakmeldingen, een mapsymlink, een symlinklus en afsluiten met een geplande melding. Hiervoor worden geen globale XDG-omgevingsvariabelen of gebruikersmappen gebruikt.

De upstream SSE-route onderdrukt alleen de oorspronkelijke Python-map-, desktop- en prullenbakmeldingen zolang de native bewaking actief is. Expliciet aangesloten externe compatibiliteitsdiensten behouden hun oorspronkelijke bewaking. Eigen native bestandsbewerkingen behouden hun bestaande directe meldingen; een daaropvolgende GIO-melding kan daarnaast plaatsvinden.

## Isolatie en grenzen

Alle appprocessen gebruikten een eigen `HOME`, XDG-configuratie, data, cache, runtime, GSettings-keyfilebackend en expliciete testprullenbak. De desktopmap was een eigen fixture. De actieve desktop, gebruikersprullenbak en persoonlijke instellingen zijn niet gewijzigd. Er zijn geen permanente prullenbak-API-aanroepen gebruikt.

De bediening gebruikt de bestaande DOM-handlers, geauthenticeerde HTTP en echte SSE-streams. De tests vormen geen volledige dekking van netwerkbestanden, mountwisselingen, alle GIO-backends, voortdurende hoge gebeurtenisbelasting of alle mogelijke afsluitraces. Desktop en thuisprullenbak bestonden bij opstarten; het later aanmaken van ontbrekende bewakingsmappen is niet afzonderlijk getest.

De ontwikkelharness staat in `work/native-watch-ui-test/run.py`, met `results.json` en een echte opname `native-watch-ui.png`. Het compacte distributiebewijs is `validation/watch-results.json`, zonder tokens, persoonlijke instellingen, proces-ID's of bestandsinhoud.

## Herhaling en timercorrectie in 0.6.0

Alle acht oorspronkelijke UI-integratiegevallen zijn opnieuw uitgevoerd tegen de uiteindelijke 0.6.0-binaries en slagen. De kopie in `work/native-watch-ui-test-0.6/run.py` verandert uitsluitend de resultaatversie; een aparte launcher geeft ook de D-Bus-serviceactivatie eigen HOME- en XDG-paden. Het compacte bewijs staat in `validation/watch-results-0.6.json`.

Een bronvergelijking vond dat de oorspronkelijke timer 300 ms na de eerste melding afgaat; de eerste native versie schoof het tijdstip bij iedere nieuwe melding op. Dit is gecorrigeerd, zodat doorlopende schrijfacties de UI periodiek blijven verversen. Een achtste module-fixture herschrijft daadwerkelijk iedere 25 ms een bestand en vereist ten minste twee mapmeldingen voordat de schrijver wordt gestopt. Ook deze test slaagt in de centrale reeks van 140 Rust-tests. De herhaalde UI-suite bevestigt daarnaast dat korte bursts één melding blijven geven en dat referentietellingen, tabblad-afmelding, desktop en prullenbak blijven werken.
