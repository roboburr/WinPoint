# Rust-bestandsfuncties — versie 0.4.0

Gecontroleerd op 1 november 2026 in een eigen schermloze GNOME 50.1/Wayland-sessie. Alle 22 integratiegevallen slagen met twee echte Rust/WebKit-Verkenners, geauthenticeerde host-HTTP en de gedeelde SSE-eventstream. De oorspronkelijke HTML/CSS-interface is behouden. Deze meting bewijst de hieronder beschreven functies en contracten; zij bewijst geen volledige visuele één-op-één-overeenkomst van alle schermen.

## Native bereik

Acht nieuwe methoden draaien in Rust: `files.trash`, `files.trashInfo`, `files.emptyTrash`, `files.restoreTrash`, `files.compress`, `files.extract`, `files.properties` en `files.setAttributes`. Met de twaalf eerder geporteerde methoden zijn dit twintig native Files-methoden. De oorspronkelijke overige diensten blijven hun eigen compatibiliteitsroute gebruiken.

De HTTP-antwoorden dragen `X-Win11-Service: rust-files`. De tweede Verkenner gebruikte bovendien een testkopie van de Python-dienst waarin de acht nieuwe methoden bewust een fout geven. Alle acht bleven via de Rust-host werken. De verspreide Python-bron is daarvoor niet aangepast.

Ook `.desktop`-items lopen nu door de native itemfabriek. De test vergeleek volledige lijsten en items voor een gewone launcher, gelokaliseerde naam, uitvoerbare launcher en launcher-symlink met de oorspronkelijke Python-resultaten, inclusief namen, MIME, pictogrammen en launcher-vlaggen. Het aanzetten van GNOME's `metadata::trusted` is geen afzonderlijk bedieningsgeval in deze suite.

## Geslaagde vergelijking met het origineel

- Eigenschappen van bestanden, mappen, bestands- en mapsymlinks, een gebroken symlink en een gemengde selectie komen overeen. De vergelijking omvat onder andere typen, grootte en aantallen, modi, eigenaar/groep, tijdvelden en de betekenis van symlinks. `accessed` is bij de opeenvolgende enkel-itemvergelijkingen buiten beschouwing gelaten: maptraversal kan juist dat tijdstip tussen beide metingen wijzigen.
- Alleen-lezen en verborgen in- en uitschakelen leveren dezelfde antwoorden en bestandswijzigingen op. Een alleen-lezen wijziging via een symlink en het zichtbaar maken van een puntbestand behouden de oorspronkelijke foutcodes.
- ZIP-uitpakken is vergeleken voor opgeslagen, DEFLATE-, bzip2-, LZMA- en Zstandard-inhoud, UTF-8- en CP437-bestandsnamen. Unix-symlinkattributen blijven conform Python gewone bestandsinhoud, ook met een nulbyte in die inhoud. De bron wordt niet aangepast.
- ZIP-aanmaken behoudt de ledenvolgorde en inhoud van de referentie, lege mappen, het volgen van bestandslinks, het overslaan van maplinks, gebroken links en FIFO's. Reguliere bestanden gebruiken DEFLATE. Compressieniveau 6 staat in de native configuratie; identieke gecomprimeerde bytes zijn niet als eis getest.
- TAR, TAR/gzip, TAR/bzip2, TAR/xz en TAR/Zstandard leveren dezelfde boom op als `tarfile` met het datafilter. Veilige symlinks en hardlinks, gefilterde rechten, gewijzigde tijden en normalisatie van linkdoelen zijn gecontroleerd. De hardlinks delen daadwerkelijk dezelfde inode.
- Traversal, absolute paden, Windows-schijfpaden, ontsnappende symlink- en hardlinkdoelen en apparaat/FIFO-leden worden geweigerd. Een ZIP met foute CRC laat geen gepubliceerde halve map of `.partial` achter; een bestaand doel met sentinel blijft intact.
- Naar de testprullenbak verplaatsen, lijst en grootte ophalen, herstellen bij een bezette oorspronkelijke naam en herstellen van een gebroken symlink komen overeen met de referentie. Bestaande inhoud wordt behouden en een map krijgt bij conflict de volgende naam.
- Legen is uitsluitend tegen de expliciet ingestelde eigen testprullenbak uitgevoerd. De verdwenen testinhoud en de gerapporteerde aantallen zijn gecontroleerd.
- De gecombineerde SSE-stream bevat native map- en prullenbakwijzigingen. Succesvolle compressie en mislukte CRC-extractie leveren afgeronde voortgang met `jobId`, `op`, `done`, `total`, `items`, `count`, `current` en `finished`.

