# Rust-bestandszoeken — versie 0.5.0

Gecontroleerd op 1 november 2026 in een eigen schermloze GNOME 50.1/Wayland-sessie met echte Rust/WebKit-Verkennerprocessen. Alle **166 gevallen** slagen. De publieke `files.find`-antwoorden dragen `X-Win11-Service: rust-files`.

## Vergelijking met de oorspronkelijke dienst

164 gevallen vergelijken het volledige native resultaat of foutobject rechtstreeks met `files.find` in de eigen private Python-backend van hetzelfde venster. De vergelijking omvat de volgorde en alle itemvelden, waaronder naam, pad, URI, pictogram, MIME, type, tijd, verborgenstatus en symlinkmetadata.

- 140 querygevallen: 30 queryprofielen met verborgen bestanden uit en aan, plus 80 gerichte globvarianten. Getest zijn Unicode-casefold (`Straße`, Griekse sigma, Turkse `İ`, ligaturen), meerdere zoektermen, Unicode-witruimte, het onderscheid tussen samengestelde en ontlede accenten, jokers over regeleinden en niet-ASCII-cijfers.
- De globmatrix omvat `*`, `?`, positieve en ontkennende klassen, letterlijke haakjes en backslashes, ontbrekende sluithaakjes, omgekeerde en dubbelzinnige bereiken, leidende/afsluitende koppeltekens en tekens die in reguliere expressies setoperatoren kunnen vormen.
- 19 optiegevallen: resultaatlimieten en hun getal-/stringconversie, MIME-prefixen, de oorspronkelijke betekenis van lege en onware waarden, verborgenstatus en tijdsbudgetten.
- Vijf foutgevallen: ongeldige queries, meer dan 500 Unicode-tekens, een bestand als zoekwortel en een ontbrekende map.

De boom bevat bestanden op verschillende diepten, verborgen submappen, `.hidden`-vermeldingen, back-upnamen, symlinks naar mappen, een symlinklus, een kapotte link, een FIFO en ongeldige UTF-8-bestandsnamen. Resultaatvolgorde en begrenzing volgen de oorspronkelijke breedte-eerstzoektocht in de bestandsysteemvolgorde. Symlinkmappen worden als resultaat weergegeven en niet recursief bezocht. De zeven aanvullende Rust-unittests controleren onder meer verlopen deadlines, de mapgrens, expliciete symlinkwortels en het uitblijven van een blokkerende FIFO-read.

## Native route en bestaande interface

Twee aanvullende gevallen gebruiken een testkopie waarin de Python-`files.find` bewust altijd een fout geeft. De zoekopdracht slaagt zowel via geauthenticeerde HTTP naar de Rust-host als via de oorspronkelijke bridge in een echt WebKit-venster. Beide antwoorden bevatten hetzelfde recursief gevonden bestand.

Het oorspronkelijke zoekvak van Verkenner opent bij Enter de afzonderlijke Alles-app. Dat gedrag en de bestaande HTML/CSS zijn behouden. De bridgeproef controleert de native `files.find`-API in het bestaande venster; de Alles-zoekindex behoort tot de overige diensten.

## Isolatie en opstartregressie

De appprocessen gebruiken een eigen `HOME`, XDG-configuratie, data, cache, runtime en GSettings-keyfilebackend. Alle bestandsmutaties blijven in de testfixtures. Voor de laatste opstartproef is opzettelijk de lange `TMPDIR` onder de werkmap gebruikt. De finale binary start daarmee succesvol en voert de native HTTP- en WebKit-zoekopdrachten uit. Dit controleert de korte `/tmp`-terugval voor Unix-sockets wanneer zowel het runtimepad als `TMPDIR` te lang zijn.

De volledige ontwikkelharness staat in `work/search-native-test/run.py`, met het resultaat in `results.json`. De 165 eerder geslaagde gevallen zijn behouden en aangevuld met de laatste bridge-/opstartproef; het resultaat vermeldt deze hervatting. De compacte distributie-uitvoer staat in `validation/search-results.json` en bevat geen tokens, persoonlijke instellingen, proces-ID's of bestandsinhoud.

De proeven controleren begrensd lokaal bestandszoeken en bestaande wire-objecten. Fysieke invoerhardware, alle bestandssystemen en alle mogelijke Unicode-versies zijn hiermee niet uitputtend getest.
