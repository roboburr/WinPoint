// A small fake GNOME Shell 50 world for the GE1c extension text harness. It implements only what the extension's
// enable(), labels, notifications and failure paths touch; anything else falls back to inert auto-stubs. Real
// files are read from the fake XDG config dir the test sets up (never the user's).
import fs from 'node:fs';
import path from 'node:path';

export const world = {
    log: [], monitors: [], timers: new Map(), nextTimer: 1, now: 1, groups: [], apps: {}, actors: [],
    configDir: '/nonexistent/config', home: '/nonexistent/home', env: {},
};
const rec = (kind, data = {}) => world.log.push({kind, ...data});

function auto(name) {
    const store = new Map();
    const target = function () {};
    return new Proxy(target, {
        get(_t, key) {
            if (key === Symbol.toPrimitive) return hint => hint === 'number' ? 0 : '';
            if (key === 'then' || key === 'toJSON') return undefined;
            if (key === Symbol.iterator) return function* () {};
            if (!store.has(key)) store.set(key, auto(`${name}.${String(key)}`));
            return store.get(key);
        },
        set(_t, key, value) { store.set(key, value); return true; },
        apply() { return auto(`${name}()`); },
        construct() { return auto(`new ${name}`); },
    });
}
const withAuto = (object, name) => new Proxy(object, {
    get(t, key, receiver) {
        if (key in t || typeof key === 'symbol') return Reflect.get(t, key, receiver);
        return auto(`${name}.${key}`);
    },
});

class Signals {
    connect(signal, fn) {
        this._handlers ??= new Map();
        this._nextId ??= 1;
        const id = this._nextId++;
        this._handlers.set(id, [signal, fn]);
        return id;
    }
    disconnect(id) { this._handlers?.delete(id); }
    emit(signal, ...args) {
        for (const [name, fn] of [...(this._handlers?.values() ?? [])])
            if (name === signal) fn(this, ...args);
    }
}

// ---- St / Clutter ------------------------------------------------------------------------------------------
class Actor extends Signals {
    constructor(props = {}) {
        super();
        this.visible = true;
        this.width = 0;
        this.height = 0;
        this._children = [];
        this._styles = new Set(String(props.style_class ?? '').split(/\s+/).filter(Boolean));
        Object.assign(this, props);
    }
    add_child(child) { this._children.push(child); }
    remove_child(child) { this._children = this._children.filter(c => c !== child); }
    get_children() { return [...this._children]; }
    destroy_all_children() { this._children = []; }
    destroy() { this.emit('destroy'); }
    add_style_class_name(n) { this._styles.add(n); }
    remove_style_class_name(n) { this._styles.delete(n); }
    add_style_pseudo_class() {}
    remove_style_pseudo_class() {}
    add_constraint() {}
    add_effect() {}
    remove_effect() {}
    set_position() {}
    set_size(w, h) { this.width = w; this.height = h; }
    set_width(w) { this.width = w; }
    set_height(h) { this.height = h; }
    set_x() {}
    set_y() {}
    get_preferred_width() {
        const own = typeof this.text === 'string' ? this.text.length * 8 : 0;
        const child = this.child?.get_preferred_width?.()[1] ?? 0;
        return [0, own + child + (this instanceof Button ? 16 : 0)];
    }
    get_preferred_height() { return [0, 40]; }
    get_theme_node() { return {get_length: () => 4}; }
    get_transformed_position() { return [0, 0]; }
    show() { this.visible = true; }
    hide() { this.visible = false; }
    navigate_focus() {}
}
class Button extends Actor {}
class Label extends Actor {}
class ScrollView extends Actor {
    constructor(props) { super(props); this.hadjustment = {lower: 0, upper: 0, value: 0, page_size: 0}; }
}
// The shell theme (D-G69's snap preview stylesheet, extension.js AccentPreview): loads and unloads nothing here.
const shellTheme = {load_stylesheet() {}, unload_stylesheet() {}};
const themeContext = Object.assign(new Signals(), {scale_factor: 1, get_theme: () => shellTheme});
const St = withAuto({
    Widget: Actor, BoxLayout: Actor, Button, Label, Icon: Actor, ScrollView, Bin: Actor,
    ThemeContext: {get_for_stage: () => themeContext},
    PolicyType: {EXTERNAL: 0, NEVER: 1, AUTOMATIC: 2}, DirectionType: {TAB_FORWARD: 0},
}, 'St');
const Clutter = withAuto({
    FixedLayout: class {}, AlignConstraint: class {}, ActorAlign: {CENTER: 1, END: 2, START: 0, FILL: 3},
    AlignAxis: {X_AXIS: 0, Y_AXIS: 1, BOTH: 2}, ScrollDirection: {UP: 0, DOWN: 1, LEFT: 2, RIGHT: 3, SMOOTH: 4},
    EVENT_STOP: true, EVENT_PROPAGATE: false, event_add_filter: () => 1, event_remove_filter() {},
    EventType: {KEY_PRESS: 1}, KEY_Escape: 0xff1b, ModifierType: {BUTTON1_MASK: 256},
}, 'Clutter');

