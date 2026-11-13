// SPDX-License-Identifier: MIT
// GNOME bridge only. The native GTK 4 shell (win11-shell) draws the taskbar, Start and the desktop; the apps
// (Explorer, Settings, Task Manager, ...) are their own processes. The St bar below is the fallback without it.
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {NativePanelBridge} from './panelBridge.js';
import {ShortcutBridge} from './shortcutBridge.js';

const UUID = 'win11-gnome@win11shell.local';
// An old Start entry may remain from a WebKit-era install: never list it as a running app in the fallback bar.
const START = 'io.github.win11shell.Start.desktop';
const EXPLORER = 'io.github.win11shell.Explorer.desktop';
const HEIGHT = 48;
// How long, at enable (login), GNOME's top bar and the fallback bar wait for the taskbar process (D-G67).
const PANEL_PENDING_MS = 6000;
const mapId = id => ({'win11shell-explorer.desktop': EXPLORER,
    'win11shell-settings.desktop': 'org.gnome.Settings.desktop'}[id] ?? id);
const CONFIG_NAME = 'win11-gnome';
const SETTINGS_NAME = 'settings.json';

// i18n: pairs (tools/i18n/check_pairs.py). The fallback taskbar's own texts in Win11's language
// (notes/claude/contracts.md C1, C3). nl is byte-identical to 3b898e6 except the rows listed in
// tests/i18n/BASELINE-CHANGES.ext; en is Windows 11 en-GB.
const TEXT = {
    start: {nl: 'Start', en: 'Start'},
    search: {nl: 'Zoeken', en: 'Search'},
    searchLabel: {nl: '⌕   Zoeken', en: '⌕   Search'},
    taskView: {nl: 'Taakweergave', en: 'Task view'},
    system: {nl: 'Netwerk, geluid en snelle instellingen', en: 'Network, sound and quick settings'},
    // en: the window title and the desktop entry's name (cross.md item 9)
    group: {nl: 'Win11-taakbalk', en: 'Win11 Taskbar'},
    product: {nl: 'Win11 voor GNOME', en: 'Win11 for GNOME'},
    appMissing: {nl: 'Deze app is nog niet geïnstalleerd.', en: 'This app isn’t installed yet.'},
    // GLib.DateTime format of the clock's second line: nl dd-mm-yyyy, en-GB dd/mm/yyyy (Windows 11).
    clockDate: {nl: '%d-%m-%Y', en: '%d/%m/%Y'},
};

/** Win11's language from the text of its settings.json: "en", or else "nl" (C1: missing, unknown, object form). */
export function languageOf(text) {
    try {
        return JSON.parse(text)?.language === 'en' ? 'en' : 'nl';
    } catch {
        return 'nl';
    }
}

/** Task Manager's "Always on top" from the parsed settings.json (D-G21): exactly true, anything else is off. */
export function alwaysOnTopOf(settings) {
    return settings?.taskmgr?.alwaysOnTop === true;
}

/** Win11's own language setting, followed live (C1). It reads only Win11's settings.json in the user's
 * config directory (never the GNOME locale, WIN11_REFERENCE_SETTINGS or H's ~/.config/win11shell) and creates
 * no files. The settings store replaces the file by rename, hence WATCH_MOVES. Until a first setting is saved
 * the directory may not exist: then the parent directory is watched for it. Every read also keeps the parsed
 * document (`settings`, {} when missing or unreadable) and hands it to `onSettings` (D-G21: Task Manager's
 * always-on-top follows the same file live). */
export class LanguageWatch {
    constructor(onChange = () => {}, onSettings = () => {}) {
        this._onChange = onChange;
        this._onSettings = onSettings;
        this._dir = Gio.File.new_for_path(GLib.build_filenamev([GLib.get_user_config_dir(), CONFIG_NAME]));
        this._file = this._dir.get_child(SETTINGS_NAME);
        this._dirMonitor = this._parentMonitor = null;
        this._queueId = 0;
        this.settings = {};
        this.language = this._read();
        this._watchDir();
    }

