# Doel voor openbare uitgave

Het project moet na afronding als open source beschikbaar komen, met broncode en een downloadbaar installatiepakket. Er is vanuit deze werkmap nog geen openbare repository of release gepubliceerd.

Voor de eerste openbare uitgave:

- Werkende native shell (`win11-shell`: taakbalk, flyouts, Start, bureaublad en tray), losse apps en vensterknoppen in de ondersteunde GNOME-sessie aantonen.
- `win11-shell` en zijn backend samen onder 1 GiB houden; gestarte apps in eigen groepen; de Alles-indexer uitsluitend tijdens gebruik van Alles laten draaien.
- Installeren, bijwerken, verwijderen en terugvallen op de bestaande desktop met behoud van persoonlijke gegevens controleren.
- Ondersteunde Ubuntu-, GNOME-, Wayland-, schaal- en schermconfiguraties concreet vastleggen; ontbrekende ondersteuning en tijdelijke Python-diensten zichtbaar documenteren.
- Broncode, exacte bouwafhankelijkheden, reproduceerbare bouwinstructies, versienummers, checksums en testbewijs opnemen.
- De herkomst en licentiebestanden van alle meegeleverde code, afbeeldingen, pictogrammen en lettertypen vastleggen; projectnaam en eigen ronde logo consequent gebruiken.
- Een gebruikershandleiding, wijzigingslog en eenvoudige procedure voor foutmeldingen toevoegen.

De huidige ontwikkelbundel blijft een alpha. De bekende onverklaarde vroege paneeluitval en niet integraal gecontroleerde scherm-/appcombinaties moeten vóór een stabiele publieke release worden onderzocht. De bestaande validatierapporten maken onderscheid tussen werkelijk uitgevoerde controles en nog openstaand werk.

De standaardconfiguratie staat in `host/src/settings-defaults.json` en bevat alle ondersteunde shellinstellingen. De GTK-familie is standaard `system`. Persoonlijke geschiedenis, accounts, computergebonden paden en de indexcache vormen geen deel van het openbare profiel. De taalkeuze in Win11Shell bepaalt zowel de instellingeninterface als de bureaubladlabels.