// ---- Gio -----------------------------------------------------------------------------------------------------
const FileType = {UNKNOWN: 0, REGULAR: 1, DIRECTORY: 2};
export const FileMonitorEvent = {CHANGED: 0, CHANGES_DONE_HINT: 1, DELETED: 2, CREATED: 3, ATTRIBUTE_CHANGED: 4,
    MOVED: 7, RENAMED: 8, MOVED_IN: 9, MOVED_OUT: 10};
class FakeFile {
    constructor(p) { this._p = path.resolve(p); }
    static new_for_path(p) { return new FakeFile(p); }
    get_path() { return this._p; }
    get_basename() { return path.basename(this._p); }
    get_child(name) { return new FakeFile(path.join(this._p, name)); }
    get_parent() { const d = path.dirname(this._p); return d === this._p ? null : new FakeFile(d); }
    equal(other) { return other instanceof FakeFile && other._p === this._p; }
    load_contents() {
        rec('read', {path: this._p});
        return [true, new Uint8Array(fs.readFileSync(this._p))];
    }
    query_file_type() {
        try { return fs.statSync(this._p).isDirectory() ? FileType.DIRECTORY : FileType.REGULAR; } catch { return FileType.UNKNOWN; }
    }
    query_exists() { return fs.existsSync(this._p); }
    monitor_directory(flags) {
        const monitor = new FakeMonitor(this, flags);
        world.monitors.push(monitor);
        rec('monitor', {path: this._p, flags});
        return monitor;
    }
}
class FakeMonitor extends Signals {
    constructor(file, flags) { super(); this.file = file; this.flags = flags; this.cancelled = false; }
    cancel() { this.cancelled = true; return true; }
    fire(name, other, type) {
        if (this.cancelled) return;
        this.emit('changed', this.file.get_child(name), other ? this.file.get_child(other) : null, type);
    }
}
class Settings extends Signals {
    constructor(props = {}) { super(); this.props = props; this.settings_schema = {list_keys: () => []}; }
    get_strv() { return []; }
    get_boolean() { return false; }
    get_string() { return ''; }
    set_strv() { return true; }
    set_string() { return true; }
    reset() {}
    get_user_value() { return null; }
    static sync() {}
}
const Gio = withAuto({
    File: FakeFile, FileMonitorFlags: {NONE: 0, WATCH_MOUNTS: 1, SEND_MOVED: 2, WATCH_HARD_LINKS: 4, WATCH_MOVES: 8},
    FileMonitorEvent, FileQueryInfoFlags: {NONE: 0, NOFOLLOW_SYMLINKS: 1}, FileType,
    FileIcon: class { constructor(p) { Object.assign(this, p); } }, ThemedIcon: class {}, BytesIcon: class {},
    DBusExportedObject: {wrapJSObject: () => ({export() {}, unexport() {}, emit_signal() {}})},
    DBus: {session: {call() {}, call_sync() { throw new Error('no session bus in the GE1c harness'); }}},
    bus_own_name_on_connection: () => 1, bus_unown_name() {},
    BusNameOwnerFlags: {DO_NOT_QUEUE: 4}, DBusCallFlags: {NONE: 0, NO_AUTO_START: 1},
    SettingsSchemaSource: {get_default: () => ({lookup: () => null})}, Settings,
    DesktopAppInfo: {new: id => { rec('desktopAppInfo', {id}); return null; }},
    AppInfo: {create_from_commandline: (command, name) => { rec('appInfo', {command, name}); return {name}; }},
    AppInfoCreateFlags: {NONE: 0},
}, 'Gio');