    _read() {
        this.settings = {};
        try {
            const [ok, bytes] = this._file.load_contents(null);
            if (ok) {
                const text = new TextDecoder().decode(bytes);
                try {
                    const parsed = JSON.parse(text);
                    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) this.settings = parsed;
                } catch { /* Not JSON: no settings, nl (C1). */ }
                return languageOf(text);
            }
        } catch { /* No file yet, or unreadable: nl (C1). */ }
        return 'nl';
    }

    _monitor(directory, handler) {
        try {
            const monitor = directory.monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, null);
            monitor.connect('changed', handler);
            return monitor;
        } catch (error) {
            console.warn(`Win11 language monitor: ${error.message}`);
            return null;
        }
    }

    _watchDir() {
        this._dirMonitor?.cancel();
        this._dirMonitor = this._monitor(this._dir, (_monitor, file, other) => {
            if ([file, other].some(item => item && (item.get_basename() === SETTINGS_NAME || item.equal(this._dir))))
                this._queue();
        });
        this._syncParent();
    }

    _syncParent() {
        const exists = this._dir.query_file_type(Gio.FileQueryInfoFlags.NONE, null) === Gio.FileType.DIRECTORY;
        if (exists && this._parentMonitor) {
            this._parentMonitor.cancel();
            this._parentMonitor = null;
        } else if (!exists && !this._parentMonitor) {
            const parent = this._dir.get_parent();
            this._parentMonitor = parent && this._monitor(parent, (_monitor, file, other) => {
                if ([file, other].some(item => item?.equal(this._dir))) {
                    this._watchDir();
                    this._queue();
                }
            });
        }
    }

    /** Debounce (ge1c-review m4): every event restarts the 100 ms timer, so a writer that rewrites the file in
     * place in steps less than 100 ms apart is read once, after its last event, not half-written. The store's
     * rename is a single event, so its switch still takes 100 ms. */
    _queue() {
        if (this._queueId) GLib.source_remove(this._queueId);
        this._queueId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
            this._queueId = 0;
            this._syncParent();
            const language = this._read();
            if (language !== this.language) {
                this.language = language;
                try { this._onChange(language); } catch (error) { console.error(error); }
            }
            try { this._onSettings(this.settings); } catch (error) { console.error(error); }
            return GLib.SOURCE_REMOVE;
        });
    }

    destroy() {
        if (this._queueId) GLib.source_remove(this._queueId);
        this._queueId = 0;
        this._dirMonitor?.cancel();
        this._parentMonitor?.cancel();
        this._dirMonitor = this._parentMonitor = null;
        this._onChange = this._onSettings = () => {};
    }
}

const TASK_MANAGER = 'io.github.win11shell.TaskManager';

/** D-G21: Task Manager's "Always on top" (taskmgr.alwaysOnTop in Win11's settings.json, followed live through
 * LanguageWatch). While it is on, every window with Task Manager's WM class is kept above other windows
 * (make_above) and on every workspace (stick); switched off, or when the extension is disabled, each window gets
 * back the above and sticky states it had before. Only the public Meta.Window API of GNOME Shell 50. */
export class TaskManagerOnTop {
    constructor() {
        this._enabled = false;
        this._on = false;
        this._windows = new Map();
        this._signals = [];
    }

    enable(on) {
        this._enabled = true;
        this._on = Boolean(on);
        this._signals = [global.display.connect('window-created', (_display, window) => this._watch(window))];
        for (const window of global.display.list_all_windows()) this._watch(window);
    }

    /** Follow the setting (called on every read of settings.json). */
    set(on) {
        on = Boolean(on);
        if (!this._enabled || on === this._on) return;
        this._on = on;
        for (const window of this._windows.keys()) this._apply(window);
    }

    _isTaskManager(window) {
        return window.get_wm_class() === TASK_MANAGER;
    }

    _watch(window) {
        if (!window || !this._enabled || this._windows.has(window)) return;
        const entry = {signals: [], original: null};
        this._windows.set(window, entry);
        // On Wayland the WM class (the GTK program class) can arrive after the window was created.
        entry.signals.push(window.connect('notify::wm-class', () => this._apply(window)));
        entry.signals.push(window.connect('shown', () => this._apply(window)));
        entry.signals.push(window.connect('unmanaged', () => this._unwatch(window)));
        this._apply(window);
    }

    _unwatch(window) {
        const entry = this._windows.get(window);
        if (!entry) return;
        for (const id of entry.signals) window.disconnect(id);
        this._windows.delete(window);
    }

