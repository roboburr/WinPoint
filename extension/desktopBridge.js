// SPDX-License-Identifier: MIT
// Keep the native GTK 4 shell's desktop window (win11-shell, app id io.github.win11shell.Desktop) underneath apps.
// win11-shell owns the Desktop identity and its window; this bridge never launches it.
// Existing DING desktops retain their processes, preferences and extension state.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Mtk from 'gi://Mtk';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

export const DESKTOP_APP_ID = 'io.github.win11shell.Desktop';
// A desktop window stays in one monitor's work area (Mutter): the desktop covers the primary monitor, and win11-shell
// makes a wallpaper window for every other monitor with this app ID and the monitor's index
// (native/shell/src/desktop/screens.rs).
const SECONDARY_ID = /^io\.github\.win11shell\.SecondaryDesktop\.Monitor(\d{1,2})$/;
/** The monitor index a desktop window of another monitor names in its app ID, else -1. */
export const secondaryDesktop = window => {
    const match = SECONDARY_ID.exec(window?.get_wm_class?.() || '');
    return match ? Number(match[1]) : -1;
};
const DING_APP_ID = 'com.rastersoft.ding';
const sameRect = (a, b) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
const rectJSON = rect => ({x: rect.x, y: rect.y, w: rect.width, h: rect.height});
const identity = (window, id) => window.get_gtk_application_id() === id || window.get_wm_class() === id;
// i18n: pairs (tools/i18n/check_pairs.py): texts in Win11's language (contracts C1, C3).
const TEXT = {
    badPid: {nl: 'Er is een geldig proces-ID van het bureaublad nodig.', en: 'A valid desktop process ID is required.'},
};

export class NativeDesktopBridge {
    constructor(emit = () => {}, language = () => 'nl') { this._emit = emit; this._language = language; this._enabled = false; }

    enable() {
        this._enabled = true;
        this._signals = [];
        this._windows = new Map();
        this._readyPids = new Set();
        this._inhibitedDing = new Set();
        this._present = false;
        this._geometryId = 0;
        this._positioning = false;
        this._hotCorners = new Map();
        const connect = (object, signal, callback) => this._signals.push([object, object.connect(signal, callback)]);
        connect(global.display, 'window-created', (_display, window) => this._watchWindow(window));
        connect(Main.layoutManager, 'monitors-changed', () => this._queueGeometry());
        connect(global.display, 'workareas-changed', () => this._queueGeometry());
        connect(Main.sessionMode, 'updated', () => this._sync());
        connect(Main.layoutManager, 'hot-corners-changed', () => this._syncHotCorners(this._present));
        // DING filters its own desktops out of global.get_window_actors(). Read
        // Mutter's complete list so an already-running DING desktop is adopted too.
        for (const window of global.display.list_all_windows()) this._watchWindow(window, true);
    }

    isDesktop(window) { return identity(window, DESKTOP_APP_ID); }
    /** The desktop or the desktop window of another monitor. */
    _ours(window) { return this.isDesktop(window) || secondaryDesktop(window) >= 0; }
    _isDing(window) {
        // Never hide a normal DING settings/file window or an unrelated desktop.
        return identity(window, DING_APP_ID) && window.get_window_type() === Meta.WindowType.DESKTOP;
    }
    _userSession() { return Main.sessionMode.hasWindows && !Main.sessionMode.isLocked && !Main.sessionMode.isGreeter; }

    _watchWindow(window, existing = false) {
        if (!window || !this._enabled || this._windows.has(window)) return;
        const entry = {signals: [], actor: null, actorSignals: [], original: null, inhibited: false, secondary: -1,
            painted: existing && Boolean(window.get_compositor_private()?.mapped)};
        this._windows.set(window, entry);
        const connect = (name, callback) => entry.signals.push(window.connect(name, callback));
        for (const signal of ['notify::wm-class', 'notify::gtk-application-id', 'notify::window-type'])
            connect(signal, () => this._consider(window));
        connect('shown', () => this._consider(window));
        connect('position-changed', () => { if (entry.original) this._queueGeometry(); });
        connect('size-changed', () => { if (entry.original) this._queueGeometry(); });
        connect('raised', () => { if (entry.original && this._enabled) window.lower(); });
        connect('notify::above', () => { if (entry.original && window.is_above()) window.unmake_above(); });
        connect('unmanaged', () => {
            this._inhibitedDing.delete(window);
            const pid = window.get_pid();
            this._unwatchWindow(window);
            // Only the desktop window takes its pid along: Start, a popup menu or a dialog of the same
            // win11-shell is unmanaged on every close, and the shell reports ready once per desktop window.
            if (entry.original && entry.secondary < 0 &&
                ![...this._windows].some(([other, e]) => e.original && e.secondary < 0 && other.get_pid() === pid))
                this._readyPids.delete(pid);
            this._sync();
        });
        this._consider(window);
    }

