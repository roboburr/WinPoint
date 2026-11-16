<p align="center">
  <img src="logo-circle.png" width="72" alt="">
</p>

<h1 align="center">WinPoint</h1>

<p align="center">
  <strong>Een bureaublad in de stijl van Windows 11 voor GNOME, in Rust en GTK 4</strong>
</p>

<p align="center">
  <a href="https://github.com/roboburr/WinPoint/releases/latest"><img src="https://img.shields.io/github/v/release/roboburr/WinPoint" alt="Nieuwste uitgave"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="Licentie: MIT"></a>
  <img src="https://img.shields.io/badge/GNOME-50-4A86CF?logo=gnome&logoColor=white" alt="GNOME 50">
  <img src="https://img.shields.io/badge/Ubuntu-26.04-E95420?logo=ubuntu&logoColor=white" alt="Ubuntu 26.04"><br><br>
  <a href="#schermafbeeldingen">Schermafbeeldingen</a> · <a href="#mogelijkheden">Mogelijkheden</a> · <a href="#installeren">Installeren</a> · <a href="#bouwen-vanuit-de-broncode">Bouwen</a> · <a href="README.md">English</a>
</p>

WinPoint geeft GNOME 50 een taakbalk, Startmenu, snelle instellingen, meldingencentrum en bureaublad in de stijl van Windows 11, met een eigen Verkenner, Instellingen en Taakbeheer. Een GNOME Shell-extensie koppelt het aan GNOME; de rest is native GTK 4 en Rust.

<p align="center">
  <img src="docs/screenshots/start.png" width="860" alt="WinPoint op GNOME 50: Start geopend boven de taakbalk, over de Lotus-achtergrond">
</p>

## Schermafbeeldingen

<table>
  <tr>
    <td align="center" width="50%"><img src="docs/screenshots/desktop.png" width="100%" alt="Het bureaublad met pictogrammen en de taakbalk"><br><sub><b>Bureaublad</b> met pictogrammen en de taakbalk</sub></td>
    <td align="center" width="50%"><img src="docs/screenshots/winx.png" width="100%" alt="Het Win+X-menu boven de Startknop"><br><sub><b>Win+X-menu</b>: rechtsklik op Start</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/screenshots/quick-settings.png" width="100%" alt="De snelle instellingen"><br><sub><b>Snelle instellingen</b></sub></td>
    <td align="center"><img src="docs/screenshots/notifications.png" width="100%" alt="Meldingencentrum en agenda"><br><sub><b>Meldingen en agenda</b></sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/screenshots/task-view.png" width="100%" alt="Taakweergave met drie vensters"><br><sub><b>Taakweergave</b>: Alt+Tab</sub></td>
    <td align="center"><img src="docs/screenshots/explorer.png" width="100%" alt="Verkenner in de persoonlijke map"><br><sub><b>Verkenner</b> met tabbladen</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/screenshots/settings.png" width="100%" alt="Instellingen, pagina Persoonlijke instellingen"><br><sub><b>Instellingen</b>: Persoonlijke instellingen</sub></td>
    <td align="center"><img src="docs/screenshots/task-manager.png" width="100%" alt="Taakbeheer, pagina Prestaties"><br><sub><b>Taakbeheer</b>: Prestaties</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/screenshots/calendar.png" width="100%" alt="De Agenda-app, maandweergave"><br><sub><b>Agenda</b>: maandweergave</sub></td>
    <td align="center"><img src="docs/screenshots/light.png" width="100%" alt="Start in het lichte thema"><br><sub><b>Licht thema</b></sub></td>
  </tr>
</table>

## Mogelijkheden

