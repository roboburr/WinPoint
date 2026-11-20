// SPDX-License-Identifier: MIT
// Native compositor backdrops: the taskbar strip, the focused own app window and the open Start window
// (D-G59; before that app blur was an experiment and Start had none). At most one window blur exists at a time.
// Actor-mode blur composes only window snapshots below the glass, never the live screen framebuffer.
// D-G60: Start's registration survives its hidden window; an app window's backdrop is bound to its actor.
// D-G63: a pid holds a set of identities; the native shell (win11-shell) registers its taskbar and Start under one pid.
import Clutter from 'gi://Clutter';
import Shell from 'gi://Shell';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';

// The taskbars of the other monitors (win11-shell's io.github.win11shell.SecondaryTaskbar.Monitor<N> windows): one
// identity for all of them, a surface of the native shell like the panel.
const STRIP = 'io.github.win11shell.SecondaryTaskbar';
const STRIP_ID = /^io\.github\.win11shell\.SecondaryTaskbar\.Monitor\d{1,2}$/;
// A transient Picker retains its parent's GTK application ID but has its own
// Wayland protocol role. Prefer that role so it never inherits the main app effect.
const windowIdentity = window => {
    const protocol = window.get_wm_class() || '';
    const gtk = window.get_gtk_application_id() || '';
    if (protocol === 'io.github.win11shell.FilePicker' || gtk === 'io.github.win11shell.FilePicker')
        return 'io.github.win11shell.FilePicker';
    if (STRIP_ID.test(protocol))
        return STRIP;
    // the desktop windows of the other monitors are the desktop's (desktopBridge.js)
    if (/^io\.github\.win11shell\.SecondaryDesktop\.Monitor\d{1,2}$/.test(protocol))
        return 'io.github.win11shell.Desktop';
    return protocol || gtk;
};
// i18n: pairs (tools/i18n/check_pairs.py): texts in Win11's language (contracts C1, C3).
const TEXT = {
    badIdentity: {nl: 'Ongeldige identiteit voor de vensterachtergrond.', en: 'Invalid identity for the window backdrop.'},
    identityMismatch: {nl: 'De identiteit van de achtergrond past niet bij het venster.', en: 'The backdrop identity doesn’t match the window.'},
};
const OWN_APPS = new Set([
    'io.github.win11shell.Explorer', 'io.github.win11shell.Settings',
    'io.github.win11shell.TaskManager', 'io.github.win11shell.Search',
    'io.github.win11shell.Disks', 'io.github.win11shell.DefaultApps', 'io.github.win11shell.Installer',
    'io.github.win11shell.Calendar',
]);
const START = 'io.github.win11shell.Start';
const PANEL = 'io.github.win11shell.Panel';
// The surfaces one native shell process draws (D-G63): each may share the pid of the others.
const SHELL_SURFACES = new Set([PANEL, START, 'io.github.win11shell.Desktop', STRIP]);
const START_PHASES = ['open', 'closing', 'hidden'];
// Start's own entrance (core/flyouts.js, START_ADAPTER): the glass rises 40 px in 250 ms, decelerating.
const START_RISE = 40, START_RISE_MS = 250;

/** An actor's current stage rectangle, rounded (status() of a bound backdrop). */
const live = actor => {
    const [x, y] = actor.get_transformed_position();
    const [w, h] = actor.get_transformed_size();
    return {x: Math.round(x), y: Math.round(y), w: Math.round(w), h: Math.round(h)};
};

export class NativeFrostedBridge {
    constructor({panel = () => null, geometry = () => null,
        windows = () => [], available = () => false, taskbarAutoHide = () => false,
        onChanged = () => {}, registeredOnly = false, language = () => 'nl', strips = () => []} = {}) {
        this._panel = panel;
        // The shown taskbars of the other monitors: [{window, monitor}]. Their glass follows the panel's
        // registration (same pid, the panel's options); none means every path below is as it was.
        this._strips = strips;
        this._language = language;
        this._geometry = geometry;
        this._windows = windows;
        this._available = available;
        this._taskbarAutoHide = taskbarAutoHide;
        this._enabled = false;
        this._onChanged = onChanged;
        this._registeredOnly = registeredOnly;
        // pid -> Set of registered identities (D-G63).
        this._hosts = new Map();
        this._userTransparency = true;
        this._userHighContrast = false;
        this._userAutoHide = false;
        this._generation = String(GLib.get_monotonic_time());
        this._serial = 0;
        this._lastState = '';
        this._options = {blur: true, windowBlur: true, shadows: true, taskbarAutoHide: false, radius: 6};
        this._entries = new Map();
        this._sources = new Map();
        this._sourceCaptures = 0;
        // Start's window stays mapped while its glass slides out; the host reports when the menu opens and
        // starts closing (nativeBackdrop.start), so the blur exists only while the menu is really open.
        this._startPhase = new Map();
    }

