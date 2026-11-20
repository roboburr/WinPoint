// SPDX-License-Identifier: MIT
// Native GNOME integration for the native GTK 4 shell (win11-shell: taskbar, Start and desktop in one process).
// This module never reparents or paints a compositor-owned Meta.WindowActor.
import Gio from 'gi://Gio';
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Mtk from 'gi://Mtk';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {NativeDesktopBridge, DESKTOP_APP_ID, secondaryDesktop} from './desktopBridge.js';
import {NativeNotificationsBridge} from './notificationsBridge.js';
import {NativeFrostedBridge} from './frostedBridge.js';
import {LaunchBridge} from './launchBridge.js';
import {DisplayBridge} from './displayBridge.js';
import {InputBridge} from './inputBridge.js';

export const BRIDGE_NAME = 'io.github.win11shell.GnomeBridge';
export const BRIDGE_PATH = '/io/github/win11shell/GnomeBridge';
export const PANEL_APP_ID = 'io.github.win11shell.Panel';
export const START_APP_ID = 'io.github.win11shell.Start';
// The taskbars of the other monitors (Windows 11's "Show my taskbar on all displays"): win11-shell makes one window per
// monitor without the main taskbar, with this app ID and the monitor's index (native/shell/src/panel/strips.rs).
export const STRIP_APP_ID = 'io.github.win11shell.SecondaryTaskbar';
const STRIP_ID = /^io\.github\.win11shell\.SecondaryTaskbar\.Monitor(\d{1,2})$/;
const HEIGHT = 48;
// Over a fullscreen app (Windows 11): how long Super or Alt+Tab keeps the taskbar shown before Start or the task
// view has the focus, and how long a window chosen in the task view has to arrive before Esc's focus return.
const REVEAL_MS = 2000;
const RETURN_MS = 400;
// How long after a launch from Start or the taskbar the app's window may take the focus it asks for (D-G61).
const LAUNCH_GRANT_MS = 10000;
// The power actions of the UI's power.js (ACTIONS) that Start hands to the panel window (panel.power).
const POWER_ACTIONS = ['shutdown', 'reboot', 'logout', 'suspend', 'lock'];
// The file portal's picker (the native Explorer in picker mode, host/src/picker.rs): its own Wayland app id.
const PICKER_APP_ID = 'io.github.win11shell.FilePicker';
const USER_WINDOW_TYPES = new Set([
    Meta.WindowType.NORMAL, Meta.WindowType.DIALOG,
    Meta.WindowType.MODAL_DIALOG, Meta.WindowType.UTILITY,
]);
const INTERFACE = `<node><interface name="${BRIDGE_NAME}">
  <method name="Call">
    <arg name="method" type="s" direction="in"/>
    <arg name="argsJSON" type="s" direction="in"/>
    <arg name="resultJSON" type="s" direction="out"/>
  </method>
  <signal name="Event"><arg name="event" type="s"/><arg name="dataJSON" type="s"/></signal>
</interface></node>`;

function rectJSON(rect) {
    return {x: rect.x, y: rect.y, w: rect.width, h: rect.height};
}

function failure(code, message) {
    const error = new Error(message);
    error.code = code;
    throw error;
}

// i18n: pairs (tools/i18n/check_pairs.py). Texts this bridge returns or shows, in Win11's language
// (contracts C1, C3; plan-critique F4: the UI shows error.message in a toast). en is en-GB in Windows 11 wording
// (ge1c-review M1: 3b898e6's developer English is gone); the nl rows are listed in tests/i18n/BASELINE-CHANGES.ext.
// Codes never change; nothing matches on these texts (contracts C2).
const TEXT = {
    sessionStart: {nl: 'Start is alleen beschikbaar in de ontgrendelde gebruikerssessie.', en: 'Start is only available in the unlocked user session.'},
    startMissing: {nl: 'Win11 Start is niet geïnstalleerd.', en: 'Win11 Start isn’t installed.'},
    sessionApps: {nl: 'Apps kunnen alleen worden geopend in de ontgrendelde gebruikerssessie.', en: 'Apps can only be opened in the unlocked user session.'},
    appMissing: {nl: 'De gevraagde app is niet geïnstalleerd.', en: 'The requested app isn’t installed.'},
    panelMissing: {nl: 'De taakbalk is niet beschikbaar.', en: 'The taskbar isn’t available.'},
    keyboardMode: {nl: 'Er is een toetsenbordmodus nodig.', en: 'A keyboard mode is required.'},
    powerAction: {nl: 'Onbekende energieactie.', en: 'Unknown power action.'},
    sessionWindows: {nl: 'Vensterbeheer is alleen beschikbaar in de ontgrendelde gebruikerssessie.', en: 'Window control is only available in the unlocked user session.'},
    windowGone: {nl: 'Het gevraagde app-venster bestaat niet meer.', en: 'The requested app window no longer exists.'},
    argsObject: {nl: 'Argumenten moeten een JSON-object zijn.', en: 'Arguments must be a JSON object.'},
    offline: {nl: 'De GNOME-integratie staat uit.', en: 'The GNOME integration is turned off.'},
    explorerGone: {nl: 'Het oorspronkelijke Verkenner-venster is niet meer beschikbaar.', en: 'The original File Explorer window is no longer available.'},
    cantMinimize: {nl: 'Dit venster kan niet worden geminimaliseerd.', en: 'This window can’t be minimised.'},
    cantClose: {nl: 'Dit venster kan niet worden gesloten.', en: 'This window can’t be closed.'},
    cantMaximize: {nl: 'Dit venster kan niet worden gemaximaliseerd.', en: 'This window can’t be maximised.'},
    unknownMethod: {nl: 'Niet-ondersteunde GNOME-functie: {method}', en: 'Unsupported GNOME function: {method}'},
    badSnap: {nl: 'Onbekende uitlijnpositie voor het venster.', en: 'Unknown snap position for the window.'},
    cantResize: {nl: 'Het formaat van dit venster kan niet worden gewijzigd.', en: 'This window can’t be resized.'},
    sessionUnavailable: {nl: 'De gebruikerssessie is vergrendeld of niet beschikbaar.', en: 'The user session is locked or unavailable.'},
    badRects: {nl: 'Pictogramrechthoeken moeten een lijst zijn.', en: 'Icon rectangles must be a list.'},
    // Names for a launch context when an own .desktop file is missing: host/src/desktop-entries.json (C6),
    // "Alles" in both languages (D-G5).
    appExplorer: {nl: 'Win11 Verkenner', en: 'Win11 File Explorer'},
    appSettings: {nl: 'Win11 Instellingen', en: 'Win11 Settings'},
    appTaskManager: {nl: 'Win11 Taakbeheer', en: 'Win11 Task Manager'},
    appAlles: {nl: 'Alles', en: 'Alles'},
    appDisks: {nl: 'Win11 Schijfbeheer', en: 'Win11 Disk Management'},
    appDefaultApps: {nl: 'Win11 Standaardapps', en: 'Win11 Default apps'},
    appInstaller: {nl: 'Win11 App-installatie', en: 'Win11 App Installer'},
};

/** A text of TEXT in `language` (nl when it has none), with {name} placeholders filled from `params`. */
function text(key, language, params = {}) {
    const pair = TEXT[key];
    return (pair[language] ?? pair.nl).replace(/\{(\w+)\}/g, (all, name) => Object.hasOwn(params, name) ? String(params[name]) : all);
}

/** The monitor index a taskbar window of another monitor names in its app ID, else -1. */
export function stripMonitor(window) {
    const match = STRIP_ID.exec(window?.get_wm_class?.() || '');
    return match ? Number(match[1]) : -1;
}

/** Whether two maps of window id -> rectangle (or null) hold the same rectangles. */
function sameRects(a, b) {
    if ((a?.size ?? 0) !== b.size)
        return false;
    for (const [id, rect] of b) {
        if (!a.has(id))
            return false;
        const other = a.get(id);
        if (!rect || !other) {
            if (rect !== other)
                return false;
        } else if (rect.x !== other.x || rect.y !== other.y || rect.width !== other.width || rect.height !== other.height) {
            return false;
        }
    }
    return true;
}

function iconName(app) {
    try {
        const icon = app?.get_app_info()?.get_icon();
        if (icon instanceof Gio.ThemedIcon)
            return icon.get_names()[0] ?? null;
        if (icon instanceof Gio.FileIcon)
            return icon.get_file().get_path();
    } catch { /* Some transient windows have no desktop file. */ }
    return null;
}

export class NativePanelBridge {
    constructor({onPanelChanged = () => {}, getStartAlignment = () => 'center', frostedEnabled = true, frostedWindowBlur = true,
        language = () => 'nl'} = {}) {
        this._onPanelChanged = onPanelChanged;
        this._language = language;
        this._getStartAlignment = getStartAlignment;
        this._frostedEnabled = frostedEnabled;
        this._frostedWindowBlur = frostedWindowBlur;
        this._enabled = false;
    }

