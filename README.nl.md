English: [README.md](README.md) is the main README; this is the Dutch version.

# Win11 voor GNOME, ontwikkelversie 0.8.0

Win11 voor GNOME geeft GNOME een Windows 11-shell. De taakbalk, de flyouts, Start en het bureaublad zijn samen één native GTK 4-proces in Rust: `win11-shell` (GTK 4.22, gtk4-rs), zonder WebKit. Verkenner, Instellingen, Taakbeheer, Alles, Schijfbeheer, Standaardapps en de Installer blijven HTML, CSS en JavaScript in WebKitGTK (GTK 3), met de vertrouwde Win11Shell-interface, pictogrammen en vensterknoppen. Een GNOME Shell-extensie koppelt de taakbalk, het bureaublad, sneltoetsen en echte vensteracties aan GNOME. Het huidige ontwikkeldoel is Ubuntu met GNOME Shell 50 en Wayland.

De standaardruntime draait volledig in Rust, inclusief de oorspronkelijke publieke diensten, HTTP/eventstream, Alles-indexer en afbeeldinghelpers. De shell haalt apps, vastgemaakte items, instellingen, het systeemvak, meldingen, geluid, netwerk, energie en bestanden bij `win11-panel --http-only`, de Rust-backend zonder venster of WebKit. De oorspronkelijke Python-bronnen onder `vendor/host` blijven een referentie voor vergelijkende tests. Python wordt nog gebruikt voor installatie-, pakket- en ontwikkelhulpmiddelen; de geopende shell en apps starten daarvoor geen Python-backend. Een expliciet gekozen referentiebackend blijft beschikbaar voor ontwikkeltests.

Dit is werk in ontwikkeling. Er is vanuit deze werkmap geen openbare alpha gepubliceerd. De actuele controles, hun omgeving en grenzen staan bovenaan [VALIDATION.md](VALIDATION.md). Die controles bewijzen de genoemde scenario's; volledige één-op-één-pariteit op iedere machine is nog geen aangetoonde eigenschap. De eerdere HTML-versie van taakbalk, Start en bureaublad in WebKit is verwijderd; de rapporten daarover zijn historisch.

## Zelfstandige programma's

| Programma | Functie |
|---|---|
| `win11-panel` | Start `win11-shell`; met `--http-only` levert het de diensten van de shell (apps, instellingen, systeemvak, meldingen, geluid, netwerk, energie, bestanden), zonder venster |
| `win11-shell` | Native GTK 4-shell: taakbalk, flyouts (snelle instellingen, meldingen en agenda, jump lists, Win+X, taakweergave), Start en zoeken, het bureaublad met iconen en bestandsacties |
| `win11-explorer` | Verkenner met tabs, taboverdracht en native bestandsbediening |
| `win11-app --app settings` | Instellingen |
| `win11-app --app taskmgr` | Taakbeheer |
| `win11-app --app alles` | Alles, met eigen indexproces zolang deze app geopend is |
| `win11-app --app disks` | Schijfbeheer |
| `win11-default-apps` | Standaardapps per bestandstype of extensie |
| `win11-calendar` | Agenda in Windows 11-stijl (GTK 4), geopend op een dag vanuit de agenda van de taakbalk |
| `win11-installer` | Installatie- en verwijdervoorstellen met expliciete bevestiging; nieuwe interface wordt nog geïntegreerd en gecontroleerd |
| `win11-file-portal` | Op verzoek geactiveerde FileChooser-backend voor desktopportals |

Index-, afbeelding- en softwaremetadatahelpers zijn interne programma's die alleen op verzoek starten. Losse apps hoeven geen zichtbaar paneel te hebben. Taboverdracht tussen bestaande Verkenner-vensters blijft beschikbaar; voor loslaten op de desktop en echte schermpositie is de GNOME-koppeling nodig.

De extensie verbergt de GNOME-bovenbalk en conflicterende oude desktopvensters zolang de nieuwe oppervlakken actief zijn. Uitschakelen of uitval herstelt de oorspronkelijke desktoptoestand. Ontbreekt `win11-shell` of start die niet, dan toont de extensie na 6 seconden haar eenvoudige noodtaakbalk. Andere dockextensies worden niet door installatie uitgezet. De Windows-toets opent Start, Ctrl+Alt+Delete opent Taakbeheer en Alt+Tab gebruikt de taakweergave. De instellingen bepalen onder meer taal, taakbalkuitlijning, achtergrond, kleuren en thema. Nederlands en Engels gelden ook voor desktoplabels zoals Prullenbak/Recycle Bin.

