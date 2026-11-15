# Actuele ontwikkelcontrole — 2 november 2026

> **D-G73, 14 november 2026:** WinPoint start tijdens het draaien geen eigen Python meer. `vendor/host`, de oorspronkelijke Python-diensten die als testreferentie bleven, is verwijderd, net als het python3-apt-script waarmee Windows Update gefaseerde updates indeelde (`git show 6ebf169f:<pad>` toont beide). Die indeling doet nu `host/src/services/update/phasing.rs` met de eigen alleen-lezen opdrachten van apt (`apt-cache policy`, `apt-cache policy ?phasing` en `apt-mark showhold`; de tweede duurde op deze pc 0,23 s). De 117 unittests van de updatedienst slagen, en op opgenomen echte uitvoer van deze pc geeft de nieuwe indeling dezelfde antwoorden als het oude script. Python blijft voor de installer, het verpakken, de i18n-tools en de tests; de GNOME Shell-extensie blijft JavaScript, zoals GNOME vereist. Wat hieronder staat over de Python-diensten als testreferentie, is historisch.

> **D-G72, 14 november 2026:** de apps (Verkenner, Instellingen, Taakbeheer, Alles, Schijfbeheer, Standaardapps, Installer) zijn native GTK 4-programma's; de HTML/CSS/JavaScript-interface, WebKitGTK en de GTK 3 van de host zijn verwijderd (laatste commit met WebKit: commit `6ebf169f`). Controles van die dag op een nieuwe build van alle 19 programma's: `tools/check-no-webkit.py` zonder bevindingen (geen programma linkt WebKit, JavaScriptCore, libsoup of GTK 3); de Rust-tests van host en native en de update-, installatie- en i18n-tests slagen; alle 17 scenario's van `tests/gnome/` slagen in een private GNOME 50-sessie, in het Nederlands en tien ervan ook in het Engels. Wat hieronder over de HTML-interface staat, ook in de D-G70-noot, is historisch. De stand per app staat in [native/APPS.md](native/APPS.md), de geheugenmetingen in [notes/claude/gtk-native.md](notes/claude/gtk-native.md).

> **D-G70, 13 november 2026:** de WebKit-taakbalk, -Start en -bureaublad zijn verwijderd. De native GTK 4-shell `win11-shell` is nu de enige shell; de HTML-apps (Verkenner, Instellingen, Taakbeheer, Alles, Schijfbeheer, Standaardapps, Installer) blijven in WebKitGTK. Wat hieronder over de HTML-taakbalk, Start en het bureaublad staat, is historisch. De metingen van de native shell staan in [notes/claude/gtk-native.md](notes/claude/gtk-native.md).

De standaardruntime is nu native Rust. De oorspronkelijke Python-diensten blijven een expliciete testreferentie; installatie-, pakket- en ontwikkeltools kunnen nog Python gebruiken. Deze status vervangt de oude migratiebeschrijvingen hieronder. De oorspronkelijke HTML/CSS/JavaScript-interface blijft behouden met gerichte functionele wijzigingen en nieuwe losse apps. Er wordt geen volledige pariteit op iedere machine of een gepubliceerde openbare alpha geclaimd.

## Huidige implementatie en aantoonbare grenzen

