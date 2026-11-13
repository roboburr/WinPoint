# Oorspronkelijk bureaublad in een eigen Rust-host — 0.7.0

> **Historisch (D-G70, 13 november 2026):** dit rapport gaat over de verwijderde WebKit-bureaubladhost. De native GTK 4-shell `win11-shell` vervangt die; zie [notes/claude/gtk-native.md](notes/claude/gtk-native.md).

Getest op 1 november 2026 met GNOME Shell 50.1/Wayland, twee virtuele schermen van 1280 × 800 en 1024 × 768 naast elkaar, schaal 1, en een afzonderlijke D-Bus-sessie. HOME, configuratie, bestanden, prullenbak, cache, runtime en GNOME-voorkeuren stonden in private testmappen. De actieve gebruikerssessie, haar DING-proces, apps en indexeerwerk zijn daarbij niet aangepast.

De twee uitvoeringen slagen volledig: 52 gecombineerde controles, waaronder 19 achtergrondcontroles, en 35 gerichte desktopcontroles met aanvullend echt dubbelklikken op een map. Samen vormen zij 54 unieke controles. Resultaten en hashes staan in `validation/desktop-0.7.0.json`; de volledige kleine ontwikkelharness en logboeken staan buiten het pakket in `work/desktop-bridge-test-0.7.0/`. Achtergrondinstellingen hebben daarnaast een eigen verslag, `VALIDATION_WALLPAPER.md`.

## Uitvoering en overname

`win11-desktop` is een vijfde, zelfstandige Rust-host met de exacte app-identiteit `io.github.win11shell.Desktop`. Hij laadt het bestaande `surface=desktop`, inclusief de oorspronkelijke pictogrammen, positionering, selectierechthoek, toetsenbediening en contextmenu's. De GNOME-koppeling behandelt alleen deze exacte identiteit als eigen desktop: venstertype DESKTOP, alle werkbladen, buiten taaklijsten en onder normale apps. Zij verplaatst of herparent geen compositoractor van andere vensters.

Eén desktopvenster vult de omvattende rechthoek van de aangesloten schermen. In de test is dat 2304 × 800. Geometrieveranderingen verplaatsen alleen de eigen desktop en nemen geen focus van andere apps over. Het native desktopvenster wordt niet als taakbalk-app getoond.

DING blijft draaien en houdt zijn oorspronkelijke ingeschakelde toestand en instellingen. Alleen zijn exacte desktopvensters met app-ID `com.rastersoft.ding` worden tijdelijk ongemapt. Een gewoon DING-instellingenvenster kwalificeert niet. Overname gebeurt pas als de oorspronkelijke desktop klaar is, de native host dit meldt en het venster een frame heeft getekend. Reeds bestaande DING-vensters en later aangemaakte vensters worden gevolgd. De koppeling gebruikt Mutters volledige vensterlijst, omdat DING zijn eigen vensters uit `global.get_window_actors()` filtert.

Bij sluiten, procesuitval, rendereruitval, uitschakelen of vergrendelen geeft de nieuwe desktop de oude desktop weer vrij. Het native hostproces eindigt wanneer zijn WebKit-renderer wegvalt. De desktop meldt zijn gereedheid ook periodiek, zodat een opnieuw gestarte koppeling die toestand opnieuw kan ontvangen. Snel uit- en inschakelen wacht op het beëindigen van de vorige eigen desktop voordat een nieuwe wordt gestart. Onafhankelijke apps blijven bestaan.

GNOME-hotcorners worden uitsluitend tijdens de overname tijdelijk buiten werking gesteld. Dit voorkomt dat een selectie naar de linkerbovenhoek het oude Activiteiten-overzicht opent. De koppeling bewaart en herstelt de oorspronkelijke barrièregrootte en drag-handler, ook bij een achtergrondwissel of nieuwe monitorconfiguratie. De GNOME-voorkeur voor hotcorners wordt niet gewijzigd.

## Gecontroleerd gedrag

- Echte virtuele muisinvoer door GNOME en GTK/WebKit toont de oorspronkelijke selectierechthoek tijdens slepen. De DOM registreert `isTrusted:true`; de gemeten tussenstand is 40 × 250 pixels.
- Slepen over de eerste kolom selecteert Deze pc, Prullenbak, een testmap en twee testbestanden. Loslaten behoudt de selectie; de band verdwijnt. Slepen naar de schermrand verandert de native venstergrootte niet.
- Escape, Ctrl+A, een klik op lege ruimte en Ctrl met een extra selectierechthoek werken met echte native toetsenbordfocus. Rechtsklikken opent het oorspronkelijke Win11-contextmenu.
- Echt dubbelklikken op de testmap opent de losse native Rust-Verkenner op die map, met compositorfocus voor het nieuwe appvenster.
- Normale appvensters staan boven de desktop en kunnen focus houden. Zelfs expliciet verhogen van de desktop plaatst hem niet boven Verkenner. Herpositioneren neemt geen appfocus over.
- Voor gereedheid blijven beide echte DING-vensters gemapt; daarna zijn beide ongemapt. Uitschakelen, sluiten en relaunch herstellen respectievelijk de juiste toestand zonder DING-voorkeuren te wijzigen.
- De test beëindigt doelbewust alleen de WebKit-renderer die onder het private native desktopproces draait. DING keert terug; de zelfstandige apps blijven leven. Een nieuwe desktop werkt daarna opnieuw.
- Snel uit- en inschakelen, vergrendelen en ontgrendelen leveren opnieuw een gereed bureaublad op. De uiteindelijke uitschakeling verwijdert alleen de eigen shelloppervlakken, herstelt DING en hotcorners en laat onafhankelijke apps staan.
- De oorspronkelijke Instellingen-interface past thema's, foto's, effen kleuren, alle zes afbeeldingspassingen, automatisch volgen en een eigen foto toe op zowel de echte desktop-DOM als de GNOME-achtergrondvoorkeuren. Zie het afzonderlijke achtergrondverslag voor de 19 controles en de afbakening van de testbestandskiezer.

## Grenzen

Beide virtuele schermen hebben dezelfde schaalfactor. Gemengde DPI, negatieve schermposities en lege ruimten tussen schermen zijn niet in deze test geverifieerd. De implementatie gebruikt één desktop over de gezamenlijke rechthoek; onafhankelijke pictogramrasters per monitor zijn nog niet geïmplementeerd. Dit verslag claimt geen volledig waarschuwingsvrij GNOME-logboek: een private sessie mist enkele normale sessiediensten en de opzettelijke rendererbeëindiging veroorzaakt verwachte meldingen.