## Bestanden, apps en instellingen

De native bestandskern omvat navigatie, metadata, zoeken, mapbewaking, openen met, aanmaken, hernoemen, kopiëren/verplaatsen, klembord, Prullenbak, ZIP/TAR-archieven en eigenschappen. Archieven gebruiken `libarchive`; Prullenbak en appassociaties gebruiken GIO. Atomaire handelingen weigeren bestaande doelen te overschrijven. De bewaarde instellingen gebruiken private, atomair geschreven bestanden en gedeelde bestandsloten.

**Instellingen → Systeem → Energie en batterij** regelt de energiestand, schermuitschakeling, dimmen en automatisch slapen op netstroom of batterij. De Rust-dienst gebruikt dezelfde GNOME-voorkeuren en PowerProfiles-dienst als Ubuntu. Bestaande aangepaste tijden en ‘Nooit’ blijven behouden; wijzigingen vanuit GNOME verschijnen ook in dit venster. Beschikbaarheid en beheerdersvergrendeling worden per instelling getoond. Batterijopties verschijnen alleen wanneer een batterij aanwezig is.

De oorspronkelijke `apps.*`-functies, recente bestanden en appcatalogus draaien in Rust. **Standaardapps** voegt een afzonderlijk programma toe waarmee een gebruiker op extensie of MIME-type zoekt en precies het gekozen bestandstype aan een relevante geïnstalleerde app koppelt. De effectieve desktopvoorrang wordt teruggelezen. Openen of installeren verandert deze associaties niet automatisch.

Normale appvensters bewaren hun laatste normale positie, afmetingen en gemaximaliseerde toestand. Verkenner bewaart daarnaast de semantische toestand van maximaal twintig tabs. Een expliciet meegegeven pad of taboverdracht gaat voor herstel. Herstel begrenst het venster binnen een beschikbaar scherm. Start, taakbalk, desktop en tijdelijke bestandskiezers worden hiervan uitgesloten.

Het Verkenner-detailspaneel vraagt een videominiatuur alleen voor de zichtbare geselecteerde video. Bestaande freedesktopminiaturen worden hergebruikt; anders maakt een kortlevende native helper één stilstaand beeld met FFmpeg. Er draait geen videospeler op de achtergrond. Zonder geschikte decoder of bij een fout blijft het oorspronkelijke bestandspictogram zichtbaar. De nieuwe videoroute is een expliciete uitbreiding; oorspronkelijke afbeeldingsroutes behouden hun gedrag. Zie [host/src/images/README.md](host/src/images/README.md).

## Installeren en verwijderen van apps

De nieuwe softwaredienst herkent AppImage, DEB, Flatpak-verwijzingen/-bundels en Snap-pakketten. Inspectie voert een gedownloade AppImage niet uit. Een installatievoorstel bewaart de te beoordelen bytes privé; pas een afzonderlijke bevestiging voert het eenmalige voorstel uit. Voorstellen verlopen na tien minuten. Er kan één softwarewijziging tegelijk actief zijn. Status en annulering worden bewaard voor andere vensters; een onbekende uitkomst wordt als onderbroken gemeld.

AppImages krijgen een beheerde kopie onder `~/.local/opt/win11shell-appimages` en een eigen launcher. Het oorspronkelijke downloadbestand blijft staan. Verwijderen raakt alleen de geregistreerde appkopie en launcher; onverwachte extra bestanden blijven behouden. Bij een losse gebruikerslauncher heet de actie uitdrukkelijk **Snelkoppeling verwijderen** en blijven de appbestanden staan.

DEB gebruikt getypeerde PackageKit-transacties en een controlesimulatie; aanvullende of beschermde pakketverwijderingen worden geweigerd. Snap gebruikt de Unix-API van snapd en zijn voortgang/annulering. Flatpak gebruikt vaste argumenten en de vastgestelde gebruikers- of systeeminstallatie. Eventuele authenticatie verloopt via het besturingssysteem. Er worden geen wachtwoorden verzameld. Persoonlijke Flatpak-gegevens worden niet verwijderd en Snap-verwijdering gebruikt geen purge.

De engine is gecontroleerd met gegenereerde bestanden en nagemaakte pakketdiensten. Echte systeeminstallaties, verwijderingen en alle distributiespecifieke authenticatiedialogen zijn hiermee nog niet aangetoond. De nieuwe Installer-interface en koppelingen met Instellingen/Start worden afzonderlijk gevalideerd vóór uitgave.