    _consider(window) {
        const entry = this._windows.get(window);
        if (!this._enabled || !entry) return;
        if (this._ours(window)) {
            entry.secondary = secondaryDesktop(window);
            if (!entry.original) {
                entry.original = {type: window.get_window_type(), above: window.is_above(),
                    sticky: window.is_on_all_workspaces(), skip: window.skip_taskbar};
                window.set_type(Meta.WindowType.DESKTOP);
                window.hide_from_window_list();
                window.stick();
                if (window.is_above()) window.unmake_above();
                window.lower();
            }
            const actor = window.get_compositor_private();
            if (actor && actor !== entry.actor) {
                this._dropActor(entry);
                entry.actor = actor;
                if (!actor.mapped) Main.wm.skipNextEffect(actor);
                entry.actorSignals.push([actor, actor.connect('first-frame', () => { entry.painted = true; this._sync(); })]);
                entry.actorSignals.push([actor, actor.connect('notify::mapped', () => this._sync())]);
                // Mutter destroys the actor before the window's 'unmanaged': its handlers go with it.
                entry.actorSignals.push([actor, actor.connect('destroy', () => {
                    if (entry.actor === actor) { entry.actorSignals = []; entry.actor = null; }
                })]);
            }
            this._queueGeometry();
        }
        this._sync();
    }

    ready(args) {
        if (!Number.isInteger(args.pid) || args.pid <= 0 || args.pid > 0x7fffffff)
            throw new Error(this._t('badPid'));
        // Once per desktop window: an older win11-shell repeats this every 2 s. A re-map, a new actor or a
        // monitor change lowers and sizes the desktop through its own signals.
        if (this._readyPids.has(args.pid)) return {ready: true, present: this._present};
        if (this._readyPids.size > 512) this._readyPids.clear();
        this._readyPids.add(args.pid);
        for (const window of this._windows.keys()) if (this.isDesktop(window)) this._consider(window);
        this._sync();
        return {ready: true, present: this._present};
    }

    _t(key) {
        return TEXT[key][this._language()] ?? TEXT[key].nl;
    }

    /** The desktop window of another monitor, if any. */
    _secondaryOf(index) {
        return [...this._windows].find(([w, e]) => e.original && e.secondary === index && secondaryDesktop(w) === index)?.[0] ?? null;
    }

    /**
     * Where the desktop goes: the primary monitor (a desktop window stays in one monitor's work area, so it never
     * spanned the others), with `areas`, every monitor (its stage position and logical size, the primary marked),
     * for win11-shell's wallpaper windows of the other monitors. With one monitor this is the bounds it always was.
     */
    geometry() {
        const monitors = Main.layoutManager.monitors;
        if (!monitors.length) return {available: false, x: 0, y: 0, w: 0, h: 0, logical: {w: 0, h: 0}};
        const primary = Main.layoutManager.primaryMonitor ?? monitors[0];
        const rect = new Mtk.Rectangle({x: primary.x, y: primary.y, width: primary.width, height: primary.height});
        const window = [...this._windows.keys()].find(w => this.isDesktop(w));
        let protocol = rect;
        try { protocol = window?.stage_to_protocol_rect(rect) ?? rect; } catch { /* Before mapping. */ }
        const areas = monitors.map(m => {
            const own = m.index === primary.index ? window : this._secondaryOf(m.index);
            const scale = global.display.get_monitor_scale(m.index) || 1;
            let size = {w: Math.round(m.width / scale), h: Math.round(m.height / scale)};
            try {
                const p = own?.stage_to_protocol_rect(new Mtk.Rectangle({x: m.x, y: m.y, width: m.width, height: m.height}));
                if (p?.width > 0 && p?.height > 0) size = {w: p.width, h: p.height};
            } catch { /* Before mapping: the advertised scale. */ }
            return {index: m.index, primary: m.index === primary.index, x: m.x, y: m.y, ...size};
        });
        return {available: true, ...rectJSON(rect), width: rect.width, height: rect.height, logical: rectJSON(protocol),
            monitor: primary.index, monitors: monitors.map(m => ({index: m.index, ...rectJSON(m)})), areas};
    }

