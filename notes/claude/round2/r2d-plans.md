# R2-D converter plans (round 2)

The plan files given to `tools/i18n/extract_rs.py convert <file> --stem <stem> --plan -` (columns: ordinal at the time
of the run, key, placeholder names, language expression, en, translator note). Ordinals are those `list` printed right
before each run (after the lang-parameter refactors), not 3b898e6 ordinals; the `#@` lines in host/i18n/*.tsv hold the
3b898e6 ordinals. Kept here as the record of the conversion (notes/claude/round2/r2d.md §2).

## host/src/picker.rs (stem picker)

```tsv
1	picker.tooMany		lang	Four file pickers are already open.	shell.pickFile/portal: at most four pickers per process
2	picker.parentClosed		lang	The parent window has closed.	the window that asked for the picker is gone
3	picker.invalidAddress		lang	Invalid window address	the picker's page URL could not be parsed; no full stop in nl
16	picker.couldNotOpen		lang	Couldn’t open the file picker.	GTK did not create the picker window
29	picker.closed		lang	This selection window has closed.	a submit arrived for a picker that is gone
37	picker.chooseValidSelection		lang	Choose a valid file selection.	the page submitted none, too many or more than one path for a single choice
38	picker.chooseValidPath		lang	Choose a valid file path.	an empty, overlong or NUL-containing path
42	picker.invalidFileType		lang	Invalid file type.	the chosen filter index is out of range
47	picker.invalidOption		lang	Invalid option.	a portal choice (e.g. encoding) has no valid value
52	picker.invalidOption		lang	Invalid option.	
61	picker.chooseAbsolutePath		lang	Choose an absolute file path.	
62	picker.chooseExistingFolder		lang	Choose an existing folder.	folder mode ("Map kiezen")
63	picker.chooseFileInFolder		lang	Choose a file in an existing folder.	save mode ("Opslaan als"): the name's folder must exist
64	picker.chooseExistingFile		lang	Choose an existing file.	open mode ("Openen")
```

## host/src/transfers.rs (stem transfers)

```tsv
6	transfers.invalidTicket		lang	Invalid tab transfer code	Verkenner tab drag between windows: the ticket is malformed; no full stop in nl
19	transfers.expired		lang	The tab transfer has expired; the original tab is kept	the offer is older than 90 s; the source window keeps its tab
22	transfers.alreadyHere		lang	This tab is already in this window	a window tried to take over its own tab
25	transfers.claimedElsewhere		lang	Another window is already taking over this tab	
35	transfers.cantWithdraw		lang	A confirmed transfer can’t be withdrawn	tabRelease after tabCommit
38	transfers.onlySourceChecks		lang	Only the original window can check the transfer	tabStatus from a window that did not offer the tab
42	transfers.onlySourceDetaches		lang	Only the original window can detach the tab	tabDetach from a window that did not offer the tab
47	transfers.unknownAction		lang	Unknown tab transfer action	
49	transfers.claimedByOther		lang	Another window has claimed this tab	
51	transfers.invalidFile		lang	Invalid tab transfer file	the private offer/claim file is not a small private regular file
53	transfers.invalidContent		lang	Invalid tab contents	the offered tab state is no version-1 object or over 1 MiB
64	transfers.invalidFolder		lang	Invalid folder in the tab	a location in the tab state or its history is not valid
```

## host/src/gnome.rs (stem gnome)

```tsv
10	gnome.actionFailed		lang	The GNOME action failed	the extension refused a call without a message of its own; no full stop in nl
13	gnome.sessionUnreachable	error	lang	Can’t reach the GNOME session: {error}	{error} is GIO's D-Bus error text (system language)
16	gnome.invalidReply		lang	Invalid reply from the GNOME link	the extension's D-Bus reply is not one string ("koppeling": the extension link, as in ext.offline)
17	gnome.invalidJson	error	lang	Invalid GNOME JSON: {error}	{error} is serde_json's English parse error
59	gnome.windowActionFailed		lang	The GNOME window action failed	
63	gnome.regionNoRects		lang	The input region has no list of rectangles	the panel page's input region (window.rs Regions request)
64	gnome.tooManyRects		lang	Too many input rectangles	more than 512
65	gnome.rectNoValue	key	lang	An input rectangle has no valid {key}	{key} is x, y, w or h
70	gnome.panelNotRealized		lang	The panel window isn’t realised yet	
71	gnome.regionFailed	error	lang	Couldn’t create the input region: {error}	{error} is cairo's status
```

## host/src/process_scope.rs (stem launch)

```tsv
36	launch.nonUtf8EnvName		lang	Non-UTF-8 environment variable name	starting an app as a systemd unit (process_scope.rs): the environment must be UTF-8 text; no full stop in nl
37	launch.nonUtf8EnvValue		lang	Non-UTF-8 environment variable value	
41	launch.invalidSystemdEnvironment		lang	Invalid systemd environment	the user manager's Environment property has an unexpected type
42	launch.programNotFound		lang	Program not found	the command is not in PATH
43	launch.nonUtf8ProgramPath		lang	Non-UTF-8 program path	
44	launch.nonUtf8Argument		lang	Non-UTF-8 program argument	
45	launch.nonUtf8Argument		lang	Non-UTF-8 program argument	
68	launch.nonUtf8WorkingDirectory		lang	Non-UTF-8 working directory	
77	launch.invalidServicePath		lang	Invalid service address	systemd returned no unit object path ("adres": the D-Bus object path)
81	launch.invalidServiceState		lang	Invalid service status	systemd's service properties have an unexpected type
86	launch.programFailed		lang	The program couldn’t be started.	the unit failed before it had a main process (Task Manager restart, shell.openApp)
87	launch.programTimedOut		lang	The program didn’t start in time.	no main process after 8 s
```

## host/src/launch.rs (stem launch)

```tsv
19	launch.unknownApp		lang	This Win11Shell app doesn’t exist.	shell.openApp / Verkenner's "open in app": the app name is not one of Win11's own apps (branding Win11Shell kept, D-G16)
20	launch.paramsNotObject		lang	App parameters must be a JSON object.	
21	launch.hostNotFound	error	lang	Couldn’t find the Rust app host: {error}	the current executable's path (its folder holds win11-app …); {error} is the OS error
```

## host/src/bin/launch.rs (stem launch)

```tsv
17	launch.appNotFound		lang	The app wasn’t found	win11-launch (the GIO launcher helper): its answer goes back to the host that launched the app; no full stop in nl
20	launch.missingUri		lang	The file address is missing	a "default" launch without a uri
21	launch.unknownRequest		lang	Unknown launch request	
```

## host/src/apps/pertype.rs (stem apps)

```tsv
4	apps.invalidFileType		lang	Invalid file type.	Default Apps by file type: the MIME type argument is not valid
16	apps.schemeLinks	scheme	lang	{scheme} links	Default Apps: the name of a link type without a description of its own, e.g. "mailto-koppelingen"; {scheme} is the URI scheme (mailto, https …)
30	apps.invalidInput		lang	Invalid input.	
33	apps.invalidSearch		lang	Invalid search term.	Default Apps: the search box text is not a string of at most 200 characters
54	apps.invalidApp		lang	Invalid app.	
56	apps.appNotFound		lang	This app wasn’t found.	
61	apps.defaultNotChanged		lang	Couldn’t change the default app.	
63	apps.defaultNotChanged		lang	Couldn’t change the default app.	
```

## host/src/apps/defaults.rs (stem apps)

```tsv
68	apps.invalidPath		lang	Invalid path.	Open with: the path is not absolute
70	apps.fileGone		lang	This file doesn’t exist any more.	Open with: the file vanished
76	apps.explorerName		lang	File Explorer	Open with a folder: Win11's own Explorer, first in the list (its name as the UI and catalog-data.json's ownApps show it)
105	apps.unknownDefaultKind		lang	Unknown kind of default app.	Settings › Default apps: not browser, mail, files, music, video, image, text or terminal
108	apps.invalidApp		lang	Invalid app.	
112	apps.cantBeDefault		lang	This app can’t be chosen as the default here.	one of Win11's own apps for a kind it does not handle (only File Explorer for folders)
116	apps.somethingWentWrong		lang	Something went wrong. Try again.	setting the default terminal failed
118	apps.explorerNotInstalled		lang	File Explorer isn’t in the app menu yet. Run install.sh again, then try again.	win11shell-explorer.desktop is missing; install.sh is the installer script's file name
121	apps.appNotFound		lang	This app wasn’t found.	
124	apps.defaultNotChanged		lang	Couldn’t change the default app.	
130	apps.defaultNotChanged		lang	Couldn’t change the default app.	
```

## host/src/apps/mod.rs (stem apps)

```tsv
16	apps.shuttingDown		lang	The app service is shutting down.	an apps.* call arrived while this window's app service shuts down
18	apps.invalidInput		lang	Invalid input.	
33	apps.invalidApp		lang	Invalid app.	apps.launch: the id is empty, too long or contains NUL
38	apps.linkCantOpen		lang	This link can’t be opened.	apps.openUri without a uri
40	apps.invalidInput		lang	Invalid input.	
42	apps.invalidInput		lang	Invalid input.	
45	apps.invalidInput		lang	Invalid input.	
```

## host/src/apps/recent.rs (stem apps)

```tsv
71	apps.historyNotCleared		lang	Couldn’t clear the history.	Start › Recommended: clearing the recent-files list failed (its backup could not be written)
```

Hand steps after the plans: the two inline pairs (`map apps.schemeLinks --from host/src/apps/pertype.rs#15 --en --spec {s}=scheme`,
`map apps.explorerName --from host/src/apps/defaults.rs#75 --en`), and `map_err(|_| &tr(…))` → `map_err(|_| tr(…))` (4 sites).