    configure({enabled = false, blur = true, windowBlur = true, shadows = true, taskbarAutoHide = false, radius = 6} = {}) {
        this._enabled = enabled === true;
        this._options = {blur: blur === true, windowBlur: windowBlur === true, shadows: shadows === true,
            taskbarAutoHide: taskbarAutoHide === true,
            radius: Math.max(1, Math.min(32, Math.round(radius) || 6))};
        this.sync();
        return this.status();
    }

    register(args) {
        const identities = {panel: 'io.github.win11shell.Panel', start: START, explorer: 'io.github.win11shell.Explorer',
            settings: 'io.github.win11shell.Settings', taskmgr: 'io.github.win11shell.TaskManager',
            alles: 'io.github.win11shell.Search', disks: 'io.github.win11shell.Disks',
            defaultapps: 'io.github.win11shell.DefaultApps', installer: 'io.github.win11shell.Installer',
            calendar: 'io.github.win11shell.Calendar'};
        const identity = identities[args.kind];
        if (!identity || !Number.isSafeInteger(args.pid) || args.pid <= 0)
            throw new Error(this._t('badIdentity'));
        // Native HTTP inserts its own PID and role; browser-provided identity is never forwarded. A shell
        // surface may share its pid with the other shell surfaces (win11-shell), never with an app.
        const allowed = SHELL_SURFACES.has(identity) ? [...SHELL_SURFACES] : [identity];
        allowed.push('io.github.win11shell.FilePicker');
        const candidates = this._windows().filter(w => w.get_pid() === args.pid);
        if (candidates.some(w => w.get_window_type() === Meta.WindowType.NORMAL &&
            !allowed.includes(windowIdentity(w))))
            throw new Error(this._t('identityMismatch'));
        if (!this._hosts.has(args.pid)) this._hosts.set(args.pid, new Set());
        this._hosts.get(args.pid).add(identity);
        this._userTransparency = args.transparency === true;
        this._userHighContrast = args.highContrast === true;
        this._userAutoHide = args.autoHide === true;
        this.sync();
        return this.capabilities();
    }

    _t(key) {
        return TEXT[key][this._language()] ?? TEXT[key].nl;
    }

    /** The pid registered this identity. */
    _has(pid, identity) {
        return this._hosts.get(pid)?.has(identity) === true;
    }

    /** nativeBackdrop.start {pid, phase}: the registered Start host's menu opened, began closing or is hidden. */
    start(args) {
        if (!Number.isSafeInteger(args.pid) || !this._has(args.pid, START) || !START_PHASES.includes(args.phase))
            throw new Error(this._t('badIdentity'));
        this._startPhase.set(args.pid, args.phase);
        this.sync();
        return {phase: args.phase};
    }

    forget(pid) {
        // A native picker shares its app's PID. Retain the main window's registration
        // when that child closes; only the last eligible matching window releases it. The native shell
        // registers several identities under one pid: each is released by its own windows (D-G63).
        const identities = this._hosts.get(pid);
        if (identities) {
            const windows = this._windows().filter(w => w.get_pid() === pid);
            for (const identity of [...identities]) {
                if (windows.some(w => windowIdentity(w) === identity &&
                    (w.get_window_type() === Meta.WindowType.NORMAL || identity === PANEL)))
                    continue;
                // Resident Start hides its window on every close, which unmanages it on Wayland; the next open
                // brings a new window under the registration its page made once. Dropping it here left only the
                // first open with glass (D-G60).
                if (identity === START && GLib.file_test(`/proc/${pid}`, GLib.FileTest.EXISTS))
                    this._startPhase.set(pid, 'hidden');
                else
                    identities.delete(identity);
            }
            if (!identities.size)
                this._hosts.delete(pid);
        }
        if (!this._has(pid, START))
            this._startPhase.delete(pid);
        this.sync();
    }

