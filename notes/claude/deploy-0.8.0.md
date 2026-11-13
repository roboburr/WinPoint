# Win11 voor GNOME 0.8.0 installeren en testen (voor Robin)

Alles hieronder doe je zelf; ik heb niets op je live sessie geïnstalleerd. G is
`/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome`. De release-build staat in `G/target/release`, het
pakket voor de NucBox in `G/dist/win11-gnome-0.8.0-linux-x86_64.tar.gz`.

## Deze pc

### 1. Back-up (één keer, vóór de installatie)

```bash
cd ~ && tar -czf ~/win11-gnome-backup-0.7.0.tar.gz --ignore-failed-read .local/bin/win11-* .local/share/win11-gnome .local/share/gnome-shell/extensions/win11-gnome@win11shell.local .local/share/applications/*win11shell* .config/win11-gnome
```

### 2. Installeren

```bash
cd /home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome && WIN11_BUILD_DIR=$PWD/target/release ./scripts/install.sh
```

De installer houdt je taalkeuze uit `~/.config/win11-gnome/settings.json`. Daarna **uitloggen en weer inloggen**
(de extensie is veranderd, dus "Win11 Shell opnieuw starten" is niet genoeg). Er staat nog een herstart van Ubuntu
open (`/run/reboot-required` sinds 5 november); opnieuw opstarten in plaats van uitloggen mag ook.

### 3. Optioneel: de vier systeembestanden (R2, één wachtwoord)

Nodig voor de keuze "Nieuwe Ubuntu-versies" zonder de `/bin/sh`-melding en voor "Venster weer openen" tijdens een
Ubuntu-upgrade. Eerst kijken wat het doet, dan installeren:

```bash
cd /home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome && python3 scripts/install-system.py --dry-run && python3 scripts/install-system.py
```

Ongedaan maken: `python3 scripts/install-system.py --remove`.

### 4. Wat je kunt testen

* Instellingen › Tijd en taal › Taal en regio: wissel tussen Nederlands en English; taakbalk, Start, Verkenner, Taakbeheer en
  Instellingen volgen meteen (de Ctrl+Alt+Tab-groep na opnieuw inloggen).
* Instellingen › Update en herstel: "Controleren op updates", "Alles downloaden en installeren" (je eerste echte
  update via Win11, R3), Updategeschiedenis, Geavanceerde opties, "Nieuwe Ubuntu-versies".
* De kaart "Ubuntu 26.10": verschijnt pas als Ubuntu 26.10 aanbiedt; met deze build zegt hij "onderweg" (GNOME 51 is
  nog niet getest). Een echte upgrade nooit eerst op deze pc (R5 is een VM).
* "Win11 Shell opnieuw starten" (Update en herstel): alle Win11-vensters komen terug; Verkenner wacht als er een
  kopie loopt.
* Taakbeheer › Instellingen › Altijd op voorgrond.

### 5. Terugzetten als iets niet goed is

```bash
cd /home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome && ./scripts/uninstall.sh && cd ~ && tar -xzf ~/win11-gnome-backup-0.7.0.tar.gz
```

Daarna uitloggen en weer inloggen.

## NucBox

Eerst kijken wat de oude Hyprland-versie heeft achtergelaten (leest alleen, verandert niets):

```bash
tools/h-remnants.sh
```

Pakket overzetten en installeren (op de NucBox, in de map waar je het pakket neerzet):

```bash
tar -xzf win11-gnome-0.8.0-linux-x86_64.tar.gz && cd win11-gnome && ./scripts/install.sh --lang nl
```

Daarna uitloggen en inloggen, en `gnome-extensions enable win11-gnome@win11shell.local` als de taakbalk niet
verschijnt.