| Onderdeel | Bewijs en grens van de actuele controle |
|---|---|
| Energie en slaapstand | Vijf Rust-unitgroepen en één private D-Bus-proef controleren echte GNOME-schema's, aangepaste tijden, ‘Nooit’, beleidssloten, beide PowerProfiles-interfaces, geweigerde en niet overgenomen wijzigingen. De instellingeninterface slaagt voor 41 browsercontroles en 12 controles in een echt Rust/WebKit-venster, inclusief opslaan in een private GNOME-configuratie en externe wijzigingen via events. Het mockcontract slaagt voor 18 controles. Er zijn geen echte energievoorkeuren gewijzigd of slaapacties uitgevoerd. Bewijs: `work/native-energy-test`. |
| Native compositie | `cargo check --offline --bins` slaagt met de gekoppelde Rust-diensten en softwaremetadatahelper. De standaardrouter gebruikt geen Python-backend. Een expliciete referentie-instantie blijft voor vergelijkende tests mogelijk. Definitieve pakket- en complete frontendcontrole worden afzonderlijk geregistreerd. |
| Alles-index | `work/native-index-test/summary.json`: 152 vergelijkende zoekgevallen, 9 workerlevenscycluscontroles, 4 cache/overlaycontroles en 3 kerntests. De oorspronkelijke v2-cache is in beide richtingen gelezen. Alleen de geopende Alles-app start een indexworker; de proeven indexeren uitsluitend private fixtures. De proef met 100.000 items gebruikte 8.636 KiB PSS voor de worker; dit is geen meting van de volledige shell of de gebruikersindex. |
| Tray | `work/native-tray-test/summary.json`: 141 vergelijkende hulpfunctiegevallen, 13 private SNI/DBusMenu-controles, 7 watcher-eigendomcontroles en 8 fout-/serialisatiegevallen. Ubuntu `@`-registraties, menuacties, signalen, iconen, ontdubbeling en verwijderen zijn met nagemaakte items getest. Er zijn daarbij geen echte tray-apps bediend. |
| Afbeeldingen | `work/native-images-test/summary.json`: 159 vergelijkende routegevallen en 58 worker/cache/veiligheidsgevallen. De oorspronkelijke afbeeldingsroutes, inhoudstypen, caches en voorwaardelijke antwoorden zijn gecontroleerd met gegenereerde beelden. Lokale bestanden worden via gevalideerde geopende bestanden gestreamd. |
| Nieuwe videominiaturen | Hetzelfde rapport: 37 backendgevallen, 9 browser-UI-gevallen en 6 echte Rust/GTK/WebKit-controles in private GNOME. Een geselecteerde zichtbare video krijgt op verzoek één miniatuur. Stale selectie, corrupte/onbeschikbare bronnen, cache en decoderfallback zijn meegenomen. Gegenereerde 1080p/4K-clips gebruikten circa 61.132/76.208 KiB maximale RSS voor de productiehelper; decodeertijd was 0,10/0,16 s in die proef. Er is geen gebruikersvideo geopend en geen algemene codecgarantie aangetoond. |
| Standaardapps | `work/native-defaultapps-test/summary.json`: 31 backendcontroles, 16 volledige browser-UI-controles en 17 bestaande appcatalogus-/associatietests. Zoeken op extensie/MIME-type, EN/NL, relevante kandidaten, expliciete bevestiging en precies één wijziging zijn gecontroleerd met private MIME/desktopbestanden. Er zijn geen persoonlijke associaties aangepast. |
| Venster- en tabherstel | `work/native-windowstate-test/summary.json`: 7 kerntests, 14 levenscyclusgevallen en 11 echte GNOME-controles. Normale positie/maat, maximaliseren en terugzetten, tabs/actieve tab, expliciet pad en losgekoppeld scherm zijn gemeten. Alleen semantische tabtoestand wordt bewaard; geen inhoudslijsten. De echte proeven gebruikten private GNOME 50.1, configuratie en testmappen. |
| Dry-run van bestanden | `work/native-files-dry-test/summary.json`: 4 plancontroles, een afzonderlijk uitgevoerde leaf-guardtest en 28 native HTTP-gevallen. Bestandsbytes en modi bleven gelijk; er ontstond geen extra appvenster. De bewust als afzonderlijk gemarkeerde leaf-test vereist de private dry-runomgeving en is niet geschikt voor een onbeveiligde normale testrun. |
| Nieuwe software-engine | `work/native-software-test/summary.json`: 122 private controles: engine42, PackageKit10, snapd20, Flatpak12, DEB-metadata6, veiligheidsgrenzen28 en Snap-metadata4. Daarnaast slagen 20 echte native HTTP-controles met een gegenereerde onschadelijke AppImage. Alleen beheerde fixturebestanden zijn geïnstalleerd/verwijderd; gedownloade code is niet uitgevoerd en echte systeemproviders zijn niet gewijzigd. Pakketauthenticatie op echte distributies en de nieuwe Installer/Instellingen/Start-frontend vereisen hun eigen integratiebewijs. |
| Desktopportals | De Rust FileChooser-backend implementeert OpenFile/SaveFile/SaveFiles met de oorspronkelijke Verkenner-interface, filters, keuzes en oudervensters. Scope en protocol staan in `host/src/portal/README.md`; `work/native-portal-test` bewaart de afzonderlijke private D-Bus/GNOME- en frontendproeven. Dit vervangt alleen kiezers van apps die desktopportals gebruiken. Registratie, selectie en verpakking blijven afzonderlijk aantoonbare stappen. |