    _apply(window) {
        const entry = this._windows.get(window);
        if (!entry || !this._enabled) return;
        const want = this._on && this._isTaskManager(window);
        if (want && !entry.original) {
            entry.original = {above: window.is_above(), sticky: window.is_on_all_workspaces()};
            if (!window.is_above()) window.make_above();
            if (!window.is_on_all_workspaces()) window.stick();
        } else if (!want && entry.original) {
            const original = entry.original;
            entry.original = null;
            if (!original.above && window.is_above()) window.unmake_above();
            if (!original.sticky) window.unstick();
        }
    }

    disable() {
        this._on = false;
        for (const window of [...this._windows.keys()]) {
            this._apply(window);
            this._unwatch(window);
        }
        for (const id of this._signals) global.display.disconnect(id);
        this._signals = [];
        this._enabled = false;
    }
}

/** GNOME's and Tiling Assistant's window snap preview (.tile-preview) in the Win11 accent rather than the GNOME
 *  accent (Ubuntu's orange): a generated stylesheet in the shell theme, renewed when the accent or the shell theme
 *  changes (D-G69, Robin 13-11: "de snap preview pane is nu nog oranje, kan het meegaan met accent kleur?"). */
class AccentPreview {
    set(settings) {
        const accent = /^#[0-9a-f]{6}$/i.test(settings?.theme?.accent ?? '') ? settings.theme.accent.toLowerCase() : '#0078d4';
        const theme = St.ThemeContext.get_for_stage(global.stage).get_theme();
        // load_stylesheet emits the theme context's 'changed', which calls here again: not while loading.
        if (this._loading || (accent === this._accent && theme === this._theme)) return;
        const [r, g, b] = [1, 3, 5].map(i => parseInt(accent.slice(i, i + 2), 16));
        const css = `.tile-preview { background-color: rgba(${r},${g},${b},0.28); border: 1px solid rgba(${r},${g},${b},0.85);` +
            ` border-radius: 8px; }\n`;
        try {
            const dir = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'win11-gnome']);
            GLib.mkdir_with_parents(dir, 0o700);
            const file = Gio.File.new_for_path(GLib.build_filenamev([dir, `tile-preview-${accent.slice(1)}.css`]));
            file.replace_contents(new TextEncoder().encode(css), null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
            this._loading = true;
            if (this._file && this._theme === theme) theme.unload_stylesheet(this._file);
            theme.load_stylesheet(file);
            this._file = file;
            this._theme = theme;
            this._accent = accent;
        } catch (error) {
            console.warn(`Win11 snap preview accent: ${error.message}`);
        } finally {
            this._loading = false;
        }
    }

    disable() {
        if (this._file && this._theme) {
            try { this._theme.unload_stylesheet(this._file); } catch { /* the theme went with the shell */ }
        }
        this._file = this._theme = this._accent = null;
    }
}

function readSettings() {
    const override = GLib.getenv('WIN11_REFERENCE_SETTINGS');
    const paths = [override, `${GLib.get_user_config_dir()}/win11-gnome/settings.json`,
        `${GLib.get_home_dir()}/.config/win11shell/settings.json`].filter(Boolean);
    for (const path of paths) {
        try {
            const [ok, bytes] = Gio.File.new_for_path(path).load_contents(null);
            if (ok) return JSON.parse(new TextDecoder().decode(bytes));
        } catch { /* Optional read-only source; fall back to defaults. */ }
    }
    return {};
}