    capabilities() {
        const available = this._enabled && this._available() && !this._userHighContrast;
        const windows = [];
        for (const window of this._windows()) {
            const pid = window.get_pid();
            const id = windowIdentity(window);
            if (!this._has(pid, id) || (!id.endsWith('.Panel') && window.get_window_type() !== Meta.WindowType.NORMAL)) continue;
            const entry = this._entries.get(window);
            const kind = id.endsWith('.Panel') ? 'taskbar' : id === START ? 'start' : 'window';
            // Start's glass is translucent whenever it may be blurred, so it never switches while the menu opens.
            const blur = kind === 'start' ? this._startEligible() : Boolean(entry?.effect);
            windows.push({pid, kind, blur: Boolean(available && blur), shadow: Boolean(available && entry?.shadowOption),
                focused: global.display.focus_window === window});
        }
        for (const {window, monitor} of this._strips()) {
            const pid = window.get_pid();
            if (!this._has(pid, PANEL)) continue;
            windows.push({pid, kind: 'taskbar-secondary', monitor,
                blur: Boolean(available && this._entries.get(window)?.effect), shadow: false, focused: false});
        }
        // Hidden resident Start has no window, but its page keeps the translucent glass for the next open.
        for (const [pid, ids] of this._hosts) {
            if (ids.has(START) && !windows.some(w => w.pid === pid && w.kind === 'start'))
                windows.push({pid, kind: 'start', blur: Boolean(available && this._startEligible()), shadow: false, focused: false});
        }
        return {available, generation: this._generation, serial: this._serial, windows};
    }

    _publish() {
        const state = this.capabilities();
        const fingerprint = JSON.stringify({available: state.available, windows: state.windows});
        if (fingerprint === this._lastState) return;
        this._lastState = fingerprint;
        state.serial = ++this._serial;
        this._onChanged(state);
    }

    suspend(window) {
        // Actor notifications can arrive inside a compositor animation frame. Remove
        // synchronously; the coalesced sync may restore only a stable final frame.
        this._destroy(window);
        this._publish();
    }

    /** A window with a backdrop moved or changed size: invalidate the old shadow bounds. GNOME redraws only the damaged
     *  rectangle around the moving window, but the shadow (box-shadow) and the blur reach beyond it, so on real
     *  hardware the old shadow stayed and the rectangle showed as a box around it (Robin, 13-11: "schaduw
     *  blijft hangen", "boxed effect rondom de schaduw"). Blur my Shell's "hack level" does the same. */
    damage(window) {
        if (this._entries.has(window))
            global.stage.queue_redraw();
    }

    /** Where the window is drawn: its frame when the actor sits on it (±2 px), else the actor itself (during a
     *  drag Mutter can draw the window a frame behind its frame; the glass belongs under what is drawn). */
    _rect(window, actor) {
        const r = window.get_frame_rect();
        if (this._fits(actor, r)) return {x: r.x, y: r.y, w: r.width, h: r.height};
        const [ax, ay] = actor.get_transformed_position();
        const [aw, ah] = actor.get_transformed_size();
        return {x: Math.round(ax), y: Math.round(ay), w: Math.round(aw), h: Math.round(ah)};
    }

    /** No Mutter effect (map, minimize, fade) is scaling, sliding or fading the actor. */
    _stable(actor) {
        return actor.opacity === 255 && Math.abs(actor.scale_x - 1) <= 0.001 &&
            Math.abs(actor.scale_y - 1) <= 0.001 && Math.abs(actor.translation_x) <= 0.1 &&
            Math.abs(actor.translation_y) <= 0.1 && Math.abs(actor.translation_z) <= 0.1 &&
            Math.abs(actor.rotation_angle_z) <= 0.1;
    }