// ---- GLib ----------------------------------------------------------------------------------------------------
function addTimer(fn, ms, kind) {
    const id = world.nextTimer++;
    world.timers.set(id, {fn, ms, kind});
    return id;
}
export function runTimers({maxMs = 999, rounds = 10} = {}) {
    for (let round = 0; round < rounds; round++) {
        const due = [...world.timers].filter(([, t]) => t.ms <= maxMs);
        if (!due.length) return;
        for (const [id, timer] of due) {
            if (!world.timers.has(id)) continue;
            const keep = timer.fn();
            if (!keep) world.timers.delete(id);
        }
    }
}
const pad = n => String(n).padStart(2, '0');
const FIXED = {Y: 2026, m: 11, d: 5, H: 14, M: 13, S: 7};
class DateTime {
    static new_now_local() { return new DateTime(); }
    format(fmt) {
        rec('clock', {fmt});
        return fmt.replace(/%([YmdHMS])/g, (_all, c) => c === 'Y' ? String(FIXED.Y) : pad(FIXED[c]));
    }
}
const GLib = withAuto({
    PRIORITY_DEFAULT: 0, PRIORITY_DEFAULT_IDLE: 200, SOURCE_REMOVE: false, SOURCE_CONTINUE: true,
    USEC_PER_SEC: 1000000,
    get_user_config_dir: () => world.configDir, get_user_runtime_dir: () => path.join(world.configDir, 'runtime'), get_home_dir: () => world.home,
    getenv: name => world.env[name] ?? null, build_filenamev: parts => path.join(...parts),
    timeout_add: (_prio, ms, fn) => addTimer(fn, ms, 'timeout'),
    timeout_add_seconds: (_prio, s, fn) => addTimer(fn, s * 1000, 'seconds'),
    idle_add: (_prio, fn) => addTimer(fn, 0, 'idle'),
    source_remove: id => world.timers.delete(id),
    get_monotonic_time: () => (world.now += 1000),
    DateTime, Variant: class { constructor(sig, value) { this.sig = sig; this.value = value; } deep_unpack() { return this.value; } },
    VariantType: class {}, base64_encode: () => '',
}, 'GLib');

// ---- Shell / Meta / Mtk / GObject -----------------------------------------------------------------------------
const appSystem = Object.assign(new Signals(), {
    lookup_app: id => world.apps[id] ?? null, get_running: () => [],
});
world.appSystem = appSystem;
// The own File Explorer entry, as GNOME reads it from io.github.win11shell.Explorer.desktop (Name= in the
// language install.py or win11-panel wrote, contracts C6); the default pins of settings-defaults.json show it.
world.explorerName = 'Win11 Verkenner';
world.apps['io.github.win11shell.Explorer.desktop'] = {
    get_id: () => 'io.github.win11shell.Explorer.desktop', get_name: () => world.explorerName,
    create_icon_texture: () => new Actor(), get_windows: () => [], activate() {}, open_new_window() {},
};
const Shell = withAuto({
    AppSystem: {get_default: () => appSystem},
    WindowTracker: {get_default: () => Object.assign(new Signals(), {get_window_app: () => null})},
    ActionMode: {NORMAL: 1, OVERVIEW: 2},
}, 'Shell');
const Meta = withAuto({
    WindowType: {NORMAL: 0, DESKTOP: 1, DOCK: 2, DIALOG: 3, MODAL_DIALOG: 4, UTILITY: 5},
    KeyBindingFlags: {IGNORE_AUTOREPEAT: 1}, KeyBindingAction: {NONE: 0},
    WindowClientType: {WAYLAND: 0, X11: 1}, prefs_get_workspace_name: i => `Workspace ${i + 1}`,
}, 'Meta');
class Rectangle { constructor(p = {}) { Object.assign(this, {x: 0, y: 0, width: 0, height: 0}, p); } }
const Mtk = withAuto({Rectangle}, 'Mtk');
const GObject = withAuto({
    signal_handler_find: () => 0, signal_handler_block() {}, signal_handler_unblock() {},
    signal_handler_is_connected: () => false,
}, 'GObject');

