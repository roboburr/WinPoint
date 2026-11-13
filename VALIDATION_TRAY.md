# GNOME-traycontrole 0.6.2

Ubuntu AppIndicator publiceert zijn tray-items als `busName@/objectPath`. De bestaande SNI-lezer herkende alleen `busName/objectPath` en registreerde de huidige items daarom niet. Beide vormen gebruiken nu dezelfde strikte controle van de busnaam en het objectpad. Het bestaande HTML-systeemvak, de overloop, pictogrammen en menu-interface blijven ongewijzigd.

Twaalf controles op een eigen echte D-Bus-sessie slagen: GNOME/KDE-registraties, ongeldige invoer, initiële lijst en pictogram, wijzigingsgebeurtenis, activeren/middenklik/contextmenu/scrollen, passief/aandacht, later registreren, dubbele registratie, afmelden, verbroken verbinding en opruimen. Deze test gebruikt nagebootste SNI-apps; hij roept geen persoonlijke tray-apps aan.

Een afzonderlijke alleen-lezencontrole in de actieve Ubuntu-sessie vond acht tray-items, waarvan vijf actief of aandacht vragend. De bestaande taakbalkdienst gaf nul items terug; de herstelde lezer gaf acht terug. Daarbij zijn geen persoonlijke apps geactiveerd of gesloten. Passieve items blijven volgens het bestaande gedrag verborgen. De lijst met persoonlijke appnamen staat alleen in het lokale werkverslag en wordt niet meegeleverd.

De traydienst gebruikt vooralsnog de bestaande Python/Gio-compatibiliteitslaag. Deze correctie is geen volledige Rust-herschrijving van StatusNotifierItem of dbusmenu. Legacy XEmbed-tray-items worden niet ondersteund.