De bestands-, instellingen-, app-, systeem-, media-, beeldscherm-, opslag-, notificatie-, klembord- en shellfuncties zijn native gekoppeld. De tabellen hierboven benoemen de specifieke hier samengevatte nieuwe metingen. De oudere bestands- en instellingenrapporten blijven nuttig voor hun gemeten scenario's, maar worden daardoor niet automatisch nieuwe complete regressiemetingen.

## Geheugen en behoud van het Startmenu

`work/native-memory-test/results.json` bevat een A/B-proef in een private GNOME-sessie van 1440×900 met softwarerendering. Diensten op verzoek, WebKit-cachebeleid, gedeelde context binnen één host en minder achtergrondwerk in verborgen Start verlaagden het gestabiliseerde totaal voor Panel, Desktop en voorgeladen Start van **399,84 naar 377,40 MiB PSS**. Alle bijbehorende WebKit-processen zijn meegeteld. PSS verdeelt gedeelde pagina's proportioneel; een optelsom van RSS is een andere maat.

Het geopende Startmenu en sluitframes op **50, 100 en 140 ms waren pixelgelijk** tussen de twee varianten. Dezelfde proef behield de neerwaartse sluitanimatie en afgeronde uitsnede. Dit toont behoud van de gecontroleerde frames en werklast, geen onbeperkte garantie voor iedere GPU of resolutie. Een eerdere meting op de actieve desktop van ongeveer **435 MiB PSS** gebruikte een andere werklast en is geen rechtstreeks vergelijkbare baseline.

Het systemd-beleid geeft de shell een eigen begrensde groep als de gebruikersmanager beschikbaar is. Normale apps blijven apart; de indexer bestaat alleen tijdens gebruik van Alles. Extra apps, decoders en andere schermconfiguraties zijn niet door bovenstaande totaalcijfers gedekt. Matglas (D-G59) staat standaard aan voor de taakbalk, het actieve eigen app-venster en het geopende Startmenu; `frosted-glass` (22 controles, D-G60 met heropenen van Start en slepen) mat daarvoor 3,8 en 8,9 MiB extra PSS voor gnome-shell in deze 1440×900-sessie. GPU-geheugen op een echte videokaart valt daarbuiten.

## Grenzen voor uitgave

Deze ontwikkelstatus is geen openbare alpha-uitgave. De geïntegreerde definitieve binaries, nieuwe frontend, portalregistratie, installatie/bijwerken/verwijderen, assetmanifesten en ondersteunde Ubuntu/GNOME-/schaal-/monitorcombinaties moeten samen worden vastgelegd voordat een download wordt gepubliceerd. Er zijn geen echte systeemsoftware-installaties of verwijderingen als destructieve test uitgevoerd. Historische incidentele opstart-/Mutter-waarschuwingen blijven onder hun oorspronkelijke rapport herkenbaar en zijn niet met terugwerkende kracht opgelost verklaard.

De hieronder bewaarde rapporten gaan over hun vermelde eerdere versies. Oude uitspraken over tijdelijke Python-diensten, aantallen UI-bestanden of ontbrekende functies beschrijven die historische builds.

---

## Historisch rapport — 1 november 2026