    /** The actor is where the window's frame is (±2 px). */
    _fits(actor, r) {
        const [ax, ay] = actor.get_transformed_position();
        const [aw, ah] = actor.get_transformed_size();
        return Math.abs(ax - r.x) <= 2 && Math.abs(ay - r.y) <= 2 &&
            Math.abs(aw - r.width) <= 2 && Math.abs(ah - r.height) <= 2;
    }

    _destroy(key) {
        const entry = this._entries.get(key);
        if (!entry) return;
        this._entries.delete(key);
        this._disconnectScene(entry);
        if (entry.effect) entry.backdrop.remove_effect(entry.effect);
        entry.effect = null;
        entry.outer.destroy();
    }

    _startEligible() {
        return this._options.blur && this._options.windowBlur && this._userTransparency && !this._userHighContrast;
    }

    _desired() {
        const desired = new Map();
        if (!this._enabled || !this._available() || !this._userTransparency || this._userHighContrast) return desired;
        const panel = this._panel();
        const geometry = this._geometry();
        const panelActor = panel?.get_compositor_private();
        if (panelActor?.mapped && panelActor.visible && geometry?.available && this._options.blur &&
            !this._options.taskbarAutoHide && !this._taskbarAutoHide() && !this._userAutoHide &&
            (!this._registeredOnly || this._has(panel.get_pid(), PANEL))) {
            // Never create a full-screen backdrop for the transparent Panel host.
            const h = Math.min(geometry.h, Math.max(1, geometry.barHeight));
            desired.set(panel, {window: panel, actor: panelActor, kind: 'taskbar',
                rect: {x: geometry.x, y: geometry.y + geometry.h - h, w: geometry.w, h}});
        }
        if (this._options.blur && !this._options.taskbarAutoHide && !this._taskbarAutoHide() && !this._userAutoHide) {
            // A taskbar of another monitor is its whole window: the backdrop is its frame.
            for (const {window} of this._strips()) {
                const actor = window.get_compositor_private();
                const r = window.get_frame_rect();
                if (!actor?.mapped || !actor.visible || r.width <= 1 || r.height <= 1 ||
                    (this._registeredOnly && !this._has(window.get_pid(), PANEL))) continue;
                desired.set(window, {window, actor, kind: 'strip', rect: {x: r.x, y: r.y, w: r.width, h: r.height}});
            }
        }
        if (this._startEligible()) {
            for (const window of this._windows()) {
                if (windowIdentity(window) !== START || !this._has(window.get_pid(), START) ||
                    this._startPhase.get(window.get_pid()) !== 'open' || window.minimized ||
                    window.get_window_type() !== Meta.WindowType.NORMAL) continue;
                const actor = window.get_compositor_private();
                const r = window.get_frame_rect();
                if (!actor?.mapped || !actor.visible || !this._stable(actor) || r.width <= 1 || r.height <= 1) continue;
                desired.set(window, {window, actor, kind: 'start', rect: {x: r.x, y: r.y, w: r.width, h: r.height}});
            }
        }
        if ((this._options.blur && this._options.windowBlur) || this._options.shadows) {
            for (const window of this._windows()) {
                const id = windowIdentity(window);
                if (!OWN_APPS.has(id) || (this._registeredOnly && !this._has(window.get_pid(), id)) || window.minimized || window.is_fullscreen() ||
                    window.get_window_type() !== Meta.WindowType.NORMAL || !window.showing_on_its_workspace()) continue;
                const focused = global.display.focus_window === window && !id.endsWith('.Search');
                // The existing UI intentionally makes inactive Mica opaque. Allocating
                // another full-frame blur there is invisible and wastes compositor RAM.
                if ((!this._options.blur || !this._options.windowBlur || !focused) &&
                    (!this._options.shadows || window.is_maximized())) continue;
                const actor = window.get_compositor_private();
                if (!actor?.mapped || !actor.visible) continue;
                // A sibling shadow must never remain at the final frame while Mutter
                // scales, moves or fades its window. Suspend until the actor is stable.
                if (!this._stable(actor)) continue;
                const rect = this._rect(window, actor);
                if (rect.w <= 1 || rect.h <= 1) continue;
                desired.set(window, {window, actor, kind: 'window', rect});
            }
        }
        return desired;
    }