    _queueGeometry() {
        if (this._geometryId || this._positioning || !this._enabled) return;
        this._geometryId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._geometryId = 0;
            const geometry = this.geometry();
            if (!geometry.available) return GLib.SOURCE_REMOVE;
            const rect = new Mtk.Rectangle({x: geometry.x, y: geometry.y, width: geometry.w, height: geometry.h});
            this._positioning = true;
            try {
                for (const [window, entry] of this._windows) {
                    if (!entry.original || !this._ours(window)) continue;
                    // the desktop on the primary monitor, a desktop window of another monitor on its own
                    let target = rect;
                    if (entry.secondary >= 0) {
                        const m = Main.layoutManager.monitors[entry.secondary];
                        if (!m) continue;
                        target = new Mtk.Rectangle({x: m.x, y: m.y, width: m.width, height: m.height});
                    }
                    if (!sameRect(window.get_frame_rect(), target))
                        window.move_resize_frame(false, target.x, target.y, target.width, target.height);
                    window.lower();
                }
            } finally { this._positioning = false; }
            this._emit('desktop.geometry', geometry);
            this._sync();
            return GLib.SOURCE_REMOVE;
        });
    }

    _sync() {
        if (!this._enabled) return;
        const userSession = this._userSession();
        let present = false;
        for (const [window, entry] of this._windows) {
            if (!entry.original || !this._ours(window)) continue;
            if (!userSession && !entry.inhibited) { entry.inhibited = true; window.inhibit_mapped(); }
            else if (userSession && entry.inhibited) { entry.inhibited = false; window.uninhibit_mapped(); }
            if (entry.secondary >= 0) continue;
            const actor = window.get_compositor_private();
            if (userSession && entry.painted && actor?.mapped && this._readyPids.has(window.get_pid())) present = true;
        }
        for (const window of [...this._inhibitedDing]) {
            if (!present || !this._isDing(window)) {
                this._inhibitedDing.delete(window);
                window.uninhibit_mapped();
            }
        }
        if (present) {
            for (const window of this._windows.keys()) {
                if (this._isDing(window) && !this._inhibitedDing.has(window)) {
                    window.inhibit_mapped();
                    this._inhibitedDing.add(window);
                }
            }
        }
        this._syncHotCorners(present);
        if (this._present !== present) {
            this._present = present;
            this._emit('desktop.changed', {present});
        }
    }

    _syncHotCorners(active) {
        // GNOME's pressure barriers otherwise open Activities while selecting the
        // desktop's top-left icons. Lease their size and drag handler,
        // without changing the user's hot-corner preference.
        if (!active) {
            for (const [corner, state] of this._hotCorners) {
                if (corner.setBarrierSize === state.wrapper) {
                    corner.setBarrierSize = state.original;
                    corner.setBarrierSize(state.size);
                }
                if (corner._toggleOverview === state.noOverview) corner._toggleOverview = state.toggle;
                corner.disconnect(state.destroyId);
            }
            this._hotCorners.clear();
            return;
        }
        for (const corner of Main.layoutManager.hotCorners) {
            if (!corner || this._hotCorners.has(corner)) continue;
            const state = {original: corner.setBarrierSize, toggle: corner._toggleOverview,
                size: Main.layoutManager.panelBox.height};
            state.wrapper = size => { state.size = size; state.original.call(corner, 0); };
            state.noOverview = () => {};
            state.destroyId = corner.connect('destroy', () => this._hotCorners.delete(corner));
            this._hotCorners.set(corner, state);
            corner.setBarrierSize = state.wrapper;
            corner._toggleOverview = state.noOverview;
            state.original.call(corner, 0);
        }
    }

    _unwatchWindow(window) {
        const entry = this._windows.get(window);
        if (!entry) return;
        // The entry goes first, so a failing disconnect can never leave a stale one behind.
        this._windows.delete(window);
        for (const id of entry.signals) window.disconnect(id);
        this._dropActor(entry);
    }

    _dropActor(entry) {
        for (const [actor, id] of entry.actorSignals) {
            try { actor.disconnect(id); } catch { /* Disposed meanwhile. */ }
        }
        entry.actorSignals = [];
        entry.actor = null;
    }

    disable() {
        if (!this._enabled) return;
        this._enabled = false;
        this._syncHotCorners(false);
        if (this._geometryId) GLib.source_remove(this._geometryId);
        this._geometryId = 0;
        for (const [object, id] of this._signals) object.disconnect(id);
        for (const window of this._inhibitedDing) window.uninhibit_mapped();
        this._inhibitedDing.clear();
        for (const [window, entry] of this._windows) {
            if (!entry.original) continue;
            if (entry.inhibited) window.uninhibit_mapped();
            // The surface belongs to our shell. Ordinary apps never enter this path.
            window.set_type(entry.original.type);
            if (!entry.original.sticky) window.unstick();
            if (!entry.original.skip) window.show_in_window_list();
            if (entry.original.above) window.make_above();
        }
        for (const window of [...this._windows.keys()]) this._unwatchWindow(window);
        // win11-shell closes its desktop window and keeps the name; it shows it again when the bridge returns.
        Gio.DBus.session.call(DESKTOP_APP_ID, '/io/github/win11shell/Desktop', 'org.gtk.Actions', 'Activate',
            new GLib.Variant('(sava{sv})', ['desktop-quit', [], {}]), null,
            Gio.DBusCallFlags.NO_AUTO_START, 3000, null, (connection, reply) => {
                try { connection.call_finish(reply); } catch { /* Already absent or closing. */ }
            });
        this._readyPids.clear();
        this._present = false;
    }
}
