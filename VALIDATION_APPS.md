# Native Apps-service — versie 0.6.0

> **Historisch (D-G72, 14 november 2026):** gemeten met de HTML-interface in WebKitGTK van toen. Die is verwijderd: Verkenner, Instellingen, Taakbeheer en de andere apps zijn nu native GTK 4-programma's, en Win11 voor GNOME bevat geen WebKit en geen GTK 3-host meer. De Rust-diensten die dit rapport controleert, bestaan nog. De laatste commit met WebKit heeft de tag `last-webkit`; zie [native/APPS.md](native/APPS.md).

De oorspronkelijke acht Apps-API’s worden in Rust afgehandeld, met `X-Win11-Service: rust-apps`. De nieuwe host behoudt de bestaande HTML/CSS-interface. De API’s voor catalogus, recente bestanden en associaties hebben eigen uitgebreide rapporten: `VALIDATION_APPS_CATALOG.md`, `VALIDATION_APPS_RECENT.md` en `VALIDATION_APPS_DEFAULTS.md`.

## Starten en URI’s

De eigen integratieharness in `work/apps-launch-test/` slaagt op de definitieve binary met zes groepen:

- Een geïnstalleerde testapp ontvangt meerdere oorspronkelijke bestands-/URI-argumenten letterlijk, inclusief spaties, shelltekens en een fragmentteken in een echte bestandsnaam. Private host-omgevingssleutels ontbreken; er wordt geen shellstring samengesteld.
- Ongeldige appkeuzes, nulbytes, ongeldige URI-lijsten, ontbrekende apps, verboden schemes en ontbrekende lokale bestanden behouden de oorspronkelijke gewone foutobjecten.
- HTTP(S), mailto en lokale file-URI’s gaan uitsluitend naar private recorder-apps. Er wordt geen netwerkpagina, gebruikersbrowser of echte mailclient gestart. Ook hoofdletters in de scheme worden gecontroleerd.
- Een canonical eigen Verkenner opent bestanden uit dezelfde map als één selectie en extra mappen/Prullenbak als afzonderlijke tabs, in de oorspronkelijke volgorde.
- Verkenner, Instellingen, Taakbeheer, Alles en Schijven openen elk aantoonbaar in een afzonderlijk native Rust-venster. Er zijn geen proces-, schijf- of systeemwijzigingen via die apps uitgevoerd.
- List, launch en openUri blijven native werken wanneer alle acht Python-Apps-methoden in een eigen testkopie expliciet geblokkeerd zijn.

De minimale Apps-URI-transport- en Explorer-parametertests controleren ook de validatie en groepering zonder een gebruikersapp te starten. De huidige Rust-suite en andere native regressies staan in `validation/regressions-0.6.json`.

## Gerichte functionele correcties

Het oorspronkelijke Start-menu liet na volledig wissen nog file-rows in de ‘Meer’-lijst staan. Twee regels in `startmenu.js` maken die lijst voortaan ook leeg wanneer aanbevelingen uit staan of de geschiedenis leeg is. De bestaande Main- en More-indeling, CSS en gevulde inhoud blijven behouden. De echte More-weergave is vóór wissen geopend en daarna over twee hosts gecontroleerd.

Mapbewaking en cataloguswijzigingen gebruiken het oorspronkelijke debounce-tijdvenster vanaf de eerste wijziging. Doorlopend schrijven stelt verversing daardoor niet onbeperkt uit. Een fixture met een doorlopende schrijver bewijst dat meerdere mapmeldingen komen terwijl het schrijven nog loopt; de bestaande UI-watchsuite is opnieuw uitgevoerd.

Bij een ongeldig bracketed HTTP-adres geeft Rust `bad_uri` in plaats van het oorspronkelijke `internal`-antwoord met een Python ValueError. De gewone URI’s en fouten behouden hun gedrag. Uitzonderlijke terminal-IO-fouten behouden code en gebruikersbericht; het oorspronkelijke diagnostische `detail`-veld is niet volledig overgenomen.

## Isolatie en resterende grenzen

Alle nieuwe integratieproeven gebruiken eigen HOME/XDG/GSettings/runtime, testgeschiedenis, appdefinities en installatiemappen. Externe apps zijn lokale recorders; de echte native eigen apps zijn alleen in die sessie geopend. De actieve desktop, persoonlijke appassociaties, gebruikersgeschiedenis en schijven zijn niet aangepast.

De optionele oorspronkelijke koppeling die gestarte apps naar een cgroup met een andere prioriteit verplaatst is nog niet native aangesloten. De Alles-index en overige systeemdiensten blijven compatibiliteitscode. Een volledig één-op-één-resultaat voor ieder OS-/hardwaregeval, iedere dialoog of alle foutdiagnostiek is hiermee niet bewezen.

Enkele geïsoleerde testopstarts zijn vroeg uitgevallen. De oude logs leggen noch het exacte afsluitsignaal noch de latere liveness van Rust vast. Een gerichte strace-proef met twee nieuwe eigen vensters en hun backends bleef gezond tot expliciete cleanup; er is geen concrete lifecyclecodefout gevonden en de eerdere uitval is niet verklaard. De analyse staat in `work/backend-lifecycle-test/FINDINGS.md` en `results.json`. Dit blijft een alpha-beperking; de geslaagde functiegroepen gelden voor de daadwerkelijk opgevoerde gezonde sessies.

Het compacte distributiebewijs staat in `validation/apps-launch-results.json`, de overige `apps-*-results.json`-bestanden en `validation/regressions-0.6.json`. Tokens, persoonlijke voorkeuren, uitgebreide procestraces en ontwikkelfixtures worden niet verpakt.
