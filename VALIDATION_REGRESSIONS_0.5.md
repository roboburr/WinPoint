# Herhaalde regressiecontroles — versie 0.5.0

Op 1 november 2026 zijn de bestaande 34 bestandsregressiegevallen opnieuw uitgevoerd tegen de uiteindelijke `target/debug`-binaries, na toevoeging van native mapbewaking en het filter voor oorspronkelijke Python-mapmeldingen. Alle 34 gevallen slagen. De bestaande harness uit `work/files-regression-0.4/run.py` is gekopieerd naar `work/files-regression-0.5/run.py`; alleen de versie in de resultaatmetadata is gewijzigd naar `0.5.0`.

De meting herhaalt onder andere HTTP/RPC-grenzen, volledige metadata- en foutenvergelijkingen met de oorspronkelijke Python-dienst, veilig aanmaken en hernoemen, gelijktijdige naamsconflicten, recursief kopiëren en verplaatsen, gedeeltelijk succes, een echte verplaatsing tussen bestandssystemen en voortgangsgebeurtenissen. Kopiëren en knippen tussen afzonderlijke Verkenners, plakken via de oorspronkelijke UI-handler, automatisch verversen via SSE, de 700-bestandselectie en de gerichte klembordvolgordegevallen blijven werken. Sluiten via de bestaande captionknop tijdens een kopie van 10.000 bestanden eindigt met alle bron- en doelbestanden intact en exitcode 0.

De bestaande taakbalk-/Start-harness is daarnaast ongewijzigd herhaald in `work/panel-test-0.5/run.py`, in een eigen GNOME 50.1/Wayland-sessie op 1440×900 bij schaal 1. De native taakbalk start, vervangt de fallback, vult de monitor, reserveert 48 pixels en ontvangt invoerregio's. Het oorspronkelijke middenblok heeft `x=500`, breedte `440` en viewportbreedte `1440`: zijn middelpunt ligt exact op `720`.

Een afzonderlijke Rust-Verkenner verschijnt als actief programma in de taakbalk. De oorspronkelijke taakbalkknop minimaliseert en herstelt het echte compositorvenster. Start opent in een afzonderlijk Rust-proces, staat eveneens gecentreerd, sluit na opnieuw klikken en sluit bij focusverlies naar Verkenner. Uitschakelen van de testextensie sluit alleen het paneel; Verkenner blijft draaien. Dit controleert ook dat de nieuwe Files-worker de gedeelde proces- en focustransportpaden niet blokkeert.

Beide runs gebruikten via een aparte `launch.py` een eigen HOME, XDG-configuratie, data, cache, runtime, desktopmap, expliciete prullenbakoverride en GSettings-keyfilebackend. Het paneel is uitsluitend in een eigen installatieprefix geplaatst en in de eigen GNOME-sessie ingeschakeld. De actieve desktopinstellingen en gebruikersbestanden zijn niet aangepast. De kruisbestandssysteemproef gebruikt daarnaast uitsluitend een zelfgemaakte tijdelijke map op `/tmp`.

Deze herhaling gebruikt geauthenticeerde HTTP, SSE en bestaande DOM-handlers in echte WebKit-vensters. Fysieke invoerhardware, de volledige taboverdracht met pointer-drag, de eerdere reeks andere resoluties en schalen, alle bestandssystemen en langdurige foutscenario's zijn hier niet opnieuw getest. De venster- en pointeroverdrachtcode is in deze backendronde niet gewijzigd; hun eerdere validatie blijft een afzonderlijke meting. De nieuwe bewakingsgevallen hebben hun eigen rapport `VALIDATION_WATCH.md`.

De uitgebreide resultaten en logs staan in `work/files-regression-0.5/results.json`, `work/files-regression-0.5/run.log`, `work/panel-test-0.5/results.json` en `work/panel-test-0.5/run.log`. Het paneelrapport bevat ook de werkelijk gemeten geometrie. Deze ontwikkelbestanden bevatten geen aangepaste productie-UI.


## Aanvullende opname met het oorspronkelijke pictogramthema

Dezelfde 1440×900-proef slaagt ook met het bestaande Win11-pictogramthema en de oorspronkelijke appvoorkeuren als alleen-lezen testinput. De voorkeuren zijn naar een private configuratie gekopieerd; pictogrammen en appcatalogus worden alleen gelezen. HOME, GSettings, runtime, desktop, Trash en installatieprefix blijven eigen testmappen. De standaardmappen zijn lege eigen fixtures. Er zijn geen gebruikersapps gestart en geen actieve desktopinstellingen gewijzigd.

In deze sessie is het middenblok `x=502`, breedte `436`, dus opnieuw exact midden `720`. Start heeft `x=399`, breedte `642`, eveneens midden `720`. Alle bestaande paneel-, venster- en focuscontroles slagen opnieuw. Deze echte opname is `preview-gnome.png`; de resultaten staan in `work/panel-preview-0.5/results.json` en compact in `validation/regressions-0.5.json`. Dit is geen uitputtende pixelvergelijking van alle schermen.