    _new(item) {
        const outer = new St.Widget({reactive: false, can_focus: false,
            name: item.kind === 'taskbar' ? 'win11-frosted-taskbar' : item.kind === 'strip' ? 'win11-frosted-strip'
                : item.kind === 'start' ? 'win11-frosted-start' : 'win11-frosted-own-window'});
        const bar = item.kind === 'taskbar' || item.kind === 'strip';
        const backdrop = bar ? outer : new St.Widget({reactive: false, can_focus: false});
        if (backdrop !== outer) outer.add_child(backdrop);
        const entry = {outer, backdrop, effect: null, focused: null, kind: item.kind, rect: null};
        this._entries.set(item.window, entry);
        this._created = (this._created ?? 0) + 1;
        outer.connect('destroy', () => {
            this._disconnectScene(entry, false);
            // A compositor shutdown can dispose the containing window group before
            // the extension's disable callback; never revisit an already dead actor.
            if (this._entries.get(item.window) === entry) {
                this._entries.delete(item.window);
            }
            entry.effect = null;
        });
        return entry;
    }

    _queueSceneSync() {
        if (!this._enabled || this._sceneSyncId) return;
        this._sceneSyncId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._sceneSyncId = 0;
            this.sync();
            return GLib.SOURCE_REMOVE;
        });
    }

    _dropSource(source, destroyed = false) {
        const record = this._sources.get(source);
        if (!record) return;
        this._sources.delete(source);
        if (record.timer) GLib.Source.remove(record.timer);
        if (!destroyed) for (const id of record.signals) source.disconnect(id);
        record.content = null;
    }

    _pruneSources() {
        const used = new Set([...this._entries.values()].flatMap(entry => [...(entry.layers?.keys() ?? [])]));
        for (const source of this._sources.keys()) if (!used.has(source)) this._dropSource(source);
    }

    _disconnectScene(entry, destroyLayers = true) {
        if (destroyLayers) for (const layer of entry.layers?.values() ?? []) layer.destroy();
        entry.layers?.clear();
        this._pruneSources();
    }

    _windowContent(source) {
        let record = this._sources.get(source);
        if (record) return record.content;
        record = {content: null, timer: 0, signals: []};
        this._sources.set(source, record);
        const capture = () => {
            record.timer = 0;
            let content;
            try { content = source.paint_to_content(null); } catch { content = null; }
            if (content) {
                record.content = content;
                this._sourceCaptures++;
                for (const entry of this._entries.values()) {
                    const layer = entry.layers?.get(source);
                    if (layer) layer.set_content(content);
                }
            }
            return GLib.SOURCE_REMOVE;
        };
        const damaged = () => {
            if (!record.timer) record.timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 33, capture);
        };
        record.signals.push(source.connect('damaged', damaged));
        for (const prop of ['x', 'y', 'width', 'height', 'scale-x', 'scale-y',
            'translation-x', 'translation-y', 'opacity', 'visible', 'mapped']) {
            record.signals.push(source.connect(`notify::${prop}`, () => {
                if (prop === 'width' || prop === 'height') damaged();
                this._queueSceneSync();
            }));
        }
        record.signals.push(source.connect('destroy', () => {
            this._dropSource(source, true);
            this._queueSceneSync();
        }));
        capture();
        return record.content;
    }

    /** Compose desktop and visible windows below this client in their actual stacking order. Snapshots
     * inhibit culling; a covered client still supplies its complete pixels. Sibling glass and this client's
     * own pixels are never captured, so hover/dropdown redraws cannot feed back into their own blur. */
    _scene(entry, item, parent) {
        const desktop = this._windows().find(w => windowIdentity(w) === 'io.github.win11shell.Desktop' &&
            w.get_monitor() === item.window.get_monitor() && w.get_compositor_private()?.mapped);
        const base = desktop?.get_compositor_private();
        if (!base || base.width <= 0 || base.height <= 0) return false;
        const rect = entry.blurRect;
        const x0 = item.rect.x + rect.x, y0 = item.rect.y + rect.y;
        const children = parent.get_children();
        const target = children.indexOf(item.actor);
        if (target < 0) return false;
        const sources = [base];
        for (const actor of children.slice(0, target)) {
            const w = actor.meta_window;
            if (!w || actor === base || w === item.window || !actor.mapped || !actor.visible ||
                !actor.opacity || w.minimized || !w.showing_on_its_workspace() ||
                windowIdentity(w) === 'io.github.win11shell.Desktop') continue;
            const [x, y] = actor.get_transformed_position();
            const [width, height] = actor.get_transformed_size();
            if (x + width <= x0 || y + height <= y0 || x >= x0 + rect.w || y >= y0 + rect.h) continue;
            sources.push(actor);
        }
        entry.layers ??= new Map();
        const used = new Set();
        for (const source of sources) {
            const content = this._windowContent(source);
            if (!content) continue;
            used.add(source);
            let layer = entry.layers.get(source);
            if (!layer) {
                layer = new Clutter.Actor({reactive: false, content});
                entry.layers.set(source, layer);
                entry.backdrop.add_child(layer);
            }
            const [x, y] = source.get_transformed_position();
            let [width, height] = source.get_transformed_size();
            if (source === base) {
                // Desktop actors stop at the workarea. Extend their edge under the taskbar.
                const zoom = Math.max(1, rect.w / width, rect.h / height);
                width *= zoom; height *= zoom;
                layer.set_position(-Math.max(0, Math.min(x0 - x, width - rect.w)),
                    -Math.max(0, Math.min(y0 - y, height - rect.h)));
            } else layer.set_position(x - x0, y - y0);
            layer.set_size(width, height);
            layer.opacity = source.opacity;
        }
        for (const [source, layer] of entry.layers) if (!used.has(source)) {
            layer.destroy(); entry.layers.delete(source);
        }
        const ordered = sources.filter(source => used.has(source)).map(source => entry.layers.get(source));
        for (let i = 0; i < ordered.length; i++)
            if (entry.backdrop.get_children()[i] !== ordered[i])
                entry.backdrop.set_child_at_index(ordered[i], i);
        entry.wallpaperKind = 'windows';
        return used.has(base);
    }

    sync() {
        const desired = this._desired();
        for (const key of [...this._entries.keys()]) if (!desired.has(key)) this._destroy(key);
        for (const [key, item] of desired) {
            const parent = item.actor.get_parent();
            if (!parent) { this._destroy(key); continue; }
            const entry = this._entries.get(key) ?? this._new(item);
            if (entry.outer.get_parent() !== parent) {
                entry.outer.get_parent()?.remove_child(entry.outer);
                parent.insert_child_below(entry.outer, item.actor);
            } else {
                parent.set_child_below_sibling(entry.outer, item.actor);
            }
            const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
            const focused = global.display.focus_window === item.window;
            this._place(entry, item, parent);
            const shadowOn = this._options.shadows && item.kind === 'window' && !item.window.is_maximized();
            if (entry.focused !== focused || entry.shadowOption !== shadowOn) {
                const shadow = shadowOn
                    ? (focused ? '0 8px 26px 0 rgba(0,0,0,0.34)' : '0 4px 14px 0 rgba(0,0,0,0.20)') : 'none';
                const radius = item.kind === 'taskbar' || item.kind === 'strip' ? 0 : 8;
                entry.outer.set_style(`background-color: rgba(28,28,28,0.01); border-radius: ${radius}px; box-shadow: ${shadow};`);
                entry.focused = focused;
                entry.shadowOption = shadowOn;
            }
            let blurOn = this._options.blur && (item.kind === 'taskbar' || item.kind === 'strip' || item.kind === 'start' ||
                (this._options.windowBlur && focused && !(windowIdentity(item.window)).endsWith('.Search')));
            blurOn = blurOn && this._scene(entry, item, parent);
            if (!blurOn) this._disconnectScene(entry);
            if (blurOn && !entry.effect) {
                entry.effect = new Shell.BlurEffect({mode: Shell.BlurMode.ACTOR,
                    radius: this._options.radius * scale, brightness: 1});
                entry.backdrop.add_effect_with_name('win11-frosted-backdrop', entry.effect);
                if (item.kind === 'start') {
                    // Rise with the glass; the window clips the part still below it.
                    entry.outer.clip_to_allocation = true;
                    entry.backdrop.translation_y = START_RISE * scale;
                    entry.backdrop.ease({translation_y: 0, duration: START_RISE_MS, mode: Clutter.AnimationMode.EASE_OUT_EXPO});
                }
            } else if (!blurOn && entry.effect) {
                entry.backdrop.remove_effect(entry.effect);
                entry.effect = null;
            }
            if (entry.effect && entry.effect.radius !== this._options.radius * scale)
                entry.effect.radius = this._options.radius * scale;
            entry.outer.show();
        }
        this._pruneSources();
        this._publish();
    }

    /** Position and size an entry's backdrop for item.rect (stage coordinates) inside its window's parent. */
    _place(entry, item, parent) {
        const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        // Keep the rectangular native backdrop inside the 8 px rounded corners (--radius-overlay): a corner
        // of the rectangle inset by r(1 - 1/sqrt 2) lies on the arc, so the unblurred rim is 3 px instead of
        // the 8 px it was. A maximized window has square corners. The page continues to paint its chrome.
        const rounded = item.kind === 'start' || (item.kind === 'window' && !item.window.is_maximized());
        const inset = rounded ? Math.min(Math.ceil(8 * scale * (1 - Math.SQRT1_2)), item.rect.w / 4, item.rect.h / 4) : 0;
        const blurRect = {x: inset, y: inset, w: item.rect.w - inset * 2, h: item.rect.h - inset * 2};
        if (item.kind === 'window') {
            // An app window's shadow and glass are bound to its actor (Clutter.BindConstraint, as Rounded Window
            // Corners binds its shadow): Clutter moves and sizes them in the same layout pass as the window, so
            // nothing trails or stays behind during a drag (Robin, 13-11: "schaduw blijft soms hangen"). Before
            // that the extension moved them itself after each step (D-G60).
            if (entry.boundTo !== item.actor) {
                entry.outer.clear_constraints();
                entry.outer.add_constraint(new Clutter.BindConstraint({source: item.actor, coordinate: Clutter.BindCoordinate.POSITION}));
                entry.outer.add_constraint(new Clutter.BindConstraint({source: item.actor, coordinate: Clutter.BindCoordinate.SIZE}));
                entry.inner = new Clutter.BindConstraint({source: entry.outer, coordinate: Clutter.BindCoordinate.SIZE});
                entry.backdrop.clear_constraints();
                entry.backdrop.add_constraint(entry.inner);
                entry.boundTo = item.actor;
            }
            entry.inner.offset = -2 * inset;
            entry.backdrop.set_position(inset, inset);
            entry.backdrop.remove_clip();
            entry.backdrop.clip_to_allocation = true;
            entry.rect = {...item.rect};
            entry.blurRect = {...blurRect};
            return;
        }
        const [ok, px, py] = parent.transform_stage_point(item.rect.x, item.rect.y);
        entry.outer.set_position(ok ? px : item.rect.x, ok ? py : item.rect.y);
        entry.outer.set_size(item.rect.w, item.rect.h);
        if (entry.backdrop !== entry.outer) {
            entry.backdrop.set_position(blurRect.x, blurRect.y);
            entry.backdrop.set_size(blurRect.w, blurRect.h);
        }
        entry.backdrop.set_clip(0, 0, blurRect.w, blurRect.h);
        entry.backdrop.clip_to_allocation = true;
        entry.rect = {...item.rect};
        entry.blurRect = {...blurRect};
    }

    status() {
        return {experimental: true, enabled: this._enabled, sources: this._sources.size, sourceCaptures: this._sourceCaptures, options: {...this._options}, created: this._created ?? 0,
            entries: [...this._entries.values()].map(e => ({kind: e.kind, focused: e.focused,
                rect: e.boundTo ? live(e.outer) : e.rect, blurRect: e.blurRect, effect: Boolean(e.effect), source: e.wallpaperKind ?? null, layers: e.layers?.size ?? 0, mode: e.effect?.mode ?? null, shadow: e.shadowOption,
                reactive: e.outer.reactive, clips: e.backdrop.clip_to_allocation}))};
    }

    disable() {
        this._enabled = false;
        if (this._sceneSyncId) GLib.Source.remove(this._sceneSyncId);
        this._sceneSyncId = 0;
        for (const key of [...this._entries.keys()]) this._destroy(key);
        this._publish();
        this._hosts.clear();
    }
}