## Bestandskiezer via desktopportals

`win11-file-portal` implementeert OpenFile, SaveFile en SaveFiles achter `xdg-desktop-portal`. Het gebruikt dezelfde Verkenner-interface en native validatie als de interne kiezer. Alleen deze FileChooser-backend wordt geselecteerd; de portalfrontend blijft verantwoordelijk voor appidentiteit en documenttoegang. De kiezer maakt of overschrijft het gekozen bestand niet zelf.

Apps moeten de desktopportal gebruiken om deze kiezer te zien. Een eigen ingebouwde appkiezer verandert hierdoor niet. De backend stopt na vijftien seconden zonder aanvragen en deelt alleen tussen zijn tijdelijke kiezervensters een WebKit-context. Details en protocolgrenzen staan in [host/src/portal/README.md](host/src/portal/README.md). Registratie, backendselectie en installatie worden afzonderlijk gecontroleerd.

## Geheugen en matglas

Diensten starten pas wanneer een oppervlak ze nodig heeft. De Alles-indexer is een afzonderlijk Rust-proces en hoort uitsluitend bij de geopende Alles-app. De oorspronkelijke v2-indexcache blijft bruikbaar. Taakbalk, flyouts, Start en bureaublad zijn vensters van één GTK 4-proces, dus Start openen start geen nieuw proces. Gewone apps blijven losse processen.

In een private 1440×900-sessie met softwarerendering gebruikten `win11-shell`, zijn backend en de SVG-lader samen **119 MiB PSS en 76 MiB anoniem geheugen plus swap**, tegen 443 en 269 MiB voor de eerdere WebKit-taakbalk, -Start en -bureaublad met al hun WebKit-processen (`tests/gnome/scenarios/native-shell.py`). De shell gebruikt standaard de cairo-renderer van GTK (`GSK_RENDERER=cairo`); met de GL-renderer (llvmpipe) was dat 244 en 113 MiB. PSS telt gedeeld geheugen proportioneel en verschilt van een opgetelde RSS-kolom. Deze meting omvat geen compositorgeheugen of GPU-VRAM. Zie [notes/claude/gtk-native.md](notes/claude/gtk-native.md).

De backend van de shell (`win11-panel --http-only`) draait in een eigen begrensde systemd-groep wanneer de gebruikersmanager beschikbaar is; `win11-shell` zelf valt daar nog buiten. Gewone gestarte apps horen in eigen groepen. Dit beleid en de genoemde metingen zijn geen universele geheugengarantie voor alle videodecoders, extra apps of schermconfiguraties.