**Eerdere meting: versie 0.6.1.** De definitieve build slaagt voor 140 Rust-tests, 26 GNOME-bovenbalk-/lifecyclecontroles en 13 Start-groepen. Start gebruikt opnieuw de oorspronkelijke opwaartse 250 ms en neerwaartse 167 ms, met hergebruik van één vooraf geladen proces. Elf echte DOM-groepen bevestigen dat de Alles-indexvoortgang niet opnieuw binnenkomt of verticaal verspringt. Zie `VALIDATION_TOPBAR.md`, `VALIDATION_ALLES_PROGRESS.md` en `validation/regressions-0.6.1.json`; het Start-rapport van deze versie is met de WebKit-Start verwijderd (D-G70).

De API-integraties hieronder blijven herkenbaar eerdere metingen. De onverklaarde incidentele vroege paneelafsluiting, de secundaire CLI-waarschuwing en de afzonderlijke Mutter-opruimwaarschuwingen blijven expliciet als grenzen vermeld.

**Eerdere meting: versie 0.6.0.** Alle acht Apps-methoden draaien native naast de 28 Files-methoden en zeven Settings-methoden. De centrale suite slaagt met 140 Rust-tests. De nieuwe Apps-suites slagen met 46 groepen: catalogus10, standaardapps12, recente bestanden18 en starten6. De bestaande bestanden slagen opnieuw met34 gevallen; de watch/UI-suite met8. De 1440×900 paneel-/Startproef met het oorspronkelijke pictogramthema en private voorkeurenkopie slaagt eveneens. Zie `VALIDATION_APPS.md`, de bijbehorende drie specialistische app-rapporten en `validation/regressions-0.6.json`.

De Start-‘Meer’-lijst wist nu tegelijk met de hoofdlijst wanneer geschiedenis leeg is of aanbevelingen uit staan. Dit verandert twee functionele regels, geen CSS of gevulde indeling. Het oorspronkelijke refreshvenster is hersteld bij doorlopende mapwijzigingen. Een nieuwe getraceerde twee-venster-lifecycleproef bleef gezond; historische incidentele vroege opstartuitval is niet verklaard en blijft een alpha-beperking. Eerdere rapporten hieronder zijn herkenbaar historische metingen.


**Eerdere meting: versie 0.5.0.** Alle 28 oorspronkelijke openbare Files-methoden zijn nu native afgehandeld. De centrale suite slaagt met 104 Rust-tests. Twaalf gecombineerde desktopgroepen, acht echte watch/UI-groepen, 166 zoekcontroles en 34 bestaande bestandsregressies slagen. De taakbalk staat bij de herhaalde 1440×900-proef exact in het midden; native minimaliseren/herstellen, Start en focusgedrag slagen. Zie `VALIDATION_FILES_DESKTOP.md`, `VALIDATION_SEARCH.md`, `VALIDATION_WATCH.md`, `VALIDATION_ACTIONS.md`, `VALIDATION_REGRESSIONS_0.5.md` en `validation/regressions-0.5.json`.

De codewijziging voor lange runtimepaden is ook gecontroleerd met de oorspronkelijk problematische lange TMPDIR: hostopstart, native zoekaanroep en de oorspronkelijke WebKit-bridge slagen. De UI-baseline blijft ongewijzigd ten opzichte van 0.4.0. De oudere rapporten hieronder blijven historische metingen; niet iedere eerdere app-, tab- en schermcombinatie is deze ronde opnieuw uitgevoerd.


Versie **0.4.0** voegt acht native Verkenner-methoden en zeven native Instellingen-methoden toe. De oorspronkelijke interfacebestanden zijn in deze versie niet aangepast. Concrete API-, echte WebKit- en synchronisatieproeven staan in `VALIDATION_FILES_COMPLETE.md` en `VALIDATION_SETTINGS.md`; de compacte actuele herhalingen in `validation/regressions-0.4.json`.

De centrale suite van 0.4.0 slaagt met **78 Rust-tests**. De nieuwe bestandsintegratie slaagt met 22 gevallen; Instellingen met twaalf groepen (waaronder de aanvullende GIO-prullenbakproef). De eerdere bestandskern is herhaald met 34 geslaagde gevallen. Het bestaande taakbalkpaneel is opnieuw exact gecentreerd gemeten op 1440×900 bij schaal 1; vensterminimalisatie/herstel en Start-toggle/focus sluiten slagen. De volledige taboverdracht met virtuele Wayland-pointer slaagt opnieuw in tien toestanden, inclusief losmaken naar een eigen venster en terugplaatsen.