    enable() {
        if (this._enabled)
            return;
        this._enabled = true;
        this._desktopBridge = new NativeDesktopBridge((event, data) => this._emit(event, data), this._language);
        this._desktopBridge.enable();
        this._signals = [];
        this._windows = new Map();
        this._unmanagingWindows = new WeakSet();
        this._actorSignals = new Map();
        this._frostedId = 0;
        this._panel = null;
        this._panelOriginal = null;
        this._panelInhibited = false;
        this._panelPresent = false;
        // The taskbars of the other monitors: Meta.Window -> {monitor, original, inhibited, strut}; the icon
        // rectangles of every taskbar ('main' or a monitor index) for the minimize animation: as it sent them (in
        // its own window) and where they are on the stage now.
        this._strips = new Map();
        this._iconSources = new Map();
        this._iconRects = new Map();
        this._lastMonitors = '';
        this._notificationsBridge = new NativeNotificationsBridge(
            (event, data) => this._emit(event, data),
            pid => this._enabled && this._panelPresent && this._panel?.get_pid() === pid && !this._hidden(),
            this._language);
        this._notificationsBridge.enable();
        // Settings parity's bridges (notes/claude/settings-parity.md section 8.1): launch.*, display.*, input.*. A
        // new set at every enable (the session mode's unlock too); the display bridge's keep-question ids rise across.
        const emit = (event, data) => this._emit(event, data);
        this._launchBridge = new LaunchBridge(emit, this._language);
        this._launchBridge.enable();
        this._displayBridge = new DisplayBridge(emit, this._language);
        this._displayBridge.enable();
        this._inputBridge = new InputBridge(emit, this._language);
        this._inputBridge.enable();
        this._strut = null;
        this._updateId = 0;
        this._geometryId = 0;
        this._lastWindows = '';
        this._lastFocused = undefined;
        this._desktopRestore = null;
        this._startActivated = new WeakSet();
        this._startFocused = new WeakSet();
        this._positioningStarts = new WeakSet();
        this._pendingTaskView = false;
        this._revealUntil = 0;
        this._revealId = 0;
        this._panelExclusive = false;
        this._returnArmed = 0;
        this._returnTo = null;
        this._returnId = 0;
        this._tabDrags = new Map();
        this._dndSourcePid = null;
        this._dndEnded = null;
        this._dndEscaped = false;
        this._dragFilter = 0;
        this._inOverview = Main.overview.visible;
        this._exported = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._exported.export(Gio.DBus.session, BRIDGE_PATH);
        this._nameId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BRIDGE_NAME, Gio.BusNameOwnerFlags.DO_NOT_QUEUE,
            () => {
                this._queueWindows();
                this._emitMonitors(true);
            },
            () => console.warn('Win11 GNOME bridge could not own its D-Bus name'));
        this._highContrast = false;
        const a11ySchema = Gio.SettingsSchemaSource.get_default()?.lookup('org.gnome.desktop.a11y.interface', true);
        if (a11ySchema?.has_key('high-contrast')) {
            this._a11y = new Gio.Settings({settings_schema: a11ySchema});
            this._highContrast = this._a11y.get_boolean('high-contrast');
            this._connect(this._a11y, 'changed::high-contrast', () => {
                this._highContrast = this._a11y.get_boolean('high-contrast');
                this._frostedBridge?.sync();
            });
        }
        this._frostedBridge = new NativeFrostedBridge({panel: () => this._panel,
            geometry: () => this._geometry(), startReady: window => this._actorSignals.get(window)?.ready === true, windows: () => [...this._windows.keys()], registeredOnly: true,
            strips: () => [...this._strips].filter(([, entry]) => !entry.inhibited)
                .map(([window, entry]) => ({window, monitor: entry.monitor})),
            available: () => this._enabled && !this._hidden() && !this._highContrast,
            onChanged: data => this._emit('nativeBackdrop.changed', data), language: this._language});
        this._frostedBridge.configure({enabled: this._frostedEnabled, blur: true, windowBlur: this._frostedWindowBlur, shadows: true, radius: 14});
        this._launchGrants = new Map();
        this._takeAttention();
        this._connect(global.display, 'restacked', () => this._queueWindows());
        this._connect(global.display, 'window-created', (_display, window) => this._watchWindow(window));
        const dnd = global.backend?.get_dnd?.();
        if (dnd) {
            this._connect(dnd, 'dnd-enter', () => {
                this._dndSourcePid = global.display.focus_window?.get_pid() ?? null;
                this._dndEscaped = false;
            });
            this._connect(dnd, 'dnd-leave', () => {
                this._dndEnded = {pid: this._dndSourcePid,
                    time: GLib.get_monotonic_time(), pointer: [...global.get_pointer()],
                    cancelled: this._dragFilter ? this._dndEscaped : null};
                this._dndSourcePid = null;
            });
            // Mutter handles DnD Escape before stage captured-event. Observe the raw event
            // without consuming it, and retain only an Escape bit for an active own tab drag.
            if (typeof Clutter.event_add_filter === 'function') {
                this._dragFilter = Clutter.event_add_filter(null, event => {
                    if (this._enabled && this._dndSourcePid !== null &&
                        this._tabDrags.has(this._dndSourcePid) &&
                        event.type() === Clutter.EventType.KEY_PRESS &&
                        event.get_key_symbol() === Clutter.KEY_Escape)
                        this._dndEscaped = true;
                    return Clutter.EVENT_PROPAGATE;
                });
            }
        }
        this._connect(global.display, 'notify::focus-window', () => {
            this._syncStartFocus();
            // A launch whose window got the focus by itself used its token: its grant ends (D-G61).
            const focus = global.display.focus_window;
            const app = focus && this._launchGrants.size ? Shell.WindowTracker.get_default().get_window_app(focus) : null;
            if (app)
                this._launchGrants.delete(app.get_id());
            // The new focus decides whether the taskbar shows over a fullscreen app; a pending Super or
            // Alt+Tab reveal ends here (Start or the panel has the focus by now, or the user went elsewhere).
            this._revealUntil = 0;
            this._syncVisibility();
            this._queueWindows();
        });
        this._connect(global.display, 'in-fullscreen-changed', () => this._syncVisibility());
        this._connect(global.workspace_manager, 'active-workspace-changed', () => { this._queueWindows(); this._queueGeometry(); });
        // A monitor plugged in or out (or a new primary) can renumber the monitors without moving a window.
        this._connect(Main.layoutManager, 'monitors-changed', () => {
            this._queueGeometry();
            this._queueWindows();
        });
        this._connect(global.display, 'workareas-changed', () => this._queueGeometry());
        this._connect(Main.sessionMode, 'updated', () => {
            this._syncVisibility();
            this._queueWindows();
            this._queueGeometry();
        });
        this._connect(Main.overview, 'showing', () => {
            this._inOverview = true;
            this._syncVisibility();
        });
        this._connect(Main.overview, 'hidden', () => {
            this._inOverview = false;
            this._syncVisibility();
        });
        this._connect(St.ThemeContext.get_for_stage(global.stage), 'notify::scale-factor', () => this._queueGeometry());
        this._connect(global, 'shutdown', () => this.disable());
        for (const actor of global.get_window_actors()) {
            if (actor.meta_window)
                this._watchWindow(actor.meta_window, true);
        }
        this._queueWindows();
    }

    _connect(object, signal, callback) {
        this._signals.push([object, object.connect(signal, callback)]);
    }

    _t(key, params) {
        return text(key, this._language(), params);
    }

    _isPanel(window) {
        // The Wayland app ID first: win11-shell's Start and desktop windows (D-G63) carry its GTK application ID
        // (the Panel's) but their own app ID, and must never be taken for the taskbar.
        return (window.get_wm_class() || window.get_gtk_application_id()) === PANEL_APP_ID;
    }

    _isStart(window) {
        // An exact application identity plus a normal main window: titles and caller IDs never qualify.
        return window.get_window_type() === Meta.WindowType.NORMAL &&
            (window.get_gtk_application_id() === START_APP_ID || window.get_wm_class() === START_APP_ID);
    }

    _considerStart(window, shown = false) {
        if (!this._enabled || this._unmanagingWindows.has(window) || !this._isStart(window))
            return;
        this._positionStart(window);
        const actor = window.get_compositor_private();
        if (actor && !this._startActivated.has(window))
            // Start supplies its own upward/downward animation (win11-shell).
            // Do not combine that motion with GNOME's normal app-map scale effect.
            Main.wm.skipNextEffect(actor);
        if (!(shown || actor?.mapped) || this._startActivated.has(window))
            return;
        this._startActivated.add(window);
        if (this._isUserSession()) {
            window.unminimize();
            Main.activateWindow(window, global.get_current_time());
            this._syncStartFocus();
        }
    }

    _syncStartFocus() {
        const focused = global.display.focus_window;
        if (focused && this._isStart(focused)) {
            this._startFocused.add(focused);
            return;
        }
        // Dismiss Start when the compositor focus leaves it (a GTK window's own focus-out need not match it),
        // but only for our own Start windows that actually held compositor focus.
        let dismiss = false;
        for (const window of this._windows.keys()) {
            if (this._isStart(window) && this._startFocused.has(window)) {
                this._startFocused.delete(window);
                dismiss = true;
            }
        }
        if (dismiss)
            // Hiding a resident GTK window can leave its old Meta.Window in the
            // unmanaging phase briefly. Address the menu, not that stale window.
            this._startAction('start-dismiss');
    }

    /** Re-anchor mapped Start windows without taking focus again during monitor or size changes. */
    positionStarts() {
        if (!this._enabled)
            return;
        for (const window of this._windows.keys()) {
            if (this._isStart(window))
                this._positionStart(window);
        }
    }

    _positionStart(window) {
        // Mutter clears the workspace and emits workspace-changed before unmanaged.
        // Repositioning that closing window enters its work-area constraints with
        // no workspace and aborts the whole compositor (meta_window_get_workspaces).
        if (!this._enabled || !this._windows.has(window) || this._unmanagingWindows.has(window) ||
            !window.get_workspace())
            return;
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor || this._positioningStarts.has(window))
            return;
        this._positioningStarts.add(window);
        try {
            if (window.get_monitor() !== monitor.index)
                window.move_to_monitor(monitor.index);
            const workspace = global.workspace_manager.get_active_workspace();
            let work = workspace.get_work_area_for_monitor(monitor.index);
            // Over a fullscreen app the taskbar shows without a strut (_strutHidden): keep Start above it,
            // where the strut would have put it, instead of 48 px lower behind the taskbar.
            if (this._panelPresent && this._strutHidden() && !this._hidden()) {
                const top = monitor.y + monitor.height - this._geometry().barHeight;
                if (work.y + work.height > top && top > work.y)
                    work = new Mtk.Rectangle({x: work.x, y: work.y, width: work.width, height: top - work.y});
            }
            const monitorRect = new Mtk.Rectangle({x: monitor.x, y: monitor.y,
                width: monitor.width, height: monitor.height});
            let protocolWork;
            let protocolMonitor;
            let toStage;
            try {
                protocolWork = window.stage_to_protocol_rect(work);
                protocolMonitor = window.stage_to_protocol_rect(monitorRect);
                if (!(protocolWork?.width > 0 && protocolWork?.height > 0 && protocolMonitor?.width > 0))
                    throw new Error('Client protocol geometry is not ready');
                toStage = rect => window.protocol_to_stage_rect(rect);
            } catch {
                const scale = global.display.get_monitor_scale(monitor.index) || 1;
                const logical = rect => ({x: rect.x / scale, y: rect.y / scale,
                    width: rect.width / scale, height: rect.height / scale});
                protocolWork = logical(work);
                protocolMonitor = logical(monitorRect);
                toStage = rect => new Mtk.Rectangle({x: Math.round(rect.x * scale), y: Math.round(rect.y * scale),
                    width: Math.round(rect.width * scale), height: Math.round(rect.height * scale)});
            }
            const width = Math.max(1, Math.min(642, Math.floor(protocolWork.width - 24)));
            const height = Math.max(1, Math.min(726, Math.floor(protocolWork.height - 24)));
            const marginX = Math.min(12, Math.max(0, Math.floor((protocolWork.width - width) / 2)));
            const marginY = Math.min(12, Math.max(0, Math.floor((protocolWork.height - height) / 2)));
            const minX = protocolWork.x + marginX;
            const maxX = protocolWork.x + protocolWork.width - marginX - width;
            const centredX = protocolMonitor.x + (protocolMonitor.width - width) / 2;
            const anchorX = this._getStartAlignment() === 'left' ? minX : centredX;
            const target = toStage(new Mtk.Rectangle({
                x: Math.round(Math.max(minX, Math.min(maxX, anchorX))),
                y: Math.round(protocolWork.y + protocolWork.height - marginY - height), width, height,
            }));
            const current = window.get_frame_rect();
            if (current.x !== target.x || current.y !== target.y ||
                current.width !== target.width || current.height !== target.height)
                window.move_resize_frame(false, target.x, target.y, target.width, target.height);
        } finally {
            this._positioningStarts.delete(window);
        }
    }

    _toggleStart() {
        if (!this._isUserSession())
            return failure('session_unavailable', this._t('sessionStart'));
        const starts = [...this._windows.keys()].filter(window => this._isStart(window));
        // Opening Start over a fullscreen app shows the taskbar with it, as in Windows 11.
        if (!starts.length)
            this._requestReveal();
        if (this._residentStart()) {
            // win11-shell owns Start's name and its window (normally unmapped, so absent from _windows):
            // activate that GApplication directly.
            this._activateStart();
            return {opened: starts.length === 0, resident: true};
        }
        // No Start without win11-shell. Bring the taskbar (whose host execs win11-shell) back if it is gone,
        // as the task view does; Start opens with the next press.
        if (!this._panelPresent)
            Shell.AppSystem.get_default().lookup_app(`${PANEL_APP_ID}.desktop`)?.open_new_window(-1);
        return failure('not_found', this._t('startMissing'));
    }

    _openTaskView() {
        if (!this._isUserSession()) return {opened: false};
        // The task view is drawn in the panel window: over a fullscreen app, show that window first. Esc then
        // gives the keyboard back to the app (_setPanelKeyboard), so the taskbar hides again.
        if (this._fullscreenMonitor())
            this._returnArmed = this._panelExclusive ? 2 : 1;
        this._requestReveal();
        if (!this._panelPresent) {
            this._pendingTaskView = true;
            Shell.AppSystem.get_default().lookup_app(`${PANEL_APP_ID}.desktop`)?.open_new_window(-1);
            return {opened: true, pending: true};
        }
        Gio.DBus.session.call(PANEL_APP_ID, '/io/github/win11shell/Panel', 'org.gtk.Actions', 'Activate',
            new GLib.Variant('(sava{sv})', ['panel-taskview', [], {}]), null,
            Gio.DBusCallFlags.NO_AUTO_START, 3000, null, (connection, reply) => {
                try { connection.call_finish(reply); }
                catch (error) { if (this._enabled) console.warn(`Win11 task view: ${error.message}`); }
            });
        return {opened: true};
    }

    /**
     * A power action chosen in Start. Start is its own window, only as large as the menu, so its confirmation dialog
     * and the shutdown screen would be cut off there; the panel window covers the monitor and draws them full-screen,
     * as it does for the same action from Win+X. Over a fullscreen app this follows the task view (_openTaskView).
     * Without a panel window Start keeps the action and shows the dialog itself.
     */
    _openPower(args) {
        if (!this._isUserSession())
            return failure('session_unavailable', this._t('sessionUnavailable'));
        if (!POWER_ACTIONS.includes(args.action))
            return failure('bad_args', this._t('powerAction'));
        if (!this._panelPresent)
            return failure('session_unavailable', this._t('panelMissing'));
        if (this._fullscreenMonitor())
            this._returnArmed = this._panelExclusive ? 2 : 1;
        this._requestReveal();
        Gio.DBus.session.call(PANEL_APP_ID, '/io/github/win11shell/Panel', 'org.gtk.Actions', 'Activate',
            new GLib.Variant('(sava{sv})', ['panel-power', [new GLib.Variant('s', args.action)], {}]), null,
            Gio.DBusCallFlags.NO_AUTO_START, 3000, null, (connection, reply) => {
                try { connection.call_finish(reply); }
                catch (error) { if (this._enabled) console.warn(`Win11 power: ${error.message}`); }
            });
        return {opened: true};
    }

    _residentStart() {
        try {
            return Gio.DBus.session.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus', 'NameHasOwner', new GLib.Variant('(s)', [START_APP_ID]),
                new GLib.VariantType('(b)'), Gio.DBusCallFlags.NONE, 1000, null).deep_unpack()[0];
        } catch {
            return false;
        }
    }

    _activateStart() {
        Gio.DBus.session.call(START_APP_ID, '/io/github/win11shell/Start',
            'org.freedesktop.Application', 'Activate', new GLib.Variant('(a{sv})', [{}]),
            null, Gio.DBusCallFlags.NO_AUTO_START, 3000, null, (connection, reply) => {
                try { connection.call_finish(reply); }
                catch (error) { if (this._enabled) console.warn(`Win11 Start activation: ${error.message}`); }
            });
    }

    _stopStart() {
        // Also reaches a hidden menu with no Meta.Window. win11-shell owns the action: Start closes, the shell
        // process (taskbar and desktop) stays.
        this._startAction('start-quit');
    }

    _startAction(action) {
        Gio.DBus.session.call(START_APP_ID, '/io/github/win11shell/Start',
            'org.gtk.Actions', 'Activate', new GLib.Variant('(sava{sv})', [action, [], {}]),
            null, Gio.DBusCallFlags.NO_AUTO_START, 3000, null, (connection, reply) => {
                try { connection.call_finish(reply); }
                catch { /* No running Start, or it already exited with its panel. */ }
            });
    }

    _activationToken(args) {
        if (!this._isUserSession())
            return failure('session_unavailable', this._t('sessionApps'));
        const own = {
            explorer: ['io.github.win11shell.Explorer.desktop', 'appExplorer'],
            settings: ['io.github.win11shell.Settings.desktop', 'appSettings'],
            taskmgr: ['io.github.win11shell.TaskManager.desktop', 'appTaskManager'],
            alles: ['io.github.win11shell.Search.desktop', 'appAlles'],
            disks: ['io.github.win11shell.Disks.desktop', 'appDisks'],
            defaultapps: ['io.github.win11shell.DefaultApps.desktop', 'appDefaultApps'],
            installer: ['io.github.win11shell.Installer.desktop', 'appInstaller'],
        };
        let info;
        if (typeof args.app === 'string' && Object.hasOwn(own, args.app)) {
            const [id, key] = own[args.app];
            const title = this._t(key);
            info = Gio.DesktopAppInfo.new(id) ??
                Gio.AppInfo.create_from_commandline('/usr/bin/true', title, Gio.AppInfoCreateFlags.NONE);
        } else if (typeof args.desktopId === 'string' && args.desktopId.length <= 1024 &&
            !args.desktopId.includes('/') && !args.desktopId.includes('\0')) {
            info = Gio.DesktopAppInfo.new(args.desktopId);
        }
        if (!info)
            return failure('not_found', this._t('appMissing'));
        const id = info.get_id();
        if (id)
            this._launchGrants.set(id, GLib.get_monotonic_time() + LAUNCH_GRANT_MS * 1000);
        // D-Bus calls have no Clutter input event: get_current_time() is zero here.
        // A compositor timestamp gives Mutter a valid user-launch startup sequence.
        const context = global.create_app_launch_context(global.display.get_current_time_roundtrip(), -1);
        return {token: context.get_startup_notify_id(info, []) ?? ''};
    }

    /**
     * An app that runs once (Claude, WhatsApp, Steam, often Firefox) hands a launch from Start or the taskbar to
     * its running window, which asks for the focus without the token it was given: GNOME then shows "“…” is
     * ready" instead of the window (D-G61). Like Windows, whose launcher may give the foreground to what it
     * starts, a window of an app launched here within LAUNCH_GRANT_MS gets the focus, once (a window of that app
     * that gets the focus by itself ends the grant, see notify::focus-window). Every other request
     * still goes to GNOME's own handler, which is reconnected on disable.
     */
    _takeAttention() {
        const handler = Main.windowAttentionHandler;
        if (typeof handler?._onWindowDemandsAttention !== 'function')
            return;
        global.display.disconnectObject(handler);
        this._attentionHandler = handler;
        const onAttention = (display, window) => {
            if (!this._grantFocus(window))
                handler._onWindowDemandsAttention(display, window);
        };
        global.display.connectObject('window-demands-attention', onAttention,
            'window-marked-urgent', onAttention, this);
    }

    _releaseAttention() {
        const handler = this._attentionHandler;
        if (!handler)
            return;
        this._attentionHandler = null;
        global.display.disconnectObject(this);
        global.display.connectObject(
            'window-demands-attention', handler._onWindowDemandsAttention.bind(handler),
            'window-marked-urgent', handler._onWindowDemandsAttention.bind(handler),
            handler);
    }

    _grantFocus(window) {
        if (!window || window.has_focus() || window.is_skip_taskbar() || !this._isUserSession())
            return false;
        const id = Shell.WindowTracker.get_default().get_window_app(window)?.get_id();
        const until = id ? this._launchGrants.get(id) : 0;
        if (!until)
            return false;
        this._launchGrants.delete(id);
        if (GLib.get_monotonic_time() > until)
            return false;
        Main.activateWindow(window, global.display.get_current_time_roundtrip());
        return true;
    }

    _setPanelKeyboard(args) {
        if (!this._isUserSession() || !this._panel)
            return failure('session_unavailable', this._t('panelMissing'));
        if (typeof args.exclusive !== 'boolean')
            return failure('bad_args', this._t('keyboardMode'));
        // 'exclusive' while a flyout or menu of the taskbar is open: it counts as an open surface of ours.
        this._panelExclusive = args.exclusive;
        if (args.exclusive) {
            const before = global.display.focus_window;
            if (this._returnArmed) {
                this._returnArmed = 2;
                this._returnTo = before && this._userWindows(true).includes(before) ? before : null;
            }
            const timestamp = global.display.get_current_time_roundtrip();
            this._panel.unminimize();
            Main.activateWindow(this._panel, timestamp);
            this._panel.focus(timestamp);
        } else {
            if (this._returnArmed === 2) {
                this._returnArmed = 0;
                this._queueFocusReturn();
            }
            this._syncVisibility();
        }
        return {keyboard: args.exclusive ? 'exclusive' : 'on-demand'};
    }

    /** The compositor focus is on the taskbar window or on one of its popups (win11-shell's light dismiss). */
    _panelFocused() {
        const focus = global.display.focus_window;
        return Boolean(this._panel && focus && (focus === this._panel || focus.get_transient_for() === this._panel));
    }

    /**
     * panel.inputChanged: win11-shell set a new input region on the taskbar window (a flyout opened, closed or
     * changed size, a toast came or went) and the frame that commits it went out. mutter keeps a new input region
     * without repicking, so a pointer above an area the panel just gave up stayed with the invisible panel surface:
     * clicks on the window under it did nothing until the pointer crossed the strip (Robin, 14-11). A relayout
     * repicks every pointer.
     */
    _panelInputChanged() {
        if (!this._panel)
            return failure('session_unavailable', this._t('panelMissing'));
        this._panel.get_compositor_private()?.queue_relayout();
        return {};
    }

    _queueFocusReturn() {
        if (this._returnId)
            GLib.source_remove(this._returnId);
        this._returnId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, RETURN_MS, () => {
            this._returnId = 0;
            this._returnFocus();
            return GLib.SOURCE_REMOVE;
        });
    }

    /**
     * Esc (or a click outside) closed the task view over a fullscreen app without choosing a window: the panel
     * still has the keyboard, so give it back to the window it came from (else the topmost one), as Windows 11
     * does; that app has the keys again and the taskbar hides. A window chosen in the task view, Start or a new
     * flyout within RETURN_MS wins. Only over a fullscreen app: elsewhere the focus stays as it always was.
     */
    _returnFocus() {
        const target = this._returnTo;
        this._returnTo = null;
        if (!this._enabled || !this._panel || this._panelExclusive || !this._isUserSession() || this._inOverview ||
            !this._fullscreenMonitor() || this._startOpen())
            return;
        const focus = global.display.focus_window;
        if (focus && focus !== this._panel)
            return;
        const workspace = global.workspace_manager.get_active_workspace();
        const windows = this._userWindows(true).filter(window => !window.minimized && window.located_on_workspace(workspace));
        const next = windows.includes(target) ? target : global.display.sort_windows_by_stacking(windows).at(-1);
        if (next)
            Main.activateWindow(next, global.display.get_current_time_roundtrip());
    }

    /** Super or Alt+Tab over a fullscreen app: show the taskbar now, before Start or the task view has the
     * focus. The next focus change or REVEAL_MS ends this request, so it can never keep the taskbar up. */
    _requestReveal() {
        if (!this._enabled || !this._fullscreenMonitor())
            return;
        this._revealUntil = GLib.get_monotonic_time() + REVEAL_MS * 1000;
        if (this._revealId)
            GLib.source_remove(this._revealId);
        this._revealId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, REVEAL_MS, () => {
            this._revealId = 0;
            this._revealUntil = 0;
            if (this._returnArmed === 1)
                this._returnArmed = 0;
            this._syncVisibility();
            return GLib.SOURCE_REMOVE;
        });
        this._syncVisibility();
    }

    _startOpen() {
        return [...this._windows.keys()].some(window => this._isStart(window));
    }

    _isUserSession() {
        return Main.sessionMode.hasWindows && !Main.sessionMode.isLocked && !Main.sessionMode.isGreeter;
    }

    _queueFrosted() {
        if (!this._enabled || this._frostedId) return;
        this._frostedId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._frostedId = 0;
            this._frostedBridge?.sync();
            return GLib.SOURCE_REMOVE;
        });
    }

    _watchActor(window, alreadyDrawn = false) {
        const identity = window.get_wm_class() || window.get_gtk_application_id() || '';
        if (!/^io\.github\.win11shell\.(Explorer|Settings|TaskManager|Search|Disks|DefaultApps|Installer|Calendar|Start)$/.test(identity)) return;
        const actor = window.get_compositor_private();
        if (!actor || this._actorSignals.get(window)?.actor === actor) return;
        this._unwatchActor(window);
        const signals = [];
        // A Mutter effect suspends the backdrop until the actor is stable again. Moves and resizes need nothing:
        // the backdrop is bound to the actor (frostedBridge.js _place).
        for (const property of ['visible', 'mapped', 'opacity', 'scale-x', 'scale-y', 'translation-x', 'translation-y', 'translation-z', 'rotation-angle-z'])
            signals.push(actor.connect(`notify::${property}`, () => {
                this._frostedBridge?.suspend(window);
                this._queueFrosted();
            }));
        signals.push(actor.connect('destroy', () => {
            this._actorSignals.delete(window);
            this._queueFrosted();
        }));
        const watched = {actor, signals, ready: identity !== 'io.github.win11shell.Start' || alreadyDrawn};
        if (!watched.ready) signals.push(actor.connect('first-frame', () => {
            watched.ready = true;
            this._queueFrosted();
        }));
        this._actorSignals.set(window, watched);
    }

    _unwatchActor(window) {
        const watched = this._actorSignals.get(window);
        if (!watched) return;
        this._actorSignals.delete(window);
        for (const id of watched.signals) watched.actor.disconnect(id);
    }

    _watchWindow(window, alreadyDrawn = false) {
        if (!this._enabled || this._windows.has(window))
            return;
        const signals = [];
        this._windows.set(window, signals);
        const connect = (name, fn) => signals.push(window.connect(name, fn));
        connect('unmanaging', () => this._unmanagingWindows.add(window));
        const changed = () => {
            if (this._unmanagingWindows.has(window))
                return;
            this._watchActor(window);
            this._considerPanel(window);
            this._considerStrip(window);
            this._considerStart(window);
            this._considerPicker(window);
            if (this._panel === window || this._strips.has(window))
                this._queueGeometry();
            this._queueWindows();
        };
        for (const name of [
            'notify::title', 'notify::wm-class', 'notify::gtk-application-id',
            'notify::minimized', 'notify::maximized-horizontally', 'notify::maximized-vertically',
            'notify::urgent', 'notify::demands-attention', 'notify::transient-for', 'workspace-changed',
            'position-changed', 'size-changed',
        ])
            connect(name, changed);
        // A moving or resizing window with a shadow or blur: redraw the whole stage (frostedBridge damage).
        connect('position-changed', () => this._frostedBridge?.damage(window));
        connect('size-changed', () => this._frostedBridge?.damage(window));
        connect('shown', () => {
            if (this._unmanagingWindows.has(window))
                return;
            this._watchActor(window);
            this._considerPanel(window);
            this._considerStrip(window);
            this._considerStart(window, true);
            this._considerPicker(window);
            if (this._panel === window)
                this._reportPanel(true);
            if (this._isStart(window))
                this._syncVisibility();
            this._queueWindows();
        });
        connect('unmanaged', () => {
            const start = this._isStart(window);
            if (this._panel === window)
                this._forgetPanel();
            if (this._strips.has(window))
                this._forgetStrip(window);
            this._unwatchWindow(window);
            // Start closed: over a fullscreen app that has the focus again, the taskbar hides with it.
            if (start)
                this._syncVisibility();
            this._queueWindows();
        });
        this._watchActor(window, alreadyDrawn);
        this._considerPanel(window);
        this._considerStrip(window);
        this._considerStart(window);
        this._considerPicker(window);
    }

    _unwatchWindow(window) {
        this._unwatchActor(window);
        this._tabDrags?.delete(window.get_pid());
        for (const id of this._windows.get(window) ?? [])
            window.disconnect(id);
        this._windows.delete(window);
        this._frostedBridge?.forget(window.get_pid());
    }

    _considerPanel(window) {
        if (!this._enabled)
            return;
        if (!this._isPanel(window)) {
            // A new win11-shell window (the desktop after the lock screen, Start) can carry the Panel's app ID for
            // a moment before it takes its own: if it was taken for the taskbar, let it go and take the real one.
            if (this._panel === window) {
                this._releasePanel();
                const panel = [...this._windows.keys()].find(other => other !== window && this._isPanel(other));
                if (panel)
                    this._considerPanel(panel);
            }
            return;
        }
        if (this._panel && this._panel !== window)
            return;
        if (!this._panel) {
            this._panelOriginal = {
                type: window.get_window_type(), above: window.is_above(),
                sticky: window.is_on_all_workspaces(), skip: window.skip_taskbar,
            };
            this._panel = window;
            window.set_type(Meta.WindowType.DOCK);
            window.hide_from_window_list();
            window.stick();
            window.make_above();
            this._positionPanel();
            this._syncVisibility();
            const actor = window.get_compositor_private();
            if (actor?.mapped || actor?.visible)
                this._reportPanel(true);
        }
    }

    // A picker belongs to the window it was opened for, as an owned window on Windows: no taskbar entry of its own.
    // Mutter keeps a modal one (a modal dialog) off the task list already, but a non-modal one is a normal window
    // with a transient parent; hide that one too (r2d-picker).
    _considerPicker(window) {
        if (window.skip_taskbar || !window.get_transient_for())
            return;
        if (window.get_wm_class() !== PICKER_APP_ID && window.get_gtk_application_id() !== PICKER_APP_ID)
            return;
        window.hide_from_window_list();
    }

    _reportPanel(present) {
        if (this._panelPresent === present)
            return;
        this._panelPresent = present;
        this._notificationsBridge?.sync();
        try { this._onPanelChanged(present); } catch (error) { console.error(error); }
        this._syncStrut();
        this._emit('panel.changed', {present});
        this._emitMonitors();
        if (present && this._pendingTaskView) {
            this._pendingTaskView = false;
            this._openTaskView();
        }
    }

    // Undo only what _considerPanel did and nobody changed since (the desktop bridge may already have made the
    // window its desktop), then forget it.
    _releasePanel() {
        const panel = this._panel;
        const original = this._panelOriginal;
        if (panel && original) {
            try {
                if (this._panelInhibited)
                    panel.uninhibit_mapped();
                if (panel.get_window_type() === Meta.WindowType.DOCK)
                    panel.set_type(original.type);
                if (!original.above && panel.is_above())
                    panel.unmake_above();
            } catch (e) {
                logError(e, 'win11-gnome: releasing a window taken for the taskbar');
            }
        }
        this._forgetPanel();
    }

    _forgetPanel() {
        this._panel = null;
        this._panelOriginal = null;
        this._panelInhibited = false;
        this._panelExclusive = false;
        this._returnArmed = 0;
        this._returnTo = null;
        this._removeStrut();
        this._reportPanel(false);
    }

    /**
     * A taskbar window of another monitor (win11-shell, app ID STRIP_APP_ID.Monitor<N>): like the panel a dock
     * on every workspace, above the windows and out of the window list, placed at the bottom of its monitor with
     * its own strut. It takes the place of nothing: a window that carried the panel's app ID for a moment is the
     * panel's until _considerPanel lets it go.
     */
    _considerStrip(window) {
        if (!this._enabled)
            return;
        const monitor = stripMonitor(window);
        const entry = this._strips.get(window);
        if (monitor < 0 || this._panel === window) {
            if (entry)
                this._releaseStrip(window);
            return;
        }
        if (entry) {
            if (entry.monitor !== monitor) {
                entry.monitor = monitor;
                this._queueGeometry();
            }
            return;
        }
        const created = {monitor, inhibited: false, strut: null, original: {
            type: window.get_window_type(), above: window.is_above(),
            sticky: window.is_on_all_workspaces(), skip: window.skip_taskbar,
        }};
        this._strips.set(window, created);
        window.set_type(Meta.WindowType.DOCK);
        window.hide_from_window_list();
        window.stick();
        window.make_above();
        this._positionStrip(window, created);
        this._syncStrips();
        this._queueFrosted();
        this._emitMonitors();
    }

    /** Undo what _considerStrip did and nobody changed since, then forget the window (its app ID changed). */
    _releaseStrip(window) {
        const entry = this._strips.get(window);
        if (!entry)
            return;
        try {
            if (entry.inhibited)
                window.uninhibit_mapped();
            if (window.get_window_type() === Meta.WindowType.DOCK)
                window.set_type(entry.original.type);
            if (!entry.original.above && window.is_above())
                window.unmake_above();
        } catch (e) {
            logError(e, 'win11-gnome: releasing a window taken for a taskbar');
        }
        this._forgetStrip(window);
    }

    _forgetStrip(window) {
        const entry = this._strips.get(window);
        if (!entry)
            return;
        this._strips.delete(window);
        this._removeStripStrut(entry);
        // its buttons are gone: its windows minimize into the taskbar that still lists them (unless a new taskbar
        // window of that monitor sent its own already)
        if (!this._stripOf(entry.monitor)) {
            this._iconSources.delete(entry.monitor);
            if (this._iconRects.has(entry.monitor))
                this._applyIconRects(entry.monitor, new Map());
        }
        this._queueFrosted();
    }

    /** The adopted taskbar window of monitor `index`, if any. */
    _stripOf(index) {
        for (const [window, entry] of this._strips) {
            if (entry.monitor === index)
                return window;
        }
        return null;
    }

    /** A monitor's rectangle in the protocol units of our clients (its own taskbar's, else the panel's), else by
     *  its advertised scale. */
    _logicalMonitor(monitor) {
        const stage = new Mtk.Rectangle({x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height});
        for (const window of [this._stripOf(monitor.index), this._panel]) {
            if (!window)
                continue;
            try {
                const protocol = window.stage_to_protocol_rect(stage);
                if (protocol?.width > 0 && protocol?.height > 0)
                    return rectJSON(protocol);
            } catch { /* Not mapped yet. */ }
        }
        const scale = global.display.get_monitor_scale(monitor.index) || 1;
        return {x: Math.round(monitor.x / scale), y: Math.round(monitor.y / scale),
            w: Math.round(monitor.width / scale), h: Math.round(monitor.height / scale)};
    }

    /** panel.monitors: every monitor (stage rectangle and logical size), the primary and whether the panel is up;
     *  win11-shell makes a taskbar for each other monitor from it. */
    _monitorsJSON() {
        const layout = Main.layoutManager;
        const primary = layout.primaryIndex ?? -1;
        const monitors = (layout.monitors ?? []).map(monitor => ({
            index: monitor.index, primary: monitor.index === primary, ...rectJSON(monitor),
            scale: global.display.get_monitor_scale(monitor.index) || 1, logical: this._logicalMonitor(monitor),
        }));
        return {available: monitors.length > 0, primary, panel: this._panelPresent, monitors};
    }

    _emitMonitors(force = false) {
        if (!this._enabled)
            return;
        const data = this._monitorsJSON();
        const fingerprint = JSON.stringify(data);
        if (fingerprint === this._lastMonitors && !force)
            return;
        this._lastMonitors = fingerprint;
        this._emit('panel.monitors', data);
    }

    /** Where the taskbar of another monitor goes: its bottom 48 logical px, the monitor's whole width. None for the
     *  primary monitor or one that is gone: such a taskbar stays hidden until win11-shell removes it. */
    _stripGeometry(entry) {
        const layout = Main.layoutManager;
        const monitor = layout.monitors?.[entry.monitor];
        if (!monitor || entry.monitor === layout.primaryIndex)
            return null;
        const logical = this._logicalMonitor(monitor);
        const height = Math.max(1, Math.round(HEIGHT * monitor.width / Math.max(1, logical.w)));
        return {x: monitor.x, y: monitor.y + monitor.height - height, w: monitor.width, h: height};
    }

    _positionStrip(window, entry) {
        const target = this._stripGeometry(entry);
        if (!target)
            return;
        const rect = window.get_frame_rect();
        if (rect.x !== target.x || rect.y !== target.y || rect.width !== target.w || rect.height !== target.h)
            window.move_resize_frame(false, target.x, target.y, target.w, target.h);
    }

    /** The panel's rule for the taskbar of another monitor: hidden outside the user session, in the overview, and
     *  while a fullscreen app on that monitor (or one of its transients) has the focus (_showOverFullscreen
     *  without Start, the task view and the reveal, which belong to the main taskbar). */
    _stripHidden(entry) {
        if (!this._isUserSession() || this._inOverview || !this._stripGeometry(entry))
            return true;
        if (!global.display.get_monitor_in_fullscreen(entry.monitor))
            return false;
        const workspace = global.workspace_manager.get_active_workspace();
        const fullscreen = window => window.is_fullscreen() && !window.minimized &&
            window.get_monitor() === entry.monitor && window.located_on_workspace(workspace);
        if (!this._userWindows(true).some(fullscreen))
            return true;
        let focus = global.display.focus_window;
        for (let depth = 0; depth < 16 && focus?.get_transient_for(); depth++)
            focus = focus.get_transient_for();
        return !(Boolean(focus) && !fullscreen(focus));
    }

    _syncStrips() {
        if (!this._enabled || !this._strips?.size)
            return;
        let changed = false;
        for (const [window, entry] of this._strips) {
            const hidden = this._stripHidden(entry);
            if (hidden && !entry.inhibited) {
                entry.inhibited = true;
                window.inhibit_mapped();
                changed = true;
            } else if (!hidden && entry.inhibited) {
                entry.inhibited = false;
                window.uninhibit_mapped();
                changed = true;
            }
            this._syncStripStrut(entry);
        }
        if (changed)
            this._queueFrosted();
    }

    /** As _syncStrut: an independent 48 px chrome actor reserves the bottom of that monitor, so a maximized window
     *  there ends above its taskbar; none over a fullscreen app on it. */
    _syncStripStrut(entry) {
        const target = this._stripGeometry(entry);
        if (!target || !this._isUserSession() || this._inOverview ||
            global.display.get_monitor_in_fullscreen(entry.monitor)) {
            this._removeStripStrut(entry);
            return;
        }
        if (!entry.strut) {
            const strut = new St.Widget({reactive: false, opacity: 0});
            strut.connect('destroy', () => {
                if (entry.strut === strut)
                    entry.strut = null;
            });
            entry.strut = strut;
            Main.layoutManager.addChrome(strut, {affectsStruts: true, trackFullscreen: false});
        }
        entry.strut.set_position(target.x, target.y);
        entry.strut.set_size(target.w, target.h);
    }

    _removeStripStrut(entry) {
        const strut = entry.strut;
        if (!strut)
            return;
        entry.strut = null;
        Main.layoutManager.removeChrome(strut);
        strut.destroy();
    }

    _geometry() {
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor)
            return {available: false, x: 0, y: 0, w: 0, h: 0, width: 0, height: 0, scale: 1, barHeight: HEIGHT};
        const scale = global.display.get_monitor_scale(monitor.index) || 1;
        const themeScale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        const stageRect = new Mtk.Rectangle({x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height});
        const logical = {x: monitor.x, y: monitor.y,
            w: Math.round(monitor.width / scale), h: Math.round(monitor.height / scale)};
        // Mutter 50 can convert stage rects to a specific client's protocol units.
        // This avoids guessing the client's scaling once its surface is mapped.
        if (this._panel) {
            try {
                const protocol = this._panel.stage_to_protocol_rect(stageRect);
                if (protocol?.width > 0)
                    Object.assign(logical, rectJSON(protocol));
            } catch { /* Before mapping, use the monitor's advertised scale. */ }
        }
        return {available: true, ...rectJSON(stageRect), width: monitor.width, height: monitor.height,
            scale, themeScale, barHeight: HEIGHT * themeScale, logicalBarHeight: HEIGHT,
            monitor: monitor.index, logical};
    }

    _positionPanel() {
        if (!this._enabled || !this._panel)
            return;
        const geometry = this._geometry();
        if (!geometry.available)
            return;
        const rect = this._panel.get_frame_rect();
        if (rect.x !== geometry.x || rect.y !== geometry.y || rect.width !== geometry.w || rect.height !== geometry.h)
            this._panel.move_resize_frame(false, geometry.x, geometry.y, geometry.w, geometry.h);
        this._syncStrut();
        this._emit('panel.geometry', geometry);
    }

    _queueGeometry() {
        if (!this._enabled || this._geometryId)
            return;
        this._geometryId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._geometryId = 0;
            this._positionPanel();
            for (const [window, entry] of this._strips)
                this._positionStrip(window, entry);
            this._reapplyIconRects();
            this.positionStarts();
            this._syncVisibility();
            this._emitMonitors();
            return GLib.SOURCE_REMOVE;
        });
    }

    _fullscreenMonitor() {
        const monitor = Main.layoutManager.primaryMonitor;
        return monitor && global.display.get_monitor_in_fullscreen(monitor.index) ? monitor : null;
    }

    _hidden() {
        if (!this._isUserSession() || this._inOverview)
            return true;
        const monitor = this._fullscreenMonitor();
        return Boolean(monitor) && !this._showOverFullscreen(monitor);
    }

    /**
     * Windows 11's rule. Mutter keeps a monitor "in fullscreen" while a fullscreen window lies under smaller ones
     * (Start, a dialog, a normal window), so that alone hid the taskbar under Start and left Alt+Tab's task view
     * (drawn in the panel window) invisible. Over a fullscreen app the taskbar is hidden only while that app (or
     * one of its transients) has the focus and no Start, task view or other taskbar flyout of ours is open or
     * about to open. "Fullscreen windowed" apps count when mutter treats them as fullscreen. A monitor-sized
     * override-redirect window also makes the monitor fullscreen, but nothing can be stacked above it: hidden.
     */
    _showOverFullscreen(monitor) {
        const workspace = global.workspace_manager.get_active_workspace();
        const fullscreen = window => window.is_fullscreen() && !window.minimized &&
            window.get_monitor() === monitor.index && window.located_on_workspace(workspace);
        if (!this._userWindows(true).some(fullscreen))
            return false;
        if (GLib.get_monotonic_time() < this._revealUntil || this._panelExclusive || this._startOpen())
            return true;
        let focus = global.display.focus_window;
        for (let depth = 0; depth < 16 && focus?.get_transient_for(); depth++)
            focus = focus.get_transient_for();
        return Boolean(focus) && !fullscreen(focus);
    }

    /** The strut follows mutter's fullscreen state alone, as before: a taskbar shown over a fullscreen app
     * does not shrink the work area of the windows below it. */
    _strutHidden() {
        return !this._isUserSession() || this._inOverview || Boolean(this._fullscreenMonitor());
    }

    _syncVisibility() {
        this._notificationsBridge?.sync();
        this._frostedBridge?.sync();
        this._syncStrips();
        if (!this._enabled || !this._panel)
            return;
        const hidden = this._hidden();
        // The flag first: a focus change emitted from inside mutter's call re-enters here (focus-window).
        if (hidden && !this._panelInhibited) {
            this._panelInhibited = true;
            this._panel.inhibit_mapped();
            this._queueFrosted();
        } else if (!hidden && this._panelInhibited) {
            this._panelInhibited = false;
            this._panel.uninhibit_mapped();
            // The taskbar backdrop needs the mapped panel actor; the sync above ran before it was shown.
            this._queueFrosted();
        }
        this._syncStrut();
    }

    _syncStrut() {
        // The full-monitor transparent client must NEVER become a full-monitor strut.
        // Only this independent 48 CSS px chrome actor reserves the bottom edge.
        if (!this._enabled || !this._panel || !this._panelPresent || this._strutHidden()) {
            this._removeStrut();
            return;
        }
        const geometry = this._geometry();
        if (!geometry.available)
            return;
        if (!this._strut) {
            this._strut = new St.Widget({reactive: false, opacity: 0});
            this._strut.connect('destroy', () => { this._strut = null; });
            Main.layoutManager.addChrome(this._strut, {affectsStruts: true, trackFullscreen: false});
        }
        this._strut.set_position(geometry.x, geometry.y + geometry.h - geometry.barHeight);
        this._strut.set_size(geometry.w, geometry.barHeight);
    }

    _removeStrut() {
        if (!this._strut)
            return;
        const strut=this._strut;
        this._strut=null;
        Main.layoutManager.removeChrome(strut);
        strut.destroy();
    }

    _userWindows(includeSkip = false) {
        if (!this._isUserSession())
            return [];
        return global.get_window_actors().map(actor => actor.meta_window)
            .filter(window => window && !this._isPanel(window) && stripMonitor(window) < 0 && secondaryDesktop(window) < 0 &&
                window.get_gtk_application_id() !== DESKTOP_APP_ID && window.get_wm_class() !== DESKTOP_APP_ID &&
                window.get_gtk_application_id() !== 'io.github.win11shell.Start' && window.get_wm_class() !== 'io.github.win11shell.Start' &&
                USER_WINDOW_TYPES.has(window.get_window_type()) &&
                (includeSkip || !window.skip_taskbar))
            .sort((a, b) => a.get_stable_sequence() - b.get_stable_sequence());
    }

    _windowJSON(window) {
        const app = Shell.WindowTracker.get_default().get_window_app(window);
        const workspace = window.get_workspace();
        const index = workspace?.index() ?? -1;
        const rect = window.get_frame_rect();
        return {
            id: String(window.get_id()), appId: window.get_gtk_application_id() || window.get_wm_class() || '',
            title: window.get_title() || '', workspace: index,
            workspaceName: index >= 0 ? Meta.prefs_get_workspace_name(index) : '',
            minimized: window.minimized, focused: window === global.display.focus_window,
            desktopId: ({
                'io.github.win11shell.Explorer':'win11shell-explorer.desktop',
                'io.github.win11shell.Settings':'win11shell-settings.desktop',
                'io.github.win11shell.TaskManager':'win11shell-taskmgr.desktop',
                'io.github.win11shell.Search':'win11shell-alles.desktop',
                'io.github.win11shell.Disks':'win11shell-disks.desktop',
                'io.github.win11shell.DefaultApps':'win11shell-defaultapps.desktop',
                'io.github.win11shell.Installer':'win11shell-installer.desktop',
                'io.github.win11shell.Calendar':'io.github.win11shell.Calendar.desktop',
            })[window.get_gtk_application_id() || window.get_wm_class()] ?? app?.get_id() ?? null,
            icon: iconName(app), pid: window.get_pid(),
            fullscreen: window.is_fullscreen(), maximized: window.is_maximized(),
            floating: true, urgent: window.urgent || window.demands_attention,
            snapped: null, monitor: window.get_monitor(),
            xwayland: window.get_client_type() === Meta.WindowClientType.X11,
            pinned: window.is_on_all_workspaces(), initialClass: window.get_wm_class() || '',
            displaced: false, ...rectJSON(rect),
        };
    }

    _window(args, allowActive = true) {
        if (!this._isUserSession())
            return failure('session_unavailable', this._t('sessionWindows'));
        const windows = this._userWindows(true);
        const window = args.id !== undefined
            ? windows.find(candidate => String(candidate.get_id()) === String(args.id))
            : allowActive ? windows.find(candidate => candidate === global.display.focus_window) : null;
        if (!window)
            return failure('not_found', this._t('windowGone'));
        return window;
    }

    _queueWindows() {
        if (!this._enabled || this._updateId)
            return;
        this._updateId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._updateId = 0;
            if (!this._panel) {
                for (const window of this._windows.keys())
                    this._considerPanel(window);
            }
            this._frostedBridge?.sync();
            const windows = this._userWindows().map(window => this._windowJSON(window));
            // the desktop in use (Taakweergave, the taskbar's "only this desktop"): a switch alone is a change too
            const activeWorkspace = global.workspace_manager.get_active_workspace_index();
            const fingerprint = JSON.stringify([windows, activeWorkspace]);
            if (fingerprint !== this._lastWindows) {
                this._lastWindows = fingerprint;
                this._emit('windows.changed', {windows, activeWorkspace});
            }
            const focused = windows.find(window => window.focused)?.id ?? null;
            if (focused !== this._lastFocused) {
                this._lastFocused = focused;
                this._emit('windows.focused', {id: focused});
            }
            return GLib.SOURCE_REMOVE;
        });
    }

    _emit(name, data) {
        if (this._enabled && this._exported)
            this._exported.emit_signal('Event', new GLib.Variant('(ss)', [name, JSON.stringify(data)]));
    }

    // D-Bus success values are raw results. Only failures carry an {ok:false,error} envelope. A bridge may answer
    // with a promise (inputBridge.js input.capture waits for a key): the reply waits for it. GJS takes a synchronous
    // Call before CallAsync, so there is no Call.
    CallAsync([method, argsJSON], invocation) {
        const envelope = error => ({ok: false, error: {code: error?.code || 'native_error',
            message: error?.message || String(error)}});
        const reply = value => {
            let text;
            try {
                // a method without a value answers null (JSON has no undefined, and '(s)' needs a string)
                text = JSON.stringify(value === undefined ? null : value);
            } catch (error) {
                text = JSON.stringify(envelope(error));
            }
            invocation.return_value(new GLib.Variant('(s)', [text]));
        };
        const fail = error => reply(envelope(error));
        let result;
        try {
            const args = JSON.parse(argsJSON || '{}');
            // failure() throws: the catch below answers with its envelope
            if (!args || typeof args !== 'object' || Array.isArray(args))
                failure('bad_args', this._t('argsObject'));
            result = this._call(method, args);
        } catch (error) {
            fail(error);
            return;
        }
        Promise.resolve(result).then(reply, fail);
    }

    _call(method, args) {
        if (!this._enabled)
            return failure('offline', this._t('offline'));
        if (method === 'nativeBackdrop.configure')
            return this._frostedBridge.register(args);
        if (method === 'nativeBackdrop.start')
            return this._frostedBridge.start(args);
        if (method.startsWith('notifications.'))
            return this._notificationsBridge.call(method, args);
        if (method.startsWith('launch.'))
            return this._launchBridge.call(method, args);
        if (method.startsWith('display.'))
            return this._displayBridge.call(method, args);
        if (method.startsWith('input.'))
            return this._inputBridge.call(method, args);
        if (method === 'panel.keyboard')
            return this._setPanelKeyboard(args);
        if (method === 'panel.inputChanged')
            return this._panelInputChanged();
        if (method === 'desktop.ready')
            return this._desktopBridge.ready(args);
        if (method === 'desktop.geometry')
            return this._desktopBridge.geometry();
        if (method === 'apps.activationToken')
            return this._activationToken(args);
        if (method === 'panel.geometry')
            return this._geometry();
        if (method === 'panel.monitors')
            return this._monitorsJSON();
        if (method === 'panel.openTaskView')
            return this._openTaskView();
        if (method === 'panel.toggleStart')
            return this._toggleStart();
        if (method === 'panel.power')
            return this._openPower(args);
        if (method === 'explorer.tabPosition') {
            const window = this._userWindows().find(candidate => candidate.get_pid() === args.pid &&
                (candidate.get_gtk_application_id() === 'io.github.win11shell.Explorer' ||
                    candidate.get_wm_class() === 'io.github.win11shell.Explorer'));
            if (!window)
                return failure('not_found', this._t('explorerGone'));
            const now = GLib.get_monotonic_time();
            if (args.phase === 'begin') {
                for (const [pid, time] of this._tabDrags)
                    if (now - time > 90 * GLib.USEC_PER_SEC) this._tabDrags.delete(pid);
                this._tabDrags.set(args.pid, now);
            }
            const began = this._tabDrags.get(args.pid);
            const end = this._dndEnded;
            const ended = args.phase !== 'begin' && began !== undefined &&
                end?.pid === args.pid && end.time >= began && now - end.time < 5 * GLib.USEC_PER_SEC;
            const [x, y, modifiers] = ended ? end.pointer : global.get_pointer();
            const rect = window.get_frame_rect();
            return {outside: x < rect.x || y < rect.y ||
                x >= rect.x + rect.width || y >= rect.y + rect.height,
                buttonPressed: Boolean(modifiers & Clutter.ModifierType.BUTTON1_MASK), ended,
                cancelled: ended ? end.cancelled : null};
        }
        if (method === 'shell.info')
            return {mode: 'gnome', compositor: 'gnome', layerShell: false, nativeWindows: true,
                nativePanel: this._panelPresent, panelAppId: PANEL_APP_ID,
                startFocused: Boolean(global.display.focus_window && this._isStart(global.display.focus_window)),
                panelFocused: this._panelFocused()};
        if (method === 'windows.list')
            return {supported: true, windows: this._userWindows().map(window => this._windowJSON(window)),
                activeWorkspace: global.workspace_manager.get_active_workspace_index()};
        if (method === 'windows.setIconRects')
            return this._setIconRects(args);
        if (method === 'windows.showDesktop')
            return this._showDesktop();
        const window = this._window(args);
        const timestamp = global.get_current_time();
        switch (method) {
        case 'windows.focus':
        case 'windows.restore':
            window.unminimize();
            Main.activateWindow(window, timestamp);
            break;
        case 'windows.minimize':
            if (!window.can_minimize())
                return failure('unsupported', this._t('cantMinimize'));
            window.minimize();
            break;
        case 'windows.close':
            if (!window.can_close())
                return failure('unsupported', this._t('cantClose'));
            window.delete(timestamp);
            break;
        case 'windows.maximize':
            if (!window.can_maximize())
                return failure('unsupported', this._t('cantMaximize'));
            window.maximize();
            break;
        case 'windows.toggleMaximize': {
            const wanted = typeof args.want === 'boolean' ? args.want : !window.is_maximized();
            if (wanted) {
                if (!window.can_maximize())
                    return failure('unsupported', this._t('cantMaximize'));
                window.maximize();
            } else {
                window.unmaximize();
            }
            break;
        }
        case 'windows.setPinned':
            if (args.pinned !== false) {
                window.stick();
                window.make_above();
            } else {
                window.unmake_above();
                window.unstick();
            }
            break;
        case 'windows.snap':
            return this._snap(window, args.edge);
        default:
            return failure('unknown_method', this._t('unknownMethod', {method}));
        }
        this._queueWindows();
        return {supported: true};
    }

    _snap(window, edge) {
        if (edge === 'maximize') {
            if (!window.can_maximize())
                return failure('unsupported', this._t('cantMaximize'));
            window.maximize();
        } else if (edge === 'restore') {
            window.unmaximize();
        } else {
            const work = window.get_work_area_for_monitor(window.get_monitor());
            const halfWidth = Math.floor(work.width / 2);
            const halfHeight = Math.floor(work.height / 2);
            const rects = {
                left: [work.x, work.y, halfWidth, work.height],
                right: [work.x + halfWidth, work.y, work.width - halfWidth, work.height],
                'top-left': [work.x, work.y, halfWidth, halfHeight],
                'top-right': [work.x + halfWidth, work.y, work.width - halfWidth, halfHeight],
                'bottom-left': [work.x, work.y + halfHeight, halfWidth, work.height - halfHeight],
                'bottom-right': [work.x + halfWidth, work.y + halfHeight, work.width - halfWidth, work.height - halfHeight],
            };
            if (!Object.hasOwn(rects, edge))
                return failure('bad_args', this._t('badSnap'));
            if (!window.allows_resize())
                return failure('unsupported', this._t('cantResize'));
            window.unmaximize();
            window.move_resize_frame(true, ...rects[edge]);
        }
        this._queueWindows();
        return {supported: true};
    }

    _showDesktop() {
        if (!this._isUserSession())
            return failure('session_unavailable', this._t('sessionUnavailable'));
        if (this._desktopRestore) {
            const {ids, focused} = this._desktopRestore;
            this._desktopRestore = null;
            const windows = this._userWindows();
            for (const window of windows) {
                if (ids.includes(String(window.get_id())))
                    window.unminimize();
            }
            const previous = windows.find(window => String(window.get_id()) === focused);
            if (previous)
                Main.activateWindow(previous);
        } else {
            const workspace = global.workspace_manager.get_active_workspace();
            const windows = this._userWindows().filter(window => !window.minimized && window.located_on_workspace(workspace) && window.can_minimize());
            this._desktopRestore = {ids: windows.map(window => String(window.get_id())),
                focused: global.display.focus_window ? String(global.display.focus_window.get_id()) : null};
            for (const window of windows)
                window.minimize();
        }
        this._queueWindows();
        return {supported: true, showingDesktop: Boolean(this._desktopRestore)};
    }

    /**
     * windows.setIconRects: where each window's button rests on one taskbar, for the minimize animation. The main
     * taskbar sends its rectangles in the panel window's coordinates (no `monitor`), a taskbar of another monitor in
     * its own window's with its `monitor`. Every taskbar's last rectangles are kept: a window minimizes into its
     * button on the taskbar of its own monitor, else on the main taskbar, else on any taskbar that shows it (Windows
     * 11's "All taskbars" shows it on several). With one taskbar this is the rectangle that taskbar sent.
     *
     * A taskbar sends again only when its buttons move in its own window, not when the window itself moves (another
     * monitor layout, another primary): so the rectangles are kept as sent and placed on the stage again wherever
     * that taskbar is now (_reapplyIconRects from _queueGeometry).
     */
    _setIconRects(args) {
        if (!this._isUserSession())
            return {ok: true, count: 0, anim: false};
        if (!Array.isArray(args.rects))
            return failure('bad_args', this._t('badRects'));
        const source = Number.isInteger(args.monitor) ? args.monitor : 'main';
        if (source !== 'main' && !this._stripOf(source))
            return {ok: true, count: 0, anim: false};
        const sent = [];
        for (const rect of args.rects.slice(0, 2048)) {
            if (!rect || ![rect.x, rect.y, rect.w, rect.h].every(Number.isFinite) || rect.w <= 0 || rect.h <= 0)
                continue;
            const ids = (Array.isArray(rect.ids) ? rect.ids : [rect.id]).map(id => String(id));
            sent.push({ids, x: rect.x, y: rect.y, w: rect.w, h: rect.h});
        }
        this._iconSources.set(source, {rects: sent, hidden: Boolean(args.hidden)});
        const windows = new Map(this._userWindows().map(window => [String(window.get_id()), window]));
        const {rects, count} = this._stageIconRects(source, windows);
        this._applyIconRects(source, rects, windows);
        return {ok: true, count, anim: false};
    }

    /** Taskbar `source`'s rectangles as it sent them, on the stage where that taskbar is now: the main taskbar's
     *  from the primary monitor's corner (the panel window covers it), another monitor's from the place of its
     *  taskbar there (_stripGeometry; none while that monitor is gone or the primary). Window id -> rectangle. */
    _stageIconRects(source, windows) {
        const rects = new Map();
        const sent = this._iconSources.get(source);
        if (!sent)
            return {rects, count: 0};
        let origin, factor;
        if (source === 'main') {
            const geometry = this._geometry();
            origin = {x: geometry.x, y: geometry.y};
            factor = geometry.logical?.w > 0 ? geometry.w / geometry.logical.w : 1;
        } else {
            const strip = this._stripOf(source);
            const target = strip ? this._stripGeometry(this._strips.get(strip)) : null;
            if (!target)
                return {rects, count: 0};
            let protocol = null;
            try {
                protocol = strip.stage_to_protocol_rect(new Mtk.Rectangle({x: target.x, y: target.y,
                    width: target.w, height: target.h}));
            } catch { /* Not mapped yet. */ }
            origin = {x: target.x, y: target.y};
            factor = protocol?.width > 0 ? target.w / protocol.width : 1;
        }
        let count = 0;
        for (const rect of sent.rects) {
            const target = new Mtk.Rectangle({x: Math.round(origin.x + rect.x * factor), y: Math.round(origin.y + rect.y * factor),
                width: Math.round(rect.w * factor), height: Math.round(rect.h * factor)});
            for (const id of rect.ids) {
                if (windows.has(id)) {
                    rects.set(id, sent.hidden ? null : target);
                    count++;
                }
            }
        }
        return {rects, count};
    }

    /** Every taskbar's rectangles placed again where it is now (a monitor moved, the primary changed): only a taskbar
     *  whose rectangles moved on the stage gives its windows a new icon geometry. */
    _reapplyIconRects() {
        if (!this._enabled || !this._iconSources.size)
            return;
        const windows = new Map(this._userWindows().map(window => [String(window.get_id()), window]));
        for (const source of [...this._iconSources.keys()]) {
            const {rects} = this._stageIconRects(source, windows);
            if (!sameRects(this._iconRects.get(source), rects))
                this._applyIconRects(source, rects, windows);
        }
    }

    /** Keep `rects` (window id -> rectangle or null) as taskbar `source`'s and give every window it names now or
     *  named before its rectangle (see _setIconRects); a window no taskbar names keeps the one it has. */
    _applyIconRects(source, rects, windows = null) {
        const before = this._iconRects.get(source);
        if (rects.size)
            this._iconRects.set(source, rects);
        else
            this._iconRects.delete(source);
        windows ??= new Map(this._userWindows().map(window => [String(window.get_id()), window]));
        const ids = new Set([...before?.keys() ?? [], ...rects.keys()]);
        for (const id of ids) {
            const window = windows.get(id);
            if (!window)
                continue;
            const own = this._stripOf(window.get_monitor()) ? window.get_monitor() : 'main';
            const order = [own, 'main', ...this._iconRects.keys()];
            const from = order.find(key => this._iconRects.get(key)?.has(id));
            if (from !== undefined)
                window.set_icon_geometry(this._iconRects.get(from).get(id));
        }
    }

    disable() {
        if (!this._enabled)
            return;
        this._releaseAttention();
        this._launchGrants?.clear();
        this._frostedBridge?.disable();
        this._frostedBridge = null;
        this._a11y = null;
        this._notificationsBridge?.disable();
        this._notificationsBridge = null;
        this._launchBridge?.disable();
        this._launchBridge = null;
        this._displayBridge?.disable();
        this._displayBridge = null;
        this._inputBridge?.disable();
        this._inputBridge = null;
        this._stopStart();
        this._desktopBridge?.disable();
        this._desktopBridge = null;
        this._enabled = false;
        for (const source of [this._updateId, this._geometryId, this._frostedId, this._revealId, this._returnId]) {
            if (source)
                GLib.source_remove(source);
        }
        this._updateId = this._geometryId = this._frostedId = this._revealId = this._returnId = 0;
        this._revealUntil = 0;
        if (this._nameId)
            Gio.bus_unown_name(this._nameId);
        this._nameId = 0;
        this._exported?.unexport();
        this._exported = null;
        if (this._dragFilter)
            Clutter.event_remove_filter(this._dragFilter);
        this._dragFilter = 0;
        for (const [object, id] of this._signals)
            object.disconnect(id);
        this._signals = [];
        for (const window of [...this._windows.keys()])
            this._unwatchWindow(window);
        // The taskbars of the other monitors as the panel below: restored, then closed (win11-shell drops them and
        // makes new ones once the panel is back).
        for (const [window, entry] of this._strips) {
            try {
                if (entry.inhibited)
                    window.uninhibit_mapped();
                window.set_type(entry.original.type);
                if (!entry.original.above)
                    window.unmake_above();
                if (!entry.original.sticky)
                    window.unstick();
                if (!entry.original.skip)
                    window.show_in_window_list();
            } catch (e) {
                logError(e, 'win11-gnome: restoring a taskbar window');
            }
            this._removeStripStrut(entry);
            try {
                window.delete(global.get_current_time());
            } catch (e) {
                logError(e, 'win11-gnome: closing a taskbar window');
            }
        }
        this._strips.clear();
        this._iconSources.clear();
        this._iconRects.clear();
        const panel = this._panel;
        const original = this._panelOriginal;
        if (panel && original) {
            // On the lock screen GNOME's overview can throw from inside show_in_window_list (a workspace without
            // its layout manager): the rest of disable, the close below included, must still run.
            try {
                if (this._panelInhibited)
                    panel.uninhibit_mapped();
                panel.set_type(original.type);
                if (!original.above)
                    panel.unmake_above();
                if (!original.sticky)
                    panel.unstick();
                if (!original.skip)
                    panel.show_in_window_list();
            } catch (e) {
                logError(e, 'win11-gnome: restoring the taskbar window');
            }
            // An unmanaged full-monitor transparent client must not outlive its bridge. This closes only our own
            // panel, never an application or a system surface; win11-shell hides it and shows it again when the
            // extension asks for the taskbar on its return (open_new_window).
            panel.delete(global.get_current_time());
        }
        this._forgetPanel();
        this._desktopRestore = null;
        this._tabDrags?.clear();
        this._dndSourcePid = null;
        this._dndEnded = null;
    }
}