Matglas komt van GNOME Shell zelf, niet van GTK of WebKit: onder de taakbalk, onder het geopende native Startmenu en onder het actieve eigen app-venster, met deze dekking (taakbalk 70 %, app-vensters en Start 90 %). Er bestaat hooguit één vensterblur tegelijk; een gesloten Start houdt er geen vast. In een private 1440×900-sessie met softwarerendering kostte dat gnome-shell 3,8 MiB PSS met Instellingen en 8,9 MiB met Start open, nog gemeten met de WebKit-Start (`tests/gnome/scenarios/frosted-glass.py`). Met Transparantie-effecten uit of hoog contrast blijft alles dekkend. Blur in de pagina van een HTML-app (menu's binnen een app) staat uit: die heeft WebKit op de GPU nodig, wat op NVIDIA eerder vastliep.

## Bouwen

Vereist: Rust/Cargo met ondersteuning voor edition 2024 (Rust 1.92 of nieuwer), een C/C++-bouwomgeving, GTK 3 en WebKitGTK 4.1 voor de apps en GTK 4.22 of nieuwer (GNOME 50) voor `win11-shell` en `win11-calendar`. Op de geteste Ubuntu-omgeving (Ubuntu 26.04, GTK 4.22):

```bash
sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev libgtk-4-dev libarchive13t64
cargo fetch --locked
cargo fetch --locked --manifest-path native/Cargo.toml
./scripts/build.sh
```

`./scripts/build.sh` bouwt de host (de workspace in de hoofdmap) en daarna de native workspace `native/` met `win11-shell` en `win11-calendar`. Dat zijn twee workspaces, omdat gtk4-rs 0.11 een nieuwere glib gebruikt dan de GTK 3- en WebKit-host en Cargo één `glib-sys` per workspace toestaat. Beide bouwen offline en `--locked`; `cargo fetch` haalt de crates daarom vooraf één keer op. Crates en bouwuitvoer mogen elders staan, bijvoorbeeld op een andere schijf, met de gewone Cargo-variabelen. Zet die dan al vóór `cargo fetch` en geef de installer de map met de binaries:

```bash
export CARGO_HOME=/absoluut/pad/cargo-home CARGO_TARGET_DIR=/absoluut/pad/target
./scripts/build.sh
WIN11_BUILD_DIR="$CARGO_TARGET_DIR/release" ./scripts/install.sh
```

De archiefkern vereist de systeembibliotheek `libarchive.so.13`; pakketnamen kunnen per distributie verschillen. Netwerk, audio, opslag, energie en notificaties gebruiken de bijbehorende Linux-/GNOME-diensten. Videominiaturen gebruiken optioneel FFmpeg/FFprobe. Software-installatie gebruikt, afhankelijk van het pakket, PackageKit, Flatpak of snapd; DEB/Snap-metadata gebruikt `dpkg-deb`/`unsquashfs`. Beschikbare bubblewrap-isolatie wordt voor onbetrouwbare metadata-/afbeeldingdecoders gebruikt, naast aparte helpers en resourcegrenzen.

Python 3 is nodig voor de huidige installatie- en pakkettools en sommige ontwikkeltests. PyGObject is alleen nodig wanneer de oorspronkelijke Python-referentie expliciet wordt getest. Het pakket bundelt geen Python-omgeving of kopieën van systeembibliotheken.

Controleer de bewaarde oorspronkelijke UI-baseline en de actuele distributiemanifesten met:

```bash
python3 scripts/verify-ui.py
```

`vendor/ui-original-sha256.json` bewaart de oorspronkelijke hashes; `vendor/ui-sha256.json` en `vendor/ui-patches.json` beschrijven de huidige interface en gerichte wijzigingen. De oorspronkelijke bronbestanden zijn dus een uitgangspunt, geen claim dat ieder bestand ongewijzigd is gebleven.

## Installeren en openen

Vanuit een gebouwd project of een voorbereid pakket met `bin/`:

```bash
./scripts/install.sh
~/.local/bin/win11-explorer /pad/naar/map
~/.local/bin/win11-app --app settings
~/.local/bin/win11-default-apps
```

De installer controleert de vereiste programma's vooraf en plaatst de gemarkeerde projectbestanden standaard onder `~/.local`. Bestaande persoonlijke instellingen worden behouden. Een installatie over een eerdere versie met de WebKit-shell verwijdert de oude `win11-start` en `win11-desktop` met hun app-items. Voor de shellkoppeling moet GNOME de nieuwe extensiecode laden; na een nieuwe installatie of extensiewijziging kan één keer afmelden en aanmelden nodig zijn. Activeer daarna de koppeling expliciet:

```bash
gnome-extensions enable win11-gnome@win11shell.local
```

Een reeds actief Ubuntu Dock of andere taakbalkextensie kan tegelijk zichtbaar blijven. Schakel een conflicterende taakbalkextensie desgewenst uit voordat de nieuwe shell wordt geactiveerd, bijvoorbeeld:

```bash
gnome-extensions disable ubuntu-dock@ubuntu.com
```

Er wordt geen Fluent GTK-thema geïnstalleerd. De standaard app-themafamilie is `system`. Installeren kiest geen nieuwe standaardbestandsbeheerder en past geen persoonlijke MIME-associaties aan.

Voor een ander installatiepad bepaalt `WIN11_INSTALL_PREFIX` de locatie van `bin/` en `share/win11-gnome/`; `XDG_DATA_HOME` bepaalt waar applicatie-items, iconaliases en de extensie komen. Gebruik absolute paden. Dezelfde waarden zijn nodig bij verwijderen:

```bash
WIN11_INSTALL_PREFIX="/absoluut/pad/met spaties" ./scripts/install.sh
WIN11_INSTALL_PREFIX="/absoluut/pad/met spaties" ./scripts/uninstall.sh
```

Verwijderen raakt de eigen gemarkeerde projectbestanden en bewaart persoonlijke instellingen. `./scripts/uninstall.sh --files-only` is bedoeld voor een geïsoleerde testinstallatie zonder bediening van een desktopsessie.

## Taal

Win11 voor GNOME heeft een eigen taalinstelling: Nederlands (standaard) of Engels. Die staat los van de taal van GNOME en Ubuntu. Kies de taal in **Instellingen › Tijd en taal › Taal en regio**. Alle geopende Win11-vensters schakelen meteen om: taakbalk, Start, bureaublad, Verkenner, de apps en hun venstertitels. De backend van de shell (`win11-panel --http-only`) past ook de namen van de eigen app-items aan, zodat GNOME die in dezelfde taal toont. Meldingen die al binnen waren, houden hun taal. Een eerste installatie kan de taal meteen vastleggen; een keuze die al in `settings.json` staat, blijft gelden:

```bash
./scripts/install.sh --lang en
```

Een paar teksten komen niet van Win11 en blijven in de taal van GNOME of het systeem: de wachtwoordvensters van polkit, berichten en pakketbeschrijvingen van PackageKit, teksten van NetworkManager, de eigen teksten van WebKit en de onderdelen van GNOME zelf, zoals de snelle instellingen en GNOME's melding dat een venster klaarstaat (in een Engelstalige GNOME: “X” is ready). Alles heet in beide talen Alles, en mappen op schijf, zoals Bureaublad en Documenten, worden nooit hernoemd.

De variabele `WIN11_GNOME_CONFIG_DIR` laat de Rust-programma's een andere instellingenmap gebruiken. Die is alleen bedoeld voor tests en ontwikkeling: de GNOME-extensie en de installer lezen altijd `$XDG_CONFIG_HOME/win11-gnome` (meestal `~/.config/win11-gnome`), dus in een gewone sessie zouden de taakbalk en de vensters elk een andere taal kunnen tonen. Zet hem daarom niet in een gewone sessie.

## Broncode en uitgave

De eigen code van Win11 voor GNOME valt onder [LICENSE](LICENSE) (MIT): de Rust-host, de native GTK 4-shell en -agenda onder `native/`, de installatiecode en ook de bewaarde Win11Shell-bronnen onder `vendor/ui`, `vendor/host` en `vendor/config`, die eigen eerder werk van de auteur zijn. De uitzonderingen staan in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Het meegeleverde Win11-icon-theme heeft zijn eigen GPL-3.0-licentie, auteursgegevens en bronarchief onder `vendor/`; de pakketmanifesten controleren de herkomst en symlinks. Zie onder meer [vendor/ui/assets/icons/NOTICE.txt](vendor/ui/assets/icons/NOTICE.txt) en `vendor/icon-bundle.json`. De projectlicentie verleent geen aanvullende rechten op materiaal van derden.

Het openbare profiel komt uit `host/src/settings-defaults.json`. Persoonlijke geschiedenis, accounts, indexcaches, computergebonden paden en runtime-tokens horen niet in een downloadpakket. `./scripts/package.sh` maakt na bouwen en testen een archief onder `dist/`, met daarin ook `LICENSE`, `THIRD_PARTY_NOTICES.md` en `LICENSES/`; het genereert zelf geen testbewijs of openbare release.

De voorwaarden voor publicatie staan in [RELEASE_PLAN.md](RELEASE_PLAN.md). Voor uitgave moeten de definitieve geïntegreerde build, frontend, installatie/verwijdering, ondersteunde systemen en assetvermeldingen samen worden gecontroleerd. Historische rapporten blijven in het project beschikbaar met hun oorspronkelijke versie en meetdatum.

## Licenties

- De eigen code en assets van Win11 voor GNOME vallen onder de MIT-licentie: zie [LICENSE](LICENSE). Dat geldt ook voor `vendor/ui`, `vendor/host` en `vendor/config`, eigen werk uit het eerdere Win11 Shell-project, behalve de onderdelen van derden die in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) staan.
- Het meegeleverde Win11-icon-theme (`vendor/icons/`, `vendor/icon-source/` en `vendor/ui/assets/icons/settings.svg`) valt onder GPL-3.0-only, niet onder MIT. De wijzigingen staan in [vendor/ICON_PROVENANCE.md](vendor/ICON_PROVENANCE.md).
- De standaardachtergronden Lotus (`vendor/ui/assets/wallpapers/lotus-dark.svg` en `lotus-light.svg`) zijn met AI gemaakt (beeldgeneratie van OpenAI ChatGPT) en vallen onder CC0-1.0: zie [LICENSES/CC0-1.0.txt](LICENSES/CC0-1.0.txt).
- Onderdelen van derden houden hun eigen licentie. Welke dat zijn, waar ze staan en wat hun licentie vraagt, staat in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). De volledige licentieteksten staan in [LICENSES/](LICENSES/), en de Rust-crates in de programma's staan in [LICENSES/RUST-CRATES.md](LICENSES/RUST-CRATES.md).
