# Verkenner-tabbladen: validatie

> **Historisch (D-G72, 14 november 2026):** gemeten met de HTML-interface in WebKitGTK van toen. Die is verwijderd: Verkenner, Instellingen, Taakbeheer en de andere apps zijn nu native GTK 4-programma's, en Win11 voor GNOME bevat geen WebKit en geen GTK 3-host meer. De Rust-diensten die dit rapport controleert, bestaan nog. De laatste commit met WebKit is `6ebf169f`; zie [native/APPS.md](native/APPS.md).

**Herhaling 0.4.0:** de native tabcyclus met vertrouwde virtuele Wayland-pointer slaagt opnieuw in tien toestanden, inclusief losmaken en terugplaatsen, behoud van selectie/geschiedenis en sluiten na bevestiging. Compact bewijs: `validation/regressions-0.4.json`. De bredere historische basisscenario’s hieronder zijn niet allemaal opnieuw uitgevoerd.

Gecontroleerd op 1 november 2026 in een eigen schermloze GNOME 50.1-sessie. De losse Rust/WebKit-Verkenner gebruikte zijn eigen compatibiliteitsbackend en testmappen. De actieve desktop en oorspronkelijke Win11Shell-bestanden zijn niet gewijzigd.

## Gewone tabbediening

Alle 18 basisscenario's slaagden:

- Een opgegeven map opent in het eerste tabblad, met echte bestandsinhoud.
- De plusknop opent een nieuw Home-tabblad.
- Elk tabblad behoudt zijn eigen map en eigen Terug/Vooruit-geschiedenis.
- Wisselen tussen A en B herstelt de juiste inhoud, adresbalk en navigatieknoppen.
- Een submap openen verandert het huidige tabblad.
- Ctrl+T opent een tabblad direct na het actieve tabblad, overeenkomstig het origineel.
- Ctrl+Tab en Ctrl+Shift+Tab wisselen volgens de zichtbare volgorde.
- Ctrl+W sluit het actieve tabblad.
- De sluitknop van een inactief tabblad laat het actieve tabblad intact.
- Middelklikken sluit een tabblad en herstelt het naastgelegen tabblad.
- Het laatste tabblad sluiten beëindigt het Rust-app-proces met exitcode 0.

De bediening is uitgevoerd via DOM-events in de werkelijke WebKit-weergave; resultaten zijn teruggelezen uit DOM en het native testkanaal. Fysieke muis-/toetsenbordevents maken geen deel uit van deze basismeting.

Reproduceerbare test: `work/tab-test/run.py`. Uitgebreide snapshots: `work/tab-test/results.json` (18 scenario's).

Aanvullend geslaagd: `win11-explorer MAP_A MAP_B` opent beide mappen als afzonderlijke tabbladen, met A actief. Dit is afzonderlijk gemeten in `work/tab-test/cli-multiple/results.json`. Het eerdere vermoeden dat alleen het eerste argument werd gebruikt is hiermee voor de geteste binary weerlegd.

## Tabs overdragen tussen vensters

Een afzonderlijke proef met twee echte Rust/WebKit-processen en hun compatibiliteitsbackends doorloopt 18 scenario's. De sleep- en drop-events zijn hierbij via de DOM aangeboden.

Geslaagd:

- De bron houdt zijn tab voordat het doel de overdracht heeft bevestigd, ook terwijl het doel de mapinhoud laadt.
- Na bevestiging sluit alleen de overgedragen brontab.
- De doel-tab herstelt map, Terug/Vooruit-geschiedenis en geselecteerd bestand.
- Tabs zijn binnen hetzelfde venster opnieuw te ordenen.
- Annuleren houdt de brontab intact.
- Losmaken start een nieuw Rust-Verkenner-proces en herstelt de tabtoestand.
- Het oorspronkelijke venster houdt zijn andere tabblad.
- Terugplaatsen in een bestaand venster sluit het lege, losgemaakte venster.

Een aparte JavaScript-protocolproef controleert onder meer ongeldige toestand, terugrollen na mislukte bevestiging, behoud van een ondertussen gewijzigde brontab, onduidelijke drops en native annulering.

Uitgebreide gegevens: `work/tab-transfer-test/results.json`; test: `work/tab-transfer-test/run.py`. De protocolproef staat in `work/tabtransfer-test/test.mjs`. `validation/results.json` bevat de compacte uitkomsten.

Deze 18 resultaten bewijzen de toestandsoverdracht en vensterlevenscyclus met gesimuleerde DOM-sleepevents.

## Native Wayland-sleepinvoer

Een aanvullende proef slaagt met echte Verkenner-processen en een virtuele Wayland-pointer. De invoer loopt via de compositor naar WebKit; de test roept de DOM-sleepfuncties niet zelf aan. Tien vastgelegde toestanden volgen de voorbereiding, het selecteren van een bestand, overdracht naar een ander venster, losmaken naar een nieuw venster en terugplaatsen in het oorspronkelijke venster.

WebKit meldt een vertrouwde `dragstart` in de bron, een vertrouwde `drop` in het doel en een vertrouwde `dragend` met resultaat `move`. Het doel neemt de tab over en de bron sluit de overgedragen tab na bevestiging. De overige Home-tab van de bron blijft staan.

Loslaten op het bureaublad opent een nieuw Rust-Verkenner-proces. Terugslepen naar de tabstrip van het oorspronkelijke venster herstelt de tab daar en sluit het losgemaakte venster zodra diens laatste tab is overgenomen.

Resultaten: `work/tab-transfer-test/results-pointer.json`; invoerevents: `work/tab-transfer-test/native-events.json` en `native-roundtrip-events.json`; test: `work/tab-transfer-test/run-pointer.py`.

Losmaken door op het bureaublad los te laten gebruikt de GNOME-extensie om die loslaatactie te bevestigen. De afzonderlijke Verkenner-app en overdracht tussen bestaande vensters werken onafhankelijk van het zichtbare Rust-paneel.

Een aanvullende native proef slaagt voor Escape direct gevolgd door het loslaten van de muisknop. De bron houdt beide tabbladen, de overdracht wordt ingetrokken en er volgt geen opdracht om een nieuw venster te openen. Het resultaat staat in `work/tab-transfer-test/results-escape-fast-probe.json` met `assertions_passed: true`.

Geen van deze proeven gebruikt een fysieke muis of touchpad; de native proeven gebruiken virtuele Wayland-muis- en toetsenbordinvoer.
