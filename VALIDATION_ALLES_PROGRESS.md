# Alles — stabiele indexeringsvoortgang

> **Historisch (D-G72, 14 november 2026):** gemeten met de HTML-interface in WebKitGTK van toen. Die is verwijderd: Verkenner, Instellingen, Taakbeheer en de andere apps zijn nu native GTK 4-programma's, en Win11 voor GNOME bevat geen WebKit en geen GTK 3-host meer. De Rust-diensten die dit rapport controleert, bestaan nog. De laatste commit met WebKit is `6ebf169f`; zie [native/APPS.md](native/APPS.md).

De voortgangsmeldingen vervingen voorheen bij iedere indexupdate het hele eerste-indexeringspaneel. Daardoor begon de oorspronkelijke `alles-in`-animatie steeds opnieuw. Een tijdelijk ontbrekend pad verwijderde daarnaast een tekstregel; de statusring begon zijn animatie opnieuw doordat ook die werd vervangen.

`vendor/ui/js/apps/alles/main.js` houdt nu het bestaande eerste-indexerings- en welkomstpaneel vast. Alleen tekst, percentage, voortgangsbalk en tooltip veranderen. De statusindicator blijft behouden zolang zijn betekenis en voortgangsmodus gelijk blijven. Een tijdelijk leeg pad behoudt een onzichtbare regel, zodat de hoogte gelijk blijft. De bestaande CSS, pictogrammen, zoekacties en indexer zijn niet aangepast.

## Gemeten

Een geïsoleerde Chromium 156-browser laadde de echte UI-modules en CSS via een lokale testserver. Alleen de index-antwoorden en `index.status`-eventstream waren nagebootst. De echte indexer, persoonlijke indexbestanden en lopende gebruikersindexering zijn niet benaderd.

De test vergeleek het originele bestand uit de UI-baseline met de wijziging in dezelfde viewport. Acht oorspronkelijke rechthoeken — paneel, illustratie, titel, uitleg, voortgangsbalk, teller, pad en infoblok — kwamen in de statische toestand met een pad exact overeen.

Tijdens 28 voortgangsupdates met lege, korte en lange paden en de percentages onbekend/0/12/37/100:

| Meting | Origineel | Gewijzigd |
|---|---:|---:|
| Paneelvervangingen | 28 | 0 |
| Afzonderlijke entreeanimaties | 29 | 1 |
| Grootste verticale verschuiving | 8 px | 0 px |
| Grootste hoogteverandering | 22 px | 0 px |

Elf controlegroepen slagen. De oorspronkelijke indeterminate statusring behoudt hetzelfde DOM- en animatieobject terwijl zijn tijdlijn doorloopt. Teller, percentages en ARIA-waarden volgen de laatste melding. Welkom blijft bij vijftien statusupdates hetzelfde paneel, met een actuele objecttelling. Verversen met bestaande indexbestanden toont geen nieuw eerste-indexeringsscherm. Nieuwe lege zoekvragen krijgen actuele tekst en een echte overgang naar een nieuwe eerste indexering bouwt eenmaal een nieuw paneel.

De broncode doorstaat ook de JavaScript-syntaxcontrole en een onafhankelijke bronreview. De huidige hash en meetresultaten staan in `validation/alles-progress-results.json`. De testbron staat buiten het pakket in `work/alles-progress-test/`.

De browsermeting bewijst deze DOM- en CSS-werking voor de geteste viewport; de gewijzigde UI is hiermee niet opnieuw in alle GNOME/WebKit-schermcombinaties gemeten. De gereserveerde lege padregel is een bewuste geometriewijziging wanneer de indexer geen huidig pad meldt. Een aanwezig pad behoudt de oorspronkelijke statische indeling.