- **Taakbalk**: vastgemaakte en geopende apps met groepering en jump lists, pictogrammen in het systeemvak, klok, gecentreerd of links uitgelijnd, desgewenst automatisch verbergen.
- **Start**: vastgemaakte apps, Alle apps, aanbevolen bestanden, en zoeken in apps, instellingen, bestanden en op internet.
- **Snelle instellingen**: Wi-Fi, Bluetooth, vliegtuigmodus, nachtlampje, donkere modus, niet storen, helderheid, volume en mediabediening.
- **Meldingencentrum** met pop-upmeldingen en een maandagenda.
- **Win+X-menu** en **taakweergave**.
- **Bureaublad** met Deze pc, Prullenbak, je eigen bestanden, contextmenu's, hernoemen en slepen.
- **Matglas** met de vervaging van GNOME Shell zelf, en Windows 11-vensterknoppen voor je andere GTK-apps.
- **Apps**: Verkenner (ook het venster voor openen en opslaan in apps die de desktopportal gebruiken), Instellingen, Taakbeheer, Alles (direct zoeken in bestanden, naar het voorbeeld van [Everything](https://www.voidtools.com/) van voidtools), Schijven, Standaard-apps, App-installatieprogramma (AppImage, DEB, Flatpak, Snap) en Agenda.
- **Nederlands en Engels**, op elk moment om te schakelen, los van de taal van GNOME.
- **Sneltoetsen**: de Windows-toets opent Start, Alt+Tab de taakweergave en Ctrl+Alt+Delete Taakbeheer.

## Vereisten

- Ubuntu 26.04 met GNOME Shell 50 op Wayland, het enige geteste systeem. De uitgaven zijn gebouwd voor x86_64.
- Python 3 voor de installer; die controleert de rest zelf (PyGObject inbegrepen) en noemt de Ubuntu-pakketten die ontbreken.
- Optioneel: FFmpeg voor videominiaturen; PackageKit, Flatpak of snapd voor het App-installatieprogramma.

## Installeren

Download `winpoint-<versie>-linux-x86_64.tar.gz` van de [nieuwste uitgave](https://github.com/roboburr/WinPoint/releases/latest) en voer daarna uit:

```bash
tar -xzf winpoint-*-linux-x86_64.tar.gz
cd winpoint-*/
./scripts/install.sh --lang nl
```

De installer zet WinPoint in je thuismap (`~/.local`) en voegt **WinPoint** toe als keuze bij het inloggen; `pkexec` vraagt daarvoor één keer om je wachtwoord. Meld je af, klik op je naam, kies **WinPoint** met het tandwiel rechtsonder en meld je aan. Je GNOME- en Ubuntu-sessies blijven zoals ze waren.

`--lang nl` of `--lang en` kiest de taal bij een eerste installatie; zonder die optie raadt de installer de taal uit je systeeminstellingen. Later wijzig je de taal in **Instellingen › Tijd en taal › Taal en regio**.

Wil je WinPoint liever in je gewone GNOME-sessie gebruiken, installeer dan met `--no-session` (geen wachtwoord nodig), meld je één keer af en weer aan en voer `gnome-extensions enable win11-gnome@win11shell.local` uit. Ubuntu Dock blijft daar aan; zet het uit met `gnome-extensions disable ubuntu-dock@ubuntu.com`.

## Bijwerken

WinPoint kijkt één keer per dag op GitHub en meldt het als er een nieuwe uitgave is. Klik in **Instellingen › Update en herstel** op **Downloaden en installeren**: WinPoint downloadt de uitgave, controleert hem met het controlegetal van GitHub en installeert hem; je instellingen blijven bewaard. Start daarna WinPoint opnieuw of meld je af en weer aan, zoals de pagina aangeeft. Het automatisch controleren kun je uitzetten onder **Geavanceerde opties**; een nieuwere uitgave zelf installeren, zoals hierboven, kan ook.

## Verwijderen

Voer deze opdrachten uit in de uitgepakte map van de uitgave die je hebt geïnstalleerd (download die uitgave opnieuw als je de map niet meer hebt), of in je git-kloon:

```bash
./scripts/uninstall.sh
python3 scripts/install-session.py --remove
```

De eerste opdracht zet de extensie uit, verwijdert de bestanden van WinPoint en zet het eigen uiterlijk van GNOME terug. De tweede haalt de keuze WinPoint bij het inloggen weg (`pkexec` vraagt om je wachtwoord). Je instellingen (`~/.config/win11-gnome`) en je gebeurtenissen in Agenda (`~/.local/share/win11-gnome-calendar`) blijven staan; wil je alles weg hebben, verwijder dan ook die mappen en `~/.cache/win11-gnome`.

Na een installatie met `--no-session` zet je eerst **Instellingen › Persoonlijke instellingen › Kleuren › Windows 11-vensterknoppen in apps** uit, en kies je daarna je thema en achtergrond opnieuw in de Instellingen van GNOME.

## Bouwen vanuit de broncode

Je hebt Rust 1.92 of nieuwer nodig (het `cargo`-pakket van Ubuntu is nieuw genoeg) en de ontwikkelbestanden van GTK 4:

```bash
sudo apt install git build-essential pkg-config cargo libglib2.0-dev libgdk-pixbuf-2.0-dev libgtk-4-dev libarchive13t64
git clone https://github.com/roboburr/WinPoint.git
cd WinPoint
cargo fetch --locked
cargo fetch --locked --manifest-path native/Cargo.toml
./scripts/build.sh
./scripts/install.sh --lang nl
```

`build.sh` bouwt offline; daarom halen de twee `cargo fetch`-regels eerst de crates op. De programma's komen in `target/release`, waar de installer ze vindt. Heb je `CARGO_TARGET_DIR` gezet, geef de installer die map dan mee: `WIN11_BUILD_DIR="$CARGO_TARGET_DIR/release" ./scripts/install.sh`. `WIN11_BUILD_JOBS` bepaalt het aantal gelijktijdige bouwtaken (standaard 2).

## Zo werkt het

Een GNOME Shell-extensie start `win11-shell`, één GTK 4-proces voor de taakbalk, Start, de flyouts en het bureaublad, en regelt de plaats van de vensters, het matglas en de sneltoetsen. Een Rust-backend geeft de shell via een lokale JSON-API toegang tot apps, instellingen, het systeemvak, meldingen, geluid, netwerk, energie en bestanden; elke app is een eigen GTK 4-proces. Zet je de extensie uit, dan heb je weer gewoon GNOME.

Instellingen staan in `~/.config/win11-gnome/settings.json` (standaardwaarden in [host/src/settings-defaults.json](host/src/settings-defaults.json)). Wie wil bijdragen, begint met [native/ARCH.md](native/ARCH.md), [native/APPS.md](native/APPS.md) en [tests/gnome/README.md](tests/gnome/README.md) (in het Engels) en draait voor een pull request `python3 -B tools/i18n/check.py` (elke tekst in de interface is een Nederlands/Engels paar) en `tests/rust-sandbox/run-tests.sh`.

## Bekende beperkingen

- De taakweergave toont app-pictogrammen, geen live miniaturen van de vensters.
- De sneltoets Win+X werkt nog niet; open dat menu met een rechtsklik op Start.
- Beeldscherminstellingen zijn in de Instellingen van WinPoint alleen te bekijken; wijzig ze in de Instellingen van GNOME.
- Met meer dan één beeldscherm staan de taakbalk, Start en de andere uitklapvensters alleen op het hoofdscherm, en loopt het bureaublad (achtergrond en pictogrammen) als één vlak over alle schermen. Een taakbalk op elk scherm is in de maak.
- Het App-installatieprogramma is nog niet getest met echte installaties op een systeem.
- Wachtwoordvensters, berichten van PackageKit en NetworkManager en de onderdelen van GNOME zelf volgen de taal van het systeem, niet die van WinPoint.

## Licentie

WinPoint valt onder de [MIT-licentie](LICENSE); details staan in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), de volledige teksten in [LICENSES/](LICENSES/). Uitzonderingen:

- het **Win11-pictogramthema** van [yeyushengfan258](https://github.com/yeyushengfan258/Win11-icon-theme) (`vendor/icons/`, `vendor/icon-source/` en `vendor/ui/assets/icons/settings.svg`), dat onder **GPL-3.0-only** valt (wijzigingen in [vendor/ICON_PROVENANCE.md](vendor/ICON_PROVENANCE.md));
- de **Lotus-achtergronden**, met AI gemaakt en vrijgegeven onder [CC0-1.0](LICENSES/CC0-1.0.txt);
- de Rust-crates, die hun eigen licentie houden ([LICENSES/RUST-CRATES.md](LICENSES/RUST-CRATES.md)), en twee Rust-bestanden die uit CPython zijn vertaald (PSF-2.0).

WinPoint is een onafhankelijk project, niet verbonden aan of goedgekeurd door Microsoft. Windows en Windows 11 zijn handelsmerken van Microsoft Corporation. Er zit geen code, materiaal of lettertype van Microsoft in. Alles volgt het uiterlijk en de zoektaal van Everything van voidtools; er zit geen code van voidtools in en het is niet verbonden aan voidtools.