Eén eerste geïsoleerde regressie-opstart liep vast nadat de eigen compatibiliteitsbackend een afsluitsignaal ontving. De oorzaak van dat signaal is niet vastgesteld; de volledige schone herhaling slaagde. Beide pogingen zijn als ontwikkellogs behouden. De nieuwe controles wijzigen de actieve desktop niet.

De onderstaande 0.3.0- en 0.2.0-metingen blijven historisch bewijs. Alleen expliciet in de nieuwe rapporten genoemde herhalingen zijn ook met 0.4.0 uitgevoerd.

Versie **0.3.0** voegt een Rust-bestandskern met twaalf methoden toe. De centrale Rust-suite van 0.3.0 telt 37 geslaagde tests, inclusief de nieuwe bestandsmetadata, mutaties, gedeeld bestandsklembord en eventafhandeling. De concrete bestandsintegratie en haar grenzen staan in **`VALIDATION_FILES.md`** en `validation/files-results.json`.

## Opnieuw gecontroleerd met versie 0.3.0

| Onderdeel | Actueel gemeten resultaat |
|---|---|
| Rust-unit-tests | Alle 37 tests slagen. |
| Rust-bestandskern | Alle 34 integratiegevallen slagen in drie echte Rust/WebKit-Verkenners, waaronder metadata-pariteit, mutaties, gedeeld bestandsklembord, grote selecties, automatische verversing en sluiten tijdens een lopende kopie. Zie `VALIDATION_FILES.md`. |
| Klembordvolgorde | Vier gecontroleerde proeven met oudere SSE-berichten, vertraagde Get- en Set-antwoorden en een vertraagde Get-fout zijn binnen de suite uitgevoerd en daarna afzonderlijk herhaald. De nieuwere selectie en zichtbare knipstatus blijven behouden. |
| Native taboverdracht | De volledige cyclus slaagt opnieuw in tien toestanden: tussen bestaande vensters slepen, op het bureaublad losmaken naar een eigen Rust-venster en terugplaatsen. Map, geschiedenis en selectie blijven behouden; het lege losgemaakte venster sluit. Hiervoor is een virtuele Wayland-pointer gebruikt. |
| HTML-paneel en Start | Op 1440×900 bij schaal 1 ligt het taakbalkmidden exact op x=502+436/2=720. Het paneel valt samen met de monitor. Het Verkenner-pictogram minimaliseert en herstelt het venster; Start is 642×726, staat onderaan gecentreerd en sluit via een tweede klik of native focusverlies. Uitschakelen sluit alleen het paneel. |

De compacte actuele samenvatting staat in `validation/regressions-0.3.json`. De ontwikkelresultaten zijn `work/files-native-test/results.json`, `work/files-native-test/ordering-results.json`, `work/tab-transfer-test/results-pointer.json` en `work/panel-test-0.3/results.json`.

De oudere shell-, app- en resolutiemetingen hieronder horen bij **versie 0.2.0**. Ze blijven beschikbaar als historisch bewijs en betekenen niet dat iedere combinatie opnieuw met 0.3.0 is uitgevoerd. De tabel hierboven noemt de daadwerkelijk herhaalde controles.

## Historische shellcontrole van versie 0.2.0

Dit historische deel beschrijft de gemeten werking van versie 0.2.0 op GNOME Shell 50.1. De controles draaiden in afzonderlijke schermloze GNOME/Wayland-sessies met eigen configuratie, runtime en testmappen. De oorspronkelijke Win11Shell-installatie en actieve desktop zijn daarbij niet aangepast.

De apps gebruiken de bestaande HTML/CSS-interface in Rust/GTK 3/WebKit-vensters. De oorspronkelijke Python-diensten blijven tijdelijk onderdeel van de runtime. Deze controles tonen geen volledige Rust-port of volledige één-op-één-pariteit van alle shellfuncties aan.

