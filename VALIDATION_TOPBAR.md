# GNOME-bovenbalk bij de native taakbalk — 0.6.1

> **Historisch (D-G72, 14 november 2026):** gemeten met de HTML-interface in WebKitGTK van toen. Die is verwijderd: Verkenner, Instellingen, Taakbeheer en de andere apps zijn nu native GTK 4-programma's, en Win11 voor GNOME bevat geen WebKit en geen GTK 3-host meer. De Rust-diensten die dit rapport controleert, bestaan nog. De laatste commit met WebKit heeft de tag `last-webkit`; zie [native/APPS.md](native/APPS.md).

> **Historisch (D-G70, 13 november 2026):** dit rapport is gemeten met de verwijderde WebKit-taakbalk. De extensie verbergt de GNOME-bovenbalk nog steeds, nu naast de native GTK 4-shell `win11-shell`; zie [notes/claude/gtk-native.md](notes/claude/gtk-native.md).

De GNOME-extensie verborg voorheen alleen de eigen GJS-fallback wanneer het Rust-paneel actief werd. De oorspronkelijke GNOME-bovenbalk bleef daardoor zichtbaar. De aangepaste `extension.js` verbergt ook die bovenbalk zolang het native paneel de gebruikerssessie bedient. Het GNOME-overzicht toont de oorspronkelijke bovenbalk weer. Bij verdwijnen van het native paneel, uitschakelen van de extensie of vergrendelen wordt de eerder aangetroffen toestand hersteld. De bestandsinterfaces, pictogrammen en CSS zijn hiervoor niet gewijzigd.

GNOME Shell 50 houdt ook voor verborgen chrome een strut bij. Daarom bewaart de extensie de bestaande `panelBox`-tracking en schakelt alleen diens `affectsStruts` tijdelijk uit; anders zou een lege strook bovenaan overblijven. De oorspronkelijke tracking en fullscreen-regels blijven aanwezig. De gebruikte GNOME 50-API is gecontroleerd in de lokaal geïnstalleerde primaire bron `layout.js` uit `libshell-18.so`.

## Gemeten

In een afzonderlijke GNOME Shell 50.1 Wayland-sessie met eigen HOME, XDG-mappen, sessiebus en geïnstalleerde kopie van de extensie zijn zesentwintig controles geslaagd. De test gebruikte de echte Rust-taakbalk en een zelfstandig Rust-Verkenner-proces:

- Bovenbalk en bovenste gereserveerde strook verdwijnen op het bureaublad.
- GNOME-overzicht herstelt de bovenbalk; GNOME Snelle instellingen kan daar worden geopend.
- Verlaten van het overzicht en latere GNOME-zichtbaarheidsupdates behouden het bureaubladbeleid.
- Volledig scherm verbergt beide panelen; verlaten van volledig scherm herstelt de native taakbalk.
- Verdwijnen en opnieuw starten van het native paneel herstellen respectievelijk verbergen de GNOME-bovenbalk.
- Uitschakelen herstelt de bovenbalk en strut, terwijl de zelfstandige Verkenner blijft draaien.
- Een bij activering al verborgen oorspronkelijke bovenbalk blijft na uitschakelen verborgen.
- Overgang naar de GNOME-modus `unlock-dialog` geeft de bovenbalk terug; terugkeer naar de gebruikersmodus hervat het native beleid.
- De 0.6.1-taakbalk warmt Start onzichtbaar op. Na het definitieve `start-ready`-bericht bestaat vóór de eerste klik geen Start-venster of compositoractor. Uitschakelen en vergrendelen verwijderen de resident Start-instantie. Alle vier via de private sessiebus waargenomen Start-processen zijn aan het einde verdwenen; de zelfstandige Verkenner bleef tijdens uitschakelen draaien.

De laatste meting gebruikt de definitieve native focusroute (`start-dismiss` met revisiecontrole). De JSON bevat de gecontroleerde SHA256-hashes van alle vier geteste binaries en `panelBridge.js`.

De compacte resultaten staan in `validation/topbar-results.json`; de ontwikkeltest staat buiten het pakket in `work/topbar-test/run.py`. De lockcontrole test de GNOME-sessiemodus en extensiecyclus, niet een authenticatiegesprek. De geïsoleerde compositor meldde enkele Mutter-stack-/pingwaarschuwingen tijdens vensters sluiten; de zesentwintig assertions slaagden en de test eindigde met code nul. Dit is geen bewijs voor alle combinaties van beeldschermen of andere GNOME-extensies.

GNOME-menu's waarvan de oorspronkelijke knop verborgen is, blijven via de herstelde bovenbalk in het overzicht beschikbaar. De bestaande HTML-flyouts van de Rust-taakbalk zijn niet gewijzigd.

Een al in GNOME geladen JavaScript-extensie wordt niet betrouwbaar opnieuw geïmporteerd door alleen de bestanden te vervangen. Na een bijgewerkte installatie is af- en opnieuw aanmelden nodig om deze versie in de bestaande sessie te laden.