export default class Win11Taskbar extends Extension {
    enable() {
        this._destroyed = false;
        this._signals = [];
        this._windowSignals = [];
        this._buttons = [];
        this._refreshId = 0;
        this._layoutId = 0;
        this._searchButton = null;
        this._nativePanelActive = false;
        this._topPanelOverview = Main.overview.visible;
        this._overviewHiddenAt = 0;
        this._topPanelState = null;
        this._syncingTopPanel = false;
        this._topPanelActor = Main.layoutManager.panelBox;
        this._topPanelVisibleId = 0;
        this._topPanelDestroyId = 0;
        this._taskManagerOnTop = new TaskManagerOnTop();
        this._accentPreview = new AccentPreview();
        this._language = new LanguageWatch(() => this._relabel(),
            settings => {
                this._taskManagerOnTop?.set(alwaysOnTopOf(settings));
                this._accentPreview?.set(settings);
            });
        this._accentPreview.set(this._language.settings);
        this._taskManagerOnTop.enable(alwaysOnTopOf(this._language.settings));
        this._settings = readSettings();
        this._bar = new St.Widget({style_class: 'win11-taskbar', reactive: true,
            height: HEIGHT, layout_manager: new Clutter.FixedLayout()});
        this._bar.connect('destroy', () => { this._destroyed = true; });
        if (this._settings.theme?.mode === 'light') this._bar.add_style_class_name('light');
        this._centre = new St.BoxLayout({style_class: 'win11-taskbar-centre',
            x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER});
        this._bar.add_child(this._centre);
        // Let Clutter align the allocated group itself, including after scaling or icon changes.
        this._centre.add_constraint(new Clutter.AlignConstraint({
            source: this._bar, align_axis: Clutter.AlignAxis.X_AXIS, factor: 0.5,
        }));
        const startIcon = Gio.File.new_for_path(`${this.path}/start.svg`);
        const start = this._button(this._t('start'), new St.Icon({gicon: new Gio.FileIcon({file: startIcon}), icon_size: 24}));
        // Without win11-shell there is no Start: the fallback opens GNOME's app grid, Search its overview.
        start.connect('clicked', () => this._showApps());
        this._startButton = start;
        this._centre.add_child(start);
        if (this._settings.taskbar?.search !== 'hidden') {
            const search = this._button(this._t('search'), new St.Label({text: this._t('searchLabel'), y_align: Clutter.ActorAlign.CENTER}));
            search.add_style_class_name('win11-task-search');
            this._searchButton = search;
            search.connect('clicked', () => Main.overview.toggle());
            this._centre.add_child(search);
        }
        if (this._settings.taskbar?.taskView !== false) {
            const view = this._button(this._t('taskView'), new St.Icon({icon_name: 'view-grid-symbolic', icon_size: 20}));
            view.connect('clicked', () => Main.overview.toggle());
            this._viewButton = view;
            this._centre.add_child(view);
        }
        this._running = new St.BoxLayout({style_class: 'win11-running'});
        this._runningViewport = new St.ScrollView({
            style_class: 'win11-app-viewport',
            hscrollbar_policy: St.PolicyType.EXTERNAL,
            vscrollbar_policy: St.PolicyType.NEVER,
            enable_mouse_scrolling: false,
            clip_to_allocation: true,
        });
        this._runningViewport.add_child(this._running);
        this._runningViewport.connect('scroll-event', (_actor, event) => {
            const direction = event.get_scroll_direction();
            let delta = 0;
            if (direction === Clutter.ScrollDirection.SMOOTH) {
                const [dx, dy] = event.get_scroll_delta();
                delta = (Math.abs(dx) > Math.abs(dy) ? dx : dy) * 44;
            } else if (direction === Clutter.ScrollDirection.UP || direction === Clutter.ScrollDirection.LEFT) {
                delta = -44;
            } else if (direction === Clutter.ScrollDirection.DOWN || direction === Clutter.ScrollDirection.RIGHT) {
                delta = 44;
            }
            this._scrollApps(delta);
            return Clutter.EVENT_STOP;
        });
        this._centre.add_child(this._runningViewport);
        const right = new St.BoxLayout({style_class: 'win11-tray', x_align: Clutter.ActorAlign.END,
            y_align: Clutter.ActorAlign.CENTER});
        this._tray = right;
        this._tray.add_constraint(new Clutter.AlignConstraint({
            source: this._bar, align_axis: Clutter.AlignAxis.X_AXIS, factor: 1,
        }));
        // Keep access to GNOME's real system controls; don't duplicate system services.
        const system = this._button(this._t('system'),
            new St.Icon({icon_name: 'preferences-system-symbolic', icon_size: 16}));
        system.connect('clicked', () => Main.panel.statusArea.quickSettings?.menu.toggle());
        this._systemButton = system;
        right.add_child(system);
        this._clock = new St.Label({style_class: 'win11-clock', y_align: Clutter.ActorAlign.CENTER});
        right.add_child(this._clock);
        this._bar.add_child(right);
        this._updateClock();
        this._clockId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
            if (this._destroyed) return GLib.SOURCE_REMOVE;
            this._updateClock(); return GLib.SOURCE_CONTINUE;
        });
        Main.layoutManager.addChrome(this._bar, {affectsStruts: true, trackFullscreen: true});
        Main.ctrlAltTabManager.addGroup(this._bar, this._t('group'), 'view-app-grid-symbolic');
        this._connect(Main.layoutManager, 'monitors-changed', () => { this._position(); this._positionStart(); });
        const theme = St.ThemeContext.get_for_stage(global.stage);
        this._connect(theme, 'changed', () => {
            this._queueLayout();
            this._accentPreview?.set(this._language?.settings);
        });
        this._connect(theme, 'notify::scale-factor', () => this._queueLayout());
        this._connect(Main.overview, 'showing', () => {
            this._topPanelOverview = true;
            this._bar.hide();
            this._syncTopPanel();
        });
        this._connect(Main.overview, 'hidden', () => {
            this._topPanelOverview = false;
            this._overviewHiddenAt = GLib.get_monotonic_time();
            if (!this._nativePanelActive && !this._panelPending) this._bar.show();
            this._syncTopPanel();
        });
        this._connect(Main.sessionMode, 'updated', () => this._syncTopPanel());
        this._connect(global.display, 'in-fullscreen-changed', () => this._syncTopPanel());
        this._topPanelVisibleId = this._topPanelActor.connect('notify::visible', () => this._syncTopPanel());
        this._topPanelDestroyId = this._topPanelActor.connect('destroy', () => {
            this._topPanelActor = this._topPanelState = null;
            this._topPanelVisibleId = this._topPanelDestroyId = 0;
        });
        this._connect(Shell.AppSystem.get_default(), 'app-state-changed', () => this._queueRefresh());
        // win11-panel rewrites the own entries' Name= after a language switch (C6): re-read the app names.
        this._connect(Shell.AppSystem.get_default(), 'installed-changed', () => this._queueRefresh());
        this._connect(Shell.WindowTracker.get_default(), 'tracked-windows-changed', () => this._queueRefresh());
        this._connect(global.display, 'notify::focus-window', () => this._updateFocus());
        this._connect(global.workspace_manager, 'active-workspace-changed', () => this._queueRefresh());
        this._connect(global, 'shutdown', () => this.disable());
        this._position(); this._refresh();
        this._fallbackTracked = true;
        // D-G67: at login GNOME's own top bar and our fallback bar showed for a moment before the taskbar process
        // reported (Robin, 13-11: "als ik aanmeld zie ik heel even de oude gnome"). For up to PANEL_PENDING_MS both
        // stay hidden while the taskbar starts; only a taskbar that does not come gets the fallback.
        this._panelPending = true;
        this._panelPendingId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PANEL_PENDING_MS, () => {
            this._panelPendingId = 0;
            this._endPanelPending();
            return GLib.SOURCE_REMOVE;
        });
        const language = () => this._language?.language ?? 'nl';
        this._panelBridge = new NativePanelBridge({getStartAlignment: () => readSettings().taskbar?.alignment,
            language, onPanelChanged: present => {
            this._nativePanelActive = present;
            if (present) this._endPanelPending();
            if (!this._bar || this._destroyed) return;
            if (present && this._fallbackTracked) {
                Main.layoutManager.untrackChrome(this._bar);
                this._fallbackTracked = false;
            } else if (!present && !this._fallbackTracked) {
                Main.layoutManager.trackChrome(this._bar, {affectsStruts: true, trackFullscreen: true});
                this._fallbackTracked = true;
            }
            if (present || Main.overview.visible || this._panelPending) this._bar.hide(); else this._bar.show();
            this._syncTopPanel();
        }});
        this._panelBridge.enable();
        this._shortcuts = new ShortcutBridge(this, () => this._startKey(), () => this._panelBridge?._openTaskView(), language);
        this._shortcuts.enable();
        const nativePanelApp=Shell.AppSystem.get_default().lookup_app('io.github.win11shell.Panel.desktop');
        if (nativePanelApp) nativePanelApp.open_new_window(-1);

        if (Main.overview.visible || this._panelPending) this._bar.hide();
        this._syncTopPanel();
    }

    _endPanelPending() {
        if (this._panelPendingId) {
            GLib.source_remove(this._panelPendingId);
            this._panelPendingId = 0;
        }
        if (!this._panelPending) return;
        this._panelPending = false;
        if (this._bar && !this._destroyed && !this._nativePanelActive && !Main.overview.visible) this._bar.show();
        this._syncTopPanel();
    }

    _connect(object, signal, handler) { this._signals.push([object, object.connect(signal, handler)]); }

    _syncTopPanel() {
        if (this._syncingTopPanel || this._destroyed || !this._topPanelActor)
            return;
        const userSession = Main.sessionMode.hasWindows &&
            !Main.sessionMode.isLocked && !Main.sessionMode.isGreeter;
        if ((!this._nativePanelActive && !this._panelPending) || !userSession) {
            this._restoreTopPanel();
            return;
        }
        const actor = this._topPanelActor;
        if (!this._topPanelState) {
            // GNOME 50 reserves struts even for hidden chrome. Preserve its tracking
            // record rather than untracking the panel and losing fullscreen handling.
            const tracking = Main.layoutManager._trackedActors.find(item => item.actor === actor);
            this._topPanelState = {actor, visible: actor.visible, tracking,
                affectsStruts: tracking?.affectsStruts};
        }
        const state = this._topPanelState;
        this._syncingTopPanel = true;
        try {
            actor.visible = this._topPanelOverview && state.visible;
            if (state.tracking) {
                state.tracking.affectsStruts = this._topPanelOverview ? state.affectsStruts : false;
                Main.layoutManager._queueUpdateRegions();
            }
        } finally {
            this._syncingTopPanel = false;
        }
    }

    _restoreTopPanel() {
        const state = this._topPanelState;
        if (!state)
            return;
        this._topPanelState = null;
        this._syncingTopPanel = true;
        try {
            // A panel that was visible before takeover must still respect fullscreen.
            const monitor = Main.layoutManager.primaryMonitor;
            const fullscreen = !this._topPanelOverview && monitor &&
                global.display.get_monitor_in_fullscreen(monitor.index);
            state.actor.visible = state.visible && !fullscreen;
            if (state.tracking) {
                state.tracking.affectsStruts = state.affectsStruts;
                Main.layoutManager._queueUpdateRegions();
            }
        } finally {
            this._syncingTopPanel = false;
        }
    }

    _button(title, child) {
        return new St.Button({style_class: 'win11-task-button', can_focus: true, accessible_name: title, child});
    }

    _t(key) {
        const pair = TEXT[key];
        return pair[this._language?.language ?? 'nl'] ?? pair.nl;
    }

    /** Re-label everything this bar shows after a language switch (C1); app names are GNOME's own. */
    _relabel() {
        if (this._destroyed || !this._bar) return;
        this._startButton.accessible_name = this._t('start');
        if (this._searchButton) this._searchButton.accessible_name = this._t('search');
        if (this._viewButton) this._viewButton.accessible_name = this._t('taskView');
        this._systemButton.accessible_name = this._t('system');
        // CtrlAltTabManager keeps the name it was given; re-register the same root under the new name. GNOME's
        // addGroup() connects a 'destroy' handler on the root that removeGroup() never disconnects, so each switch
        // leaves one closure on the bar until it is destroyed; it only calls the idempotent removeGroup()
        // (ctrlAltTab.js, GNOME 50; ge1c-review m5). Harmless, and smaller than reaching into _items.
        Main.ctrlAltTabManager.removeGroup(this._bar);
        Main.ctrlAltTabManager.addGroup(this._bar, this._t('group'), 'view-app-grid-symbolic');
        this._updateClock();
        this._queueRefresh();
        this._queueLayout();
    }

    _showApps() {
        if (Main.overview.showApps) Main.overview.showApps();
        else Main.overview.toggle();
    }

    /** Super: win11-shell's Start. With the fallback bar (no win11-shell) GNOME's app grid, as its Start button;
     * a Super that just closed the overview (ShortcutBridge hides it first) does not open it again. */
    _startKey() {
        try {
            this._panelBridge?._toggleStart();
        } catch (error) {
            const justHidden = GLib.get_monotonic_time() - (this._overviewHiddenAt ?? 0) < GLib.USEC_PER_SEC / 4;
            if (error.code === 'not_found' && !this._nativePanelActive && !this._panelPending) {
                if (!justHidden) this._showApps();
            } else {
                console.warn(`Win11 Start: ${error.message}`);
            }
        }
    }

    _launch(id) {
        const app = Shell.AppSystem.get_default().lookup_app(id);
        if (app) app.activate();
        else Main.notify(this._t('product'), this._t('appMissing'));
    }

    _position() {
        if (this._destroyed || !this._bar) return;
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor) return;
        const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        this._bar.set_height(HEIGHT * scale);
        this._bar.set_position(monitor.x, monitor.y + monitor.height - this._bar.height);
        this._bar.set_width(monitor.width);
        this._layout();
    }

    _queueLayout() {
        if (this._layoutId || this._destroyed || !this._bar) return;
        this._layoutId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._layoutId = 0;
            this._position();
            this._positionStart();
            return GLib.SOURCE_REMOVE;
        });
    }

    _layout() {
        if (this._destroyed || !this._bar) return;
        const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        // Reserve the tray's actual width symmetrically. The centre remains the monitor's
        // centre even when a small screen needs a scrollable app strip.
        this._tray.set_width(-1);
        const trayWidth = Math.max(138 * scale, this._tray.get_preferred_width(-1)[1]);
        this._tray.set_width(trayWidth);
        this._tray.set_y(4 * scale);
        this._tray.set_height(40 * scale);
        const children = this._centre.get_children();
        const fixedWidth = () => children.filter(child => child !== this._runningViewport)
            .reduce((width, child) => width + child.get_preferred_width(-1)[1], 0) +
            Math.max(0, children.length - 1) * this._centre.get_theme_node().get_length('spacing');
        const safeWidth = Math.max(0, this._bar.width - 2 * (trayWidth + 16 * scale));
        if (this._searchButton) {
            this._searchButton.remove_style_class_name('compact');
            this._searchButton.child.text = this._t('searchLabel');
            if (fixedWidth() + 40 * scale > safeWidth) {
                this._searchButton.add_style_class_name('compact');
                this._searchButton.child.text = '⌕';
            }
        }
        const available = Math.max(0, safeWidth - fixedWidth());
        this._runningViewport.set_width(Math.min(available, this._running.get_preferred_width(-1)[1]));
        this._runningViewport.set_height(40 * scale);
        this._centre.set_width(-1);
        this._centre.set_y(4 * scale);
        this._centre.set_height(40 * scale);
    }

    _scrollApps(delta) {
        const adjustment = this._runningViewport.hadjustment;
        const page = adjustment.page_size ?? adjustment.pageSize;
        adjustment.value = Math.max(adjustment.lower,
            Math.min(adjustment.upper - page, adjustment.value + delta));
    }

    _revealButton(button) {
        const [x] = button.get_transformed_position();
        const [left] = this._runningViewport.get_transformed_position();
        if (x < left) this._scrollApps(x - left);
        else if (x + button.width > left + this._runningViewport.width)
            this._scrollApps(x + button.width - left - this._runningViewport.width);
    }

    _positionStart() {
        if (this._panelBridge) {
            this._panelBridge.positionStarts();
            return;
        }
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor) return;
        for (const actor of global.get_window_actors()) {
            const window = actor.meta_window;
            if (window.get_wm_class() !== 'io.github.win11shell.Start') continue;
            const width = Math.min(642, monitor.width - 24);
            const height = Math.min(726, monitor.height - this._bar.height - 24);
            window.move_resize_frame(false, monitor.x + Math.round((monitor.width - width) / 2),
                monitor.y + monitor.height - this._bar.height - height - 12, width, height);
        }
    }

    _queueRefresh() {
        if (this._refreshId || this._destroyed || !this._bar) return;
        this._refreshId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._refreshId = 0; this._refresh(); return GLib.SOURCE_REMOVE;
        });
    }

    _refresh() {
        if (this._destroyed || !this._bar) return;
        for (const [window, id] of this._windowSignals) window.disconnect(id);
        this._windowSignals = [];
        this._running.destroy_all_children(); this._buttons = [];
        const system = Shell.AppSystem.get_default();
        const workspace = global.workspace_manager.get_active_workspace();
        const ids = (this._settings.taskbar?.pinned ?? [EXPLORER, 'firefox.desktop', 'org.gnome.Settings.desktop']).map(mapId);
        const pinned = [...new Set(ids)].map(id => system.lookup_app(id)).filter(Boolean);
        const pinnedIds = new Set(pinned.map(app => app.get_id()));
        const running = system.get_running().filter(app => app.get_id() !== START && !pinnedIds.has(app.get_id()))
            .sort((a, b) => a.get_name().localeCompare(b.get_name()));
        for (const app of [...pinned, ...running]) {
            const all = app.get_windows().filter(w => !w.skip_taskbar);
            for (const window of all) this._windowSignals.push([window, window.connect('workspace-changed', () => this._queueRefresh())]);
            const windows = all.filter(w => w.located_on_workspace(workspace)).sort((a,b) => a.get_stable_sequence() - b.get_stable_sequence());
            if (!windows.length && !pinnedIds.has(app.get_id())) continue;
            const content = new St.Widget({style_class: 'win11-app-content',
                layout_manager: new Clutter.FixedLayout()});
            const icon = app.create_icon_texture(24);
            content.add_child(icon);
            icon.add_constraint(new Clutter.AlignConstraint({
                source: content, align_axis: Clutter.AlignAxis.BOTH, factor: 0.5,
            }));
            if (windows.length) {
                const indicator = new St.Widget({style_class: 'win11-running-indicator'});
                content.add_child(indicator);
                indicator.add_constraint(new Clutter.AlignConstraint({
                    source: content, align_axis: Clutter.AlignAxis.X_AXIS, factor: 0.5,
                }));
                indicator.add_constraint(new Clutter.AlignConstraint({
                    source: content, align_axis: Clutter.AlignAxis.Y_AXIS, factor: 1,
                }));
            }
            const button = this._button(app.get_name(), content);
            button.connect('key-focus-in', () => this._revealButton(button));
            if (windows.length) button.add_style_class_name('win11-app-running');
            button.connect('clicked', () => {
                if (!windows.length) { app.activate(); return; }
                const focused = global.display.focus_window;
                if (windows.length === 1 && focused === windows[0] && !focused.minimized) { focused.minimize(); return; }
                Main.activateWindow(windows[(windows.indexOf(focused) + 1) % windows.length]);
            });
            this._running.add_child(button); this._buttons.push({button, windows});
        }
        this._updateFocus(); this._position(); this._positionStart();
    }

    _updateFocus() {
        if (this._destroyed || !this._bar) return;
        const focused = global.display.focus_window;
        for (const {button, windows} of this._buttons) {
            if (windows.includes(focused)) button.add_style_pseudo_class('active');
            else button.remove_style_pseudo_class('active');
        }
    }

    _updateClock() {
        const seconds = this._settings.time?.showSeconds ?? false;
        this._clock.text = GLib.DateTime.new_now_local().format(`${seconds ? '%H:%M:%S' : '%H:%M'}\n${this._t('clockDate')}`);
    }

    disable() {
        this._shortcuts?.disable();
        this._shortcuts = null;
        this._nativePanelActive = false;
        this._panelBridge?.disable();
        this._panelBridge = null;
        this._language?.destroy();
        this._language = null;
        this._taskManagerOnTop?.disable();
        this._taskManagerOnTop = null;
        if (this._refreshId) GLib.source_remove(this._refreshId);
        if (this._layoutId) GLib.source_remove(this._layoutId);
        if (this._clockId) GLib.source_remove(this._clockId);
        this._refreshId = this._clockId = this._layoutId = 0;
        for (const [object,id] of this._signals ?? []) object.disconnect(id);
        for (const [window,id] of this._windowSignals ?? []) window.disconnect(id);
        this._signals = []; this._windowSignals = [];
        this._restoreTopPanel();
        if (this._topPanelActor) {
            if (this._topPanelVisibleId) this._topPanelActor.disconnect(this._topPanelVisibleId);
            if (this._topPanelDestroyId) this._topPanelActor.disconnect(this._topPanelDestroyId);
        }
        this._topPanelActor = null;
        this._topPanelVisibleId = this._topPanelDestroyId = 0;
        if (this._bar && !this._destroyed) {
            Main.ctrlAltTabManager.removeGroup(this._bar);
            Main.layoutManager.removeChrome(this._bar); this._bar.destroy();
        }
        this._accentPreview?.disable();
        this._accentPreview = null;
        if (this._panelPendingId) GLib.source_remove(this._panelPendingId);
        this._panelPendingId = 0;
        this._panelPending = false;
        this._bar = this._running = this._runningViewport = this._clock = this._tray = this._searchButton = null; this._buttons = [];
        this._startButton = this._viewButton = this._systemButton = null;
    }
}