De meegeleverde alpha gebruikt het geteste Cargo-profiel `dev`, zonder debug-symbolen en met incrementeel bouwen uitgeschakeld. Er is geen releaseprestatie- of geheugenvergelijking uitgevoerd.

## Geslaagde controles in versie 0.2.0

| Onderdeel | Gemeten resultaat |
|---|---|
| Verkenner-venster | De oorspronkelijke vensterknoppen minimaliseren, maximaliseren, herstellen en sluiten het echte GNOME-venster. Sluiten eindigt met exitcode 0. |
| Overige losse apps | Instellingen, Taakbeheer, Alles en Schijven laden hun oorspronkelijke interface, bestaan tegelijk als vier afzonderlijke Rust-processen en doorlopen elk maximaliseren, herstellen, minimaliseren, terugzetten en sluiten met exitcode 0. |
| Oorspronkelijke HTML-taakbalk | Het Rust-paneel wordt als dock herkend, registreert invoerregio's en vervangt de eenvoudige GJS-fallback. Het pictogram van een echt Verkenner-venster geeft de actieve toestand weer en minimaliseert en herstelt dat venster. |
| Centrering van HTML-taakbalk | Het midden ligt exact op het schermmidden bij 1024×768, 1440×900 en 1920×1080 op schaal 1, en bij een virtuele monitor met modus 2560×1440 op Mutter-schaal 2. Die laatste heeft een logische viewport van 1280×720. |
| Start | De oorspronkelijke interface opent in een eigen proces, onderaan gecentreerd met 12 logische pixels tussenruimte boven de taakbalk. Het venster past binnen de beschikbare ruimte: 642×726 op 1920×1080, 642×664 op 1024×768 en 642×616 bij de schaal-2-proef. Een tweede Startklik en focus naar een andere app sluiten het menu. |
| Uitzetten van extensie | Het Rust-paneel sluit; het afzonderlijke Verkenner-proces blijft actief. |
| Verkenner-tabs | 18 basisscenario's en 18 overdrachtsscenario's slagen met DOM-events in echte WebKit-vensters. Map, geschiedenis en selectie blijven bij overdracht behouden; de bron sluit pas na bevestiging door het doel. Zie `VALIDATION_TABS.md`. |
| Native slepen en losmaken | Met een virtuele Wayland-pointer is een tab overgedragen naar een ander echt Verkenner-venster, op het bureaublad losgelaten om een nieuw venster te openen en teruggesleept naar het oorspronkelijke venster. Het lege losgemaakte venster sluit. De proef legt tien toestanden vast; WebKit registreert vertrouwde sleepevents. |
| Native sleepactie annuleren | Escape direct gevolgd door het loslaten van de muisknop annuleert de sleepactie. De twee brontabs blijven staan en er volgt geen opdracht om een nieuw venster te openen. Dit is met virtuele Wayland-toetsenbord- en muisinvoer gecontroleerd. |
| UI-integriteit | Alle 162 distributiebestanden komen overeen met het huidige hashmanifest. De baseline bevat 161 bestanden; drie modules zijn gewijzigd en één module is toegevoegd. |
| Rust-unit-tests | Alle 13 tests slagen, waaronder invoerregio's, native antwoorden, app-argumenten, taboverdracht, titelbalkregio's, instellingenimport en het onderscheid tussen een bevestigde desktop-drop en annuleren. |
| Abrupte host-exit | Na zowel SIGTERM als SIGKILL van de eigen testhost stopt de bijbehorende Python-dienst in 0,252 seconde met exitcode 0. Een expliciet gekoppelde externe backend blijft in beide gevallen actief. |

De vensterknoppen en taakbalkpictogrammen zijn in deze controles via DOM-acties bediend; de gevolgen zijn onafhankelijk teruggelezen uit de echte GNOME-vensters en de app-processen. Dit is geen meting met een fysiek invoerapparaat.

