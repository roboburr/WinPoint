# GNOME-sneltoetsen — 0.6.2

> **Historisch (D-G72, 14 november 2026):** gemeten met de HTML-interface in WebKitGTK van toen. Die is verwijderd: Verkenner, Instellingen, Taakbeheer en de andere apps zijn nu native GTK 4-programma's, en Win11 voor GNOME bevat geen WebKit en geen GTK 3-host meer. De Rust-diensten die dit rapport controleert, bestaan nog. De laatste commit met WebKit heeft de tag `last-webkit`; zie [native/APPS.md](native/APPS.md).

Getest op 1 november 2026 in een afzonderlijke D-Bus-sessie met GNOME Shell 50.1/Wayland, een virtueel scherm van 1280 × 800 en private HOME/XDG-mappen. De actieve gebruikerssessie en haar sneltoetsen zijn daarbij niet veranderd. De resultaten en bronhashes staan in `validation/shortcuts-0.6.2.json`; de ontwikkelharness en logboeken staan buiten het pakket in `work/shortcuts-test-0.6.2/`.

## Wijziging

De Windows/Super-toets wordt na het opstarten van GNOME aan dezelfde nieuwe Start-route gekoppeld als de taakbalk. De oorspronkelijke GNOME-handler wordt tijdelijk geblokkeerd en bij uitschakelen precies eenmaal vrijgegeven. Combinaties met Super blijven gewone combinaties; de instelling voor plaktoetsen blijft gerespecteerd. Staat het GNOME-overzicht nog open, dan wacht Start tot dat overzicht zijn toetsenbordfocus heeft vrijgegeven. De extensie bindt deze functies uitsluitend in de ontgrendelde gebruikerssessie.

Ctrl+Alt+Delete opent het zelfstandige Rust-programma voor Taakbeheer via zijn eigen desktop-identiteit. GNOME reserveert deze combinatie normaal voor afmelden in zijn mediasneltoetsendienst. Alleen die combinatie wordt tijdens het gebruik uit de bestaande afmeldsneltoetsen verwijderd. Andere afmeldsneltoetsen blijven behouden. Een nieuwe eigen GSettings-schema levert de vaste Taakbeheercombinatie aan Mutter; de afmeldinstelling wordt pas overgenomen als die registratie is gelukt.

De oorspronkelijke afmeldinstelling wordt als herstelrecord bewaard vóór de wijziging en bij uitschakelen teruggezet als de actuele waarde nog de eigen tijdelijke waarde is. Een oorspronkelijke standaardwaarde wordt opnieuw een standaardwaarde, zonder expliciete gebruikersoverride. Een gewijzigd gebruikersvoorkeur krijgt voorrang. Na een onderbroken GNOME-sessie wordt een bestaand geldig herstelrecord hergebruikt. De deïnstallatie kan hetzelfde herstel ook uitvoeren als de GNOME-extensiedienst niet beschikbaar is.

De installatie levert zeven passende desktop-identiteiten: Panel, Start, Explorer, Settings, TaskManager, Search en Disks. De laatste vier zijn verborgen in het GNOME-appoverzicht om dubbele vermeldingen naast de oorspronkelijke Win11-appcatalogus te voorkomen; GNOME kan ze wel volgen en activeren.

## Controles

Alle 24 GNOME-integratiegroepen en drie afzonderlijke deïnstallatiecontroles slagen:

- Echte virtuele toetsenbordinvoer controleert één en twee losse Super-aanslagen, Super vanuit het overzicht, plaktoetsen en een combinatie met Super. De testextensie telt de nieuwe Start-callback; zij start in deze specifieke fixture geen echte Start-host. De echte resident Start-host heeft afzonderlijke integratiecontroles.
- Echte Ctrl+Alt+Delete-invoer opent het native Taakbeheer van de uiteindelijke 0.6.2-binary. Een volgende aanslag hergebruikt hetzelfde proces en venster.
- De echte `gsd-media-keys` draait tijdens de test. Na uitschakelen is aangetoond dat de herstelde afmeldcombinatie weer een extern accelerator-signaal van die GNOME-dienst oplevert. Na opnieuw inschakelen bereikt dezelfde combinatie weer Taakbeheer.
- Gewijzigde afmeldcombinaties, behoud van andere combinaties, herstel van de oorspronkelijke standaard, herstel na een gesimuleerde onderbreking, blokkeren bij vergrendelen, opnieuw koppelen na ontgrendelen en mislukte registratie worden gecontroleerd.
- Deïnstallatie zonder actieve GNOME-dienst herstelt respectievelijk de standaard, een exacte aangepaste waarde en behoudt een latere afwijkende gebruikerswaarde. De eigen schema's en bestanden worden daarna verwijderd.

Een vergelijking met de tijdelijke eigen waarde kan geen intentie herkennen wanneer iemand precies diezelfde waarde kiest zonder een wijzigingssignaal. De fixture meldt verwachte waarschuwingen voor ontbrekende sessie-/systemd-diensten en bij de doelbewust mislukte registratie; deze test claimt geen volledig waarschuwingsvrij GNOME-logboek.