Archiefbewerkingen gebruiken native Rust-bestandslogica met de publieke libarchive-ABI. Er wordt geen Python- of archief-CLI-proces voor gestart. De runtime vereist `libarchive.so.13` (op Ubuntu het toepasselijke `libarchive13`/`libarchive13t64`-pakket).

## Gerichte veiligheidsverbeteringen

Archiefuitvoer wordt eerst in een private tijdelijke boom opgebouwd en vervolgens zonder overschrijven gepubliceerd. Schrijven gebeurt via vastgehouden directorydescriptors. Daardoor kan een archiefsymlink geen later bestand buiten de doelboom laten schrijven. De native tests omvatten ook een ontsnapping via een alias naar een bovenliggende map, corrupte inhoud, conflicten en het opnieuw kiezen van een naam wanneer het reeds gekozen doel intussen verschijnt. Dit behoudt de normale referentie-uitkomsten en weigert onveilige situaties.

Prullenbak legen controleert vóór verwijdering de volledige geselecteerde boom via vastgehouden descriptors, zonder symlinks te volgen. Zowel apparaatnummer als mount-ID worden vergeleken, zodat ook bind mounts op hetzelfde bestandssysteem worden ontdekt. Een gemount kind weigert het geselecteerde hoofditem voordat inhoud wordt verwijderd. Een ontoegankelijke eigen map kan tijdens de voorcontrole extra eigenaarrechten krijgen om de inhoud te inspecteren; het gemounte kind zelf wordt niet aangepast. Tijdens verwijdering worden inode en mount opnieuw gecontroleerd. Symlinks zelf worden verwijderd. De roots `files`, `info` en `expunged` worden eveneens tegen de prullenbakmount gecontroleerd.

In de geïsoleerde prullenbakmodus wordt `.trashinfo` exclusief aangemaakt. Een reeds aanwezige verweesde metadatafile wordt niet overschreven; het nieuwe item krijgt een andere interne naam. Dertien gerichte prullenbak-unittests bevatten onder andere geïnjecteerde mount-ID's en een onleesbaar gemount kind. Er is geen echte mount aangemaakt of actieve gebruikersschijf geleegd om die gevallen te testen.

## Grenzen en reproduceerbaarheid

Alle mutaties bleven onder de eigen testmap. `WIN11SHELL_TRASH_DIR` was altijd expliciet ingesteld, met eigen XDG-configuratie, data, cache en runtime. De actieve desktopconfiguratie en gebruikersprullenbak zijn niet gewijzigd. De twee herstarts tijdens ontwikkeling corrigeerden uitsluitend een verkeerd veld en een overschreven functienaam in de testharness; de uiteindelijke run slaagt volledig.

Deze suite gebruikt HTTP en de oorspronkelijke WebKit-interface voor opstart en bridge-integratie. Zij bedient niet alle prullenbak-, archief- en eigenschappen-dialogen met fysieke muis/toetsenbordinput. Netwerkshares, alle bestandssystemen, werkelijke mountwisselingen, alle mogelijke corrupte archieven, langdurige schijfruimte-uitputting en iedere uitvoeringsvolgorde bij externe races zijn geen volledige integratiedekking. Het oorspronkelijke ontwerp van taakbalk, Start, vensterknoppen en taboverdracht heeft afzonderlijke rapporten; die complete bediening is hier niet herhaald. Instellingen en de bijbehorende schermafbeelding horen bij hun eigen validatie.

Een afzonderlijke geïsoleerde GIO-verplaatsing en herstel naar een private XDG-prullenbak, zonder override, staat beschreven in [VALIDATION_SETTINGS.md](VALIDATION_SETTINGS.md). Ook dat geval gebruikt uitsluitend een eigen bestand; het herhaalt geen productie-layoutlijst of leging en geen werkelijke mounttest.

De ontwikkelharness staat in `work/files-complete-test/run.py`, met de volledige `results.json` en `run.log` in dezelfde map. Zij leest alleen helperdefinities uit `work/files-native-test/run.py` en voert de oudere testbody niet uit. Fixtures voor de oorspronkelijke dienst mogen Python's ZIP/TAR-bibliotheken gebruiken; de gemeten native productie-aanroepen dragen steeds de Rust-header. Het compacte distributiebewijs is `validation/files-complete-results.json`, zonder tokens, persoonlijke instellingen, proces-ID's of bestandsinhoud.