De schaal-2-proef gebruikt de werkelijk toegepaste monitorconfiguratie van Mutter, geen vergroting van alleen de GNOME-interface. De paneelrand valt daarbij exact samen met de logische monitorrand. Het taakbalkmidden is x=422+436/2=640 in een viewport van 1280 pixels. In die sessie slagen ook minimaliseren/terugzetten via het taakbalkpictogram, Start-toggle, Start sluiten bij focusverlies en het afzonderlijk uitschakelen van het paneel.

De eenvoudige GJS-fallback is afzonderlijk op centrering gecontroleerd: 49 metingen over 1024×768, 1280×720, 1440×900, 1920×1080 en 2560×1440, met onder meer schaalwisselingen, overloop van 83 pictogrammen en opnieuw inschakelen. Deze resultaten gelden voor de fallback en bewijzen geen aanvullende resoluties voor het oorspronkelijke HTML-paneel.

## Grenzen van deze controle

- De native sleepcyclus en Escape-annulering zijn gecontroleerd met virtuele Wayland-invoer. De 18 bredere overdrachtsscenario's gebruiken gesimuleerde DOM-sleepevents. Er is geen fysiek invoerapparaat getest.
- Andere resoluties en schaalfactoren dan de genoemde gevallen voor het HTML-paneel, fractionele schaling, het opnieuw aankoppelen van fysieke monitoren en een volledig desktopgebruik zijn nog niet in dit rapport bewezen.
- Na een abrupte host-exit kan een tijdelijke runtime-map met `compat.log` achterblijven. Het proces en zijn instancebestand zijn in de onderzochte SIGTERM- en SIGKILL-gevallen wel opgeruimd.
- De inhoudelijke functies van alle apps zijn niet integraal getest. De test van Schijven heeft bijvoorbeeld geen schijfwijzigingen uitgevoerd. Bestandsbewerkingen, systeemacties, alle flyouts en alle combinaties van schermen en invoerapparaten vallen buiten deze meting.
- Een eerste gelijktijdige app-proef bereikte de Taakbeheer-readiness niet binnen 60 seconden. Een afzonderlijke herhaling en de daaropvolgende proef met alle vier apps slaagden. Een structurele oorzaak van die eerste timeout is niet vastgesteld.

## Bewijs en herhaling

`validation/results.json` bevat een compacte samenvatting zonder persoonlijke instellingen, tokens, proces-ID's of bestandsinhoud. De ontwikkelwerkmap bevat de uitgebreide resultaten en tests:

| Test | Resultaat |
|---|---|
| `work/control-test/run.py` | `work/control-test/result.json` |
| `work/generic-test/run.py` | `work/generic-test/results.json` |
| `work/panel-test/run.py` | `work/panel-test/results.json` |
| `work/panel-test/run.py` met aparte resolutie/testmap | `work/panel-test-1920/results.json` |
| `work/panel-test/run.py` met aparte resolutie/testmap | `work/panel-test-1024/results.json` |
| `work/panel-test/run.py` met echte Mutter-schaal 2 | `work/panel-test-scale2/results.json` |
| `work/tab-test/run.py` | `work/tab-test/results.json` |
| `work/tab-transfer-test/run.py` | `work/tab-transfer-test/results.json` |
| `work/tab-transfer-test/run-pointer.py` | `work/tab-transfer-test/results-pointer.json`, `native-events.json` en `native-roundtrip-events.json` |
| Native Escape-proef in `work/tab-transfer-test/` | `work/tab-transfer-test/results-escape-fast-probe.json` |
| `work/center-test/` | `work/center-test/results.json` |
| `work/backend-check/crash_check.py` | `work/backend-check/crash-results.json` |

De `work/`-map maakt geen deel uit van het distributiearchief. De tests zijn ontwikkelharnassen voor deze geïsoleerde omgeving, geen automatisch uit te voeren installatieonderdeel. `python3 scripts/verify-ui.py` controleert de meegeleverde interfacebestanden zonder een desktopsessie te starten. Installatie- en verpakkingscontroles staan in `PACKAGING_VALIDATION.md`.
