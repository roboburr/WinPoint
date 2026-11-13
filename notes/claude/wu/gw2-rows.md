# GW2a: the rows of `host/i18n/update.tsv` (design §3.7)

Input for `tools/i18n/extract_rs.py add` (one call per row, `--stem update --apply`); the catalogue itself is
written only by the converter. Columns inside the block, tab-separated: section, key, nl, en, translator note.
nl and en are design §3.7 byte for byte (en-GB, typographic ’, sentence case). `{product}` is always passed by
the caller with the text of `update.product` (C7). One row is not in §3.7: `update.err.notHere` (see gw2.md,
deviations).

```tsv
errors	update.product	Win11 Shell	Win11 Shell	{product} in every update text (design §3.1, C7: the product name until Robin decides branding)
errors	update.err.network	Er is geen verbinding met de updateserver. Controleer je internetverbinding en probeer het opnieuw.	Can’t reach the update server. Check your internet connection and try again.	PackageKit no-network 2, no-more-mirrors 43, repo-not-available 37 (an Ubuntu URI, or GetUpdates failed too)
errors	update.warn.source	Een updatebron kon niet worden bijgewerkt: {source}. De andere updates zijn wel gecontroleerd.	One update source couldn’t be refreshed: {source}. The other updates were checked.	the same codes for a non-Ubuntu source while GetUpdates succeeds; {source} is the host name of the source
errors	update.err.offline	Je bent offline. Maak verbinding met internet en probeer het opnieuw.	You’re offline. Connect to the internet and try again.	NetworkManager connectivity none before a check
errors	update.err.repair	Een eerdere installatie is niet afgerond. Ubuntu moet dit eerst herstellen.	An earlier installation didn’t finish. Ubuntu needs to repair this first.	exit repair-required 11, error unfinished-transaction 66, or a failed configure that leaves half-done packages
errors	update.err.cannotCancel	De installatie is al begonnen en kan nu niet meer worden gestopt.	Installing has already started and can’t be stopped now.	cannot-cancel 25, or Cancel while AllowCancel is false
errors	update.err.notAuthorized	Je hebt geen toestemming gegeven, dus er is niets veranderd.	Permission wasn’t given, so nothing was changed.	not-authorized 48 (polkit)
errors	update.err.noSpace	Er is niet genoeg ruimte op de systeemschijf. Maak ruimte vrij en probeer het opnieuw.	There isn’t enough space on the system drive. Free up some space and try again.	no-space-on-device 46
errors	update.err.download	Een update kon niet worden gedownload. Probeer het later opnieuw.	An update couldn’t be downloaded. Try again later.	package-download-failed 10
errors	update.err.locked	Er wordt al software geïnstalleerd. Probeer het over een paar minuten opnieuw.	Software is already being installed. Try again in a few minutes.	cannot-get-lock 26, lock-required 67, cancelled-priority 65 / exit cancelled-priority 9
errors	update.err.untrusted	Deze update is niet ondertekend en wordt daarom niet geïnstalleerd.	This update isn’t signed, so it won’t be installed.	gpg-failure 5, bad/missing-gpg-signature 30/31, cannot-update-repo-unsigned 51, exit need-untrusted 8
errors	update.err.conflict	Ubuntu kan deze updates nu niet samen installeren. Probeer het later opnieuw, of gebruik de Software-updater.	Ubuntu can’t install these updates together right now. Try again later, or use Software Updater.	dep-resolution-failed 13, package-conflicts 36, file-conflicts 35, package-install-blocked 39
errors	update.err.dpkg	Een update is niet goed geïnstalleerd. Probeer het opnieuw; helpt dat niet, open dan de Software-updater.	An update didn’t install correctly. Try again; if that doesn’t help, open Software Updater.	package-failed-to-configure 56, -install 58, update-failed-due-to-running-process 60
errors	update.err.cancelled	Het installeren is gestopt. Er is niets half geïnstalleerd.	Installing was stopped. Nothing was left half-installed.	transaction-cancelled 17, exit cancelled 3 (cancel is only offered during the download)
errors	update.err.failed	Het installeren van de updates is mislukt.	The updates couldn’t be installed.	anything else, incl. EulaRequired, MediaChangeRequired, RepoSignatureRequired
errors	update.err.checkFailed	Controleren op updates is mislukt.	Checking for updates didn’t work.	RefreshCache and GetUpdates both failed for a non-network reason
errors	update.err.noPackageKit	PackageKit ontbreekt, dus {product} kan zelf geen updates installeren.	PackageKit is missing, so {product} can’t install updates itself.	no PackageKit name on the bus and not activatable
errors	update.err.unknownOutcome	Het is niet bekend of de updates zijn geïnstalleerd. Controleer opnieuw op updates.	It isn’t known whether the updates were installed. Check for updates again.	PackageKit vanished mid-transaction and GetOldTransactions has no record
errors	update.err.busy	Er wordt al een update geïnstalleerd.	An update is already being installed.	our own job already runs (the job lock is held)
errors	update.err.stale	De lijst met updates is veranderd. Controleer opnieuw op updates.	The list of updates changed. Check for updates again.	ids or rows not in the current snapshot; package-database-changed 61
errors	update.err.badArgs	Ongeldige invoer voor updates.	Invalid update input.	invalid arguments of an update.* method
errors	update.err.testSeam	Een testinstelling voor updates klopt niet, dus er is niets gedaan.	An update test setting is wrong, so nothing was done.	a WIN11_UPDATE_* variable is set but fails its check (design §4.11); only tests ever see it
errors	update.err.details	De details van deze update zijn niet beschikbaar.	The details of this update aren’t available.	GetUpdateDetail failed
errors	update.err.notHere	Updates installeren kan alleen vanuit Instellingen.	Updates can only be installed from Settings.	an update write method in a host that is not Settings (reader mode, design §4.1); not in §3.7 (GW2 deviation)
guards	update.guard.power	Er worden updates geïnstalleerd. Opnieuw opstarten kan zodra dat klaar is.	Updates are being installed. You can restart as soon as that’s done.	a power action while our job is past AllowCancel (committing)
guards	update.guard.upgrade	Ubuntu wordt bijgewerkt naar een nieuwe versie. Wacht tot het upgradevenster om een herstart vraagt.	Ubuntu is being upgraded to a new version. Wait until the upgrade window asks you to restart.	a power action, shell.restartAll or any update write while Ubuntu's release upgrader runs
guards	update.guard.restart	Er worden updates geïnstalleerd. {product} kan opnieuw starten zodra dat klaar is.	Updates are being installed. {product} can restart as soon as that’s done.	shell.restartAll while one of our update jobs is active (any phase)
guards	update.guard.software	Er wordt nog een app geïnstalleerd of verwijderd. Probeer het daarna opnieuw.	An app is still being installed or removed. Try again afterwards.	shell.restartAll while a Software job is queued, running or authenticating
notifications	update.notify.installed	Updates zijn geïnstalleerd	Updates installed	notification title: our job is done
notifications	update.notify.installedBody	Je pc is up-to-date.	Your PC is up to date.	notification body under update.notify.installed
notifications	update.notify.restart	Opnieuw opstarten vereist	Restart required	notification title: our job is done and needs a restart (settings.update.notifyRestart)
notifications	update.notify.restartBody	Start je pc opnieuw op om de updates af te ronden.	Restart your PC to finish installing updates.	notification body under update.notify.restart
notifications	update.notify.restartNow	Nu opnieuw opstarten	Restart now	notification action button
notifications	update.notify.later	Later	Later	notification action button (the same word in both languages)
notifications	update.notify.failed	Updates konden niet worden geïnstalleerd	Updates couldn’t be installed	notification title: our job failed
notifications	update.notify.view	Bekijken	View	notification action button: opens Settings on the update page
release	update.release.terminalTitle	Ubuntu-upgrade	Ubuntu upgrade	title of the terminal window that runs Ubuntu's release upgrader
release	update.release.err.updatesPending	Installeer eerst de beschikbare updates.	Install the available updates first.	release start refused: a non-phased update is pending (do-release-upgrade's own rule)
release	update.release.err.rebootFirst	Start je pc eerst opnieuw op.	Restart your PC first.	release start refused: reboot-required lists libc6, linux-base or linux-image-*
release	update.release.err.busy	Er wordt nog software geïnstalleerd. Probeer het daarna opnieuw.	Software is still being installed. Try again afterwards.	release start refused: a transaction or dpkg runs
release	update.release.err.hold	{product} is nog niet klaar voor deze Ubuntu-versie.	{product} isn’t ready for this Ubuntu version yet.	release start refused: the compatibility verdict is hold or blocked
release	update.release.err.notOffered	Ubuntu biedt op dit moment geen nieuwe versie aan.	Ubuntu isn’t offering a new version right now.	release start refused: nothing offered
release	update.release.err.noTerminal	Er is geen terminal gevonden. Installeer Ptyxis (Terminal) en probeer het opnieuw.	No terminal was found. Install Ptyxis (Terminal) and try again.	no xdg-terminal-exec
release	update.release.err.notRunning	De upgrade van Ubuntu loopt niet meer.	The Ubuntu upgrade isn’t running any more.	attach refused: the upgrader is gone
release	update.release.err.noAttachHelper	Open een terminal en typ: sudo screen -d -r ubuntu-release-upgrade-screen-window	Open a terminal and type: sudo screen -d -r ubuntu-release-upgrade-screen-window	attach without the root helper; the command is typed by the user, never by our code
release	update.release.err.noPassword	Je hebt het wachtwoord niet ingevuld, dus er is niets veranderd.	You didn’t enter the password, so nothing was changed.	pkexec rc 126
release	update.release.err.wrongPassword	Het wachtwoord klopte niet. Er is niets veranderd.	The password was wrong. Nothing was changed.	pkexec rc 127
release	update.release.err.promptFailed	De instelling voor nieuwe Ubuntu-versies kon niet worden opgeslagen.	The setting for new Ubuntu versions couldn’t be saved.	the Prompt helper printed @@fail
release	update.release.err.checkFailed	Controleren op een nieuwe Ubuntu-versie is mislukt.	Checking for a new Ubuntu version didn’t work.	check-new-release failed
release	update.release.err.refused	Ubuntu heeft de upgrade niet gestart. De reden staat in het terminalvenster.	Ubuntu didn’t start the upgrade. The reason is in the terminal window.	the wrapper's rc is not 0, 126 or 127 before the upgrader appeared
release	update.release.err.noWindow	Het upgradevenster is niet verschenen. Probeer het opnieuw.	The upgrade window didn’t appear. Try again.	no upgrader and no status file 120 s after the start
release	update.release.pressEnter	Druk op Enter om dit venster te sluiten.	Press Enter to close this window.	last line of the terminal wrapper when it keeps the window open
release	update.release.err.ubuntuHold	De melding van Ubuntu kon niet worden uitgesteld.	Ubuntu’s own prompt couldn’t be postponed.	update.release.ubuntuHold could not write the update-manager keys
compat	update.compat.missing	{part} zit niet in Ubuntu {version}.	{part} isn’t in Ubuntu {version}.	compatibility reason: a blocking part has no candidate; {part} is an update.part.* text
compat	update.compat.removed	Ubuntu {version} verwijdert {part}.	Ubuntu {version} removes {part}.	compatibility reason: the simulation removes a blocking part
compat	update.compat.degraded	{part} ontbreekt in Ubuntu {version}; {feature} werkt dan niet.	{part} is missing from Ubuntu {version}; {feature} won’t work then.	compatibility reason: a degraded part is missing or removed; {feature} is an update.feature.* text
compat	update.compat.thirdParty.one	Ubuntu zet tijdens de upgrade 1 extra softwarebron uit ({name}).	Ubuntu turns off 1 extra software source during the upgrade ({name}).	compatibility note: third-party sources exist; {name} is the first source's name
compat	update.compat.thirdParty.other	Ubuntu zet tijdens de upgrade {n} extra softwarebronnen uit, zoals {name}.	Ubuntu turns off {n} extra software sources during the upgrade, such as {name}.	plural of update.compat.thirdParty
compat	update.compat.offline	Je bent offline.	You’re offline.	compatibility unknown: offline
compat	update.compat.metered	Je gebruikt een verbinding met een datalimiet; zet Updates downloaden via verbindingen met een datalimiet aan om toch te controleren.	You’re on a metered connection; turn on Download updates over metered connections to check anyway.	compatibility unknown: metered and settings.update.metered is off; names the Advanced options toggle
compat	update.compat.apt	Het pakketoverzicht van Ubuntu {version} kon niet worden geladen.	The package list of Ubuntu {version} couldn’t be loaded.	compatibility unknown: the private apt tree failed
parts	update.part.webkit	WebKitGTK (webweergave)	WebKitGTK (web view)	part label (compatibility reasons)
parts	update.part.jsc	JavaScriptCore	JavaScriptCore	part label (a name, the same in both languages)
parts	update.part.gtk	GTK 3	GTK 3	part label (a name)
parts	update.part.soup	libsoup	libsoup	part label (a name)
parts	update.part.glib	GLib	GLib	part label (a name)
parts	update.part.pixbuf	GDK Pixbuf (afbeeldingen)	GDK Pixbuf (images)	part label
parts	update.part.cairo	cairo (tekenen)	cairo (drawing)	part label
parts	update.part.zlib	zlib	zlib	part label (a name)
parts	update.part.archive	libarchive (archieven openen)	libarchive (opening archives)	part label
parts	update.part.gnomeShell	GNOME Shell	GNOME Shell	part label (a name)
parts	update.part.ffmpeg	ffmpeg	ffmpeg	degraded part (a name)
parts	update.feature.ffmpeg	video-miniaturen	video thumbnails	what does not work without ffmpeg ({feature} in update.compat.degraded)
parts	update.part.bubblewrap	bubblewrap	bubblewrap	degraded part (a name)
parts	update.feature.bubblewrap	veilige voorbeelden	safe previews	what does not work without bubblewrap
parts	update.part.squashfs	squashfs-tools	squashfs-tools	degraded part (a name)
parts	update.feature.squashfs	snap-pakketten bekijken	inspecting snap packages	what does not work without squashfs-tools
parts	update.part.flatpak	flatpak	flatpak	degraded part (a name)
parts	update.feature.flatpak	Flatpak-apps installeren	installing Flatpak apps	what does not work without flatpak
parts	update.part.packagekit	PackageKit	PackageKit	degraded part (a name)
parts	update.feature.packagekit	updates en software installeren	installing updates and software	what does not work without PackageKit
parts	update.part.terminal	een terminal	a terminal	degraded part: ptyxis, gnome-terminal or gnome-console
parts	update.feature.terminal	de volgende Ubuntu-upgrade	the next Ubuntu upgrade	what does not work without a terminal
parts	update.part.screen	screen	screen	degraded part (a name)
parts	update.feature.screen	de volgende Ubuntu-upgrade	the next Ubuntu upgrade	what does not work without screen
parts	update.part.dock	Ubuntu Dock	Ubuntu Dock	degraded part (a name)
parts	update.feature.dock	het vangnet na een upgrade	the safety net after an upgrade	what does not work without the Ubuntu Dock
post	update.post.upgraded	Ubuntu is bijgewerkt naar {version}	Ubuntu was upgraded to {version}	post-upgrade notification title: first login on a new release, extension active
post	update.post.extOff	GNOME {major} heeft {product} uitgezet	GNOME {major} turned {product} off	post-upgrade notification title: the extension is OUT_OF_DATE (4) or INACTIVE (2) although enabled
post	update.post.extOffBody	De taakbalk van {product} werkt nog niet met deze GNOME-versie. Zet de Ubuntu Dock aan tot er een update is.	{product}’s taskbar doesn’t work with this GNOME version yet. Turn on the Ubuntu Dock until there’s an update.	post-upgrade notification body
post	update.post.dockOn	Ubuntu Dock aanzetten	Turn on Ubuntu Dock	post-upgrade notification action
post	update.post.later	Later	Later	post-upgrade notification action (the same word in both languages)
post	update.post.extError	{product} kon niet starten	{product} couldn’t start	post-upgrade notification title: the extension is in ERROR (3)
post	update.post.extErrorBody	GNOME meldt een fout in {product}. Zet de Ubuntu Dock aan of installeer {product} opnieuw.	GNOME reports an error in {product}. Turn on the Ubuntu Dock or reinstall {product}.	post-upgrade notification body
post	update.post.extsOff	GNOME heeft extensies uitgezet	GNOME turned extensions off	post-upgrade notification title: org.gnome.shell disable-user-extensions is true
post	update.post.extsOffBody	Na een fout heeft GNOME alle extensies uitgezet, ook {product}.	After an error, GNOME turned off all extensions, including {product}.	post-upgrade notification body
post	update.post.extsOn	Extensies weer aanzetten	Turn extensions back on	post-upgrade notification action
post	update.post.missing	{product} mist onderdelen	{product} is missing components	post-upgrade notification title: blocking parts are missing after the upgrade
post	update.post.missingBody	Na de upgrade ontbreekt: {parts}.	Missing after the upgrade: {parts}.	post-upgrade notification body; {parts} is a joined list of update.part.* texts
post	update.post.installParts	Onderdelen installeren…	Install components…	post-upgrade notification action
```
