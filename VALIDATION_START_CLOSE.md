# Start sluiten: sessiecrash verholpen in 0.9.3

Gecontroleerd op 19 november 2026, met GNOME/Mutter 50.1 in een eigen Wayland-testsessie.

## Oorzaak en reproductie

Een melding vanaf Ubuntu 26.04 met WinPoint 0.9.2 liet bij het sluiten van Start een afgebroken
GNOME-sessie zien: `meta_window_get_workspaces: code should not be reached`, gevolgd door SIGABRT.

Mutter geeft `unmanaging` door voordat het de werkruimte van het venster verwijdert. Vervolgens geeft
het `workspace-changed` door; `unmanaged` volgt pas later. WinPoint luisterde tot die laatste melding
naar indelingswijzigingen en kon het sluitende Startvenster nog verplaatsen. Wanneer een nieuwe
positie nodig was, bereikte Mutter zijn werkruimtebeperkingen met een venster zonder werkruimte.

De crash is afzonderlijk gereproduceerd op de ongewijzigde uitgave 0.9.2. De nieuwe regressie
`tests/gnome/scenarios/start-close-lifecycle.py` borgt die voorwaarde: een callback bij `unmanaging` wisselt uitsluitend
de uitlijningsprovider van de private koppeling. Daarmee staat bij de laatste indelingsmelding een
nieuwe positie klaar. Dit breekt de private compositor af op precies dezelfde Mutter-regel 6043.
Er wordt geen persoonlijke instelling gewijzigd.

## Reparatie en controles

- De koppeling merkt een venster vanaf `unmanaging` aan als sluitend. Latere wijzigings- en
  zichtbaarheidsmeldingen starten dan geen nieuwe plaatsing.
- De plaatsingsfunctie controleert ook of het venster nog wordt gevolgd en een werkruimte heeft.
- Dezelfde gerichte proef slaagt na de reparatie: vier controles, zonder compositoruitval.
- De uitgebreidere proef sluit en heropent Start 18 keer: met de echte Startknop, een echte
  Escape-toets en klikken buiten het menu, direct tijdens de openingsanimatie en na het uitspelen.
  Elke sluiting krijgt de hierboven beschreven indelingswijziging. Alle 74 controles slagen;
  het native proces en de compositor blijven bestaan en de Startachtergrond wordt telkens opgeruimd.
- De verbindingen- en procescontroles van de afgeschermde proef slagen. Er worden geen diensten,
  instellingen of invoerapparaten van de draaiende gebruikerssessie gebruikt.

De wijziging zit in de GNOME-extensie. De GTK- en Rust-functionaliteit blijft dezelfde; de
releaseprogramma's krijgen het nieuwe productversienummer. De gemelde mini-pc is hier niet
rechtstreeks beschikbaar; het oorspronkelijke apparaat moet na bijwerken nog worden gecontroleerd.
De updatepagina vraagt opnieuw aanmelden om de gewijzigde extensie te laden.