// ---- global, Main, MessageTray, Extension ---------------------------------------------------------------------
const monitor = {x: 0, y: 0, width: 1440, height: 900, index: 0};
globalThis.global = withAuto(Object.assign(new Signals(), {
    display: withAuto(Object.assign(new Signals(), {
        focus_window: null, get_current_time_roundtrip: () => 1, get_monitor_scale: () => 1,
        get_monitor_in_fullscreen: () => false, list_all_windows: () => [], unset_input_focus() {},
    }), 'global.display'),
    stage: {}, workspace_manager: Object.assign(new Signals(), {
        get_active_workspace_index: () => 0,
        get_active_workspace: () => ({index: () => 0,
            get_work_area_for_monitor: () => new Rectangle({x: 0, y: 0, width: 1440, height: 852})}),
    }),
    get_window_actors: () => world.actors, backend: {get_dnd: () => null},
    focus_manager: {add_group() {}, remove_group() {}},
    create_app_launch_context: () => ({get_startup_notify_id: () => 'token'}),
    get_current_time: () => 1, get_pointer: () => [0, 0, 0], context: {unsafe_mode: false},
}), 'global');

const Urgency = {LOW: 0, NORMAL: 1, HIGH: 2, CRITICAL: 3};
class Source extends Signals {
    constructor(props = {}) { super(); Object.assign(this, props); this.notifications = []; this.policy = {showBanners: true}; }
    addNotification(n) {
        this.notifications.push(n);
        rec('notification', {source: this.title, title: n.title, body: n.body, actions: n.actions.map(a => a.label)});
        this.emit('notification-added', n);
    }
}
class Notification extends Signals {
    constructor(props = {}) {
        super();
        Object.assign(this, {urgency: Urgency.NORMAL, datetime: null, gicon: null, isTransient: false, resident: false,
            acknowledged: false}, props);
        this.actions = [];
    }
    addAction(label, activate) { this.actions.push({label, activate}); }
    activate() {}
    destroy(reason) { this.emit('destroy', reason); }
}
const messageTray = Object.assign(new Signals(), {
    getSources: () => [], add(source) { rec('traySource', {title: source.title}); this.emit('source-added', source); },
});
const Main = {
    layoutManager: Object.assign(new Signals(), {
        addChrome() {}, removeChrome() {}, trackChrome() {}, untrackChrome() {}, primaryMonitor: monitor,
        monitors: [monitor], panelBox: new Actor(), _trackedActors: [], _queueUpdateRegions() {}, _startingUp: false,
    }),
    overview: Object.assign(new Signals(), {visible: false, animationInProgress: false, hide() {}, toggle() {}}),
    sessionMode: Object.assign(new Signals(), {hasWindows: true, isLocked: false, isGreeter: false, currentMode: 'user'}),
    messageTray,
    wm: {addKeybinding: () => 1, removeKeybinding() {}, skipNextEffect() {}},
    activateWindow() {},
    ctrlAltTabManager: {
        addGroup(root, name, icon) { world.groups.push({root, name, icon}); rec('ctrlAltTab.add', {name}); },
        removeGroup(root) { world.groups = world.groups.filter(g => g.root !== root); rec('ctrlAltTab.remove'); },
    },
    notify(title, body) { rec('notify', {title, body}); },
    panel: {statusArea: {}}, actionMode: 1, extensionManager: {},
};
class Extension {
    constructor(meta = {}) { this.metadata = meta; this.path = meta.path; this.uuid = meta.uuid; }
    getSettings() { return new Settings({}); }
}

globalThis.__ge1c = {
    gi: {St, Clutter, Gio, GLib, Shell, Meta, Mtk, GObject},
    Main, MessageTray: {Notification, NotificationDestroyedReason: {EXPIRED: 1, DISMISSED: 2, SOURCE_CLOSED: 3}, Source, Urgency},
    Extension,
    inputSourceManager: Object.assign(new Signals(), {inputSources: {}, currentSource: null}),
};
export {Main, Actor, Button, Label};
