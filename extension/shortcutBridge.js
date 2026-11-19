// SPDX-License-Identifier: MIT
// The Windows key, WinPoint's Task View (Alt+Tab) and Task Manager (Ctrl+Alt+Delete) shortcuts.
//
// * The Windows key (mutter's overlay-key signal; GNOME's own handler is blocked) opens WinPoint's Start, unless
//   Sticky Keys is on or WinPoint's setting start.windowsKey is false (Geavanceerde toetsenbordinstellingen ›
//   "Windows-toets opent Start", notes/claude/settings-parity.md amendment 58), read from Win11's settings.json at
//   each press. GNOME's overlay-key itself is a separate GNOME-only row on that page.
// * The extension's task-view and task-manager keys (editable on Sneltoetsen, amendment 13) hold their accelerators
//   one by one: while task-view has <Alt>Tab, <Alt>Tab is taken from every org.gnome.desktop.wm.keybindings key (the
//   user's value kept in task-view-restore); while task-manager has Ctrl+Alt+Delete, that is taken from media-keys
//   logout (logout-restore). When the user gives task-view or task-manager another shortcut, the keys it took from
//   get their value back.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const TASK_MANAGER = 'io.github.win11shell.TaskManager.desktop';
const MODES = Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW;
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const strings = value => Array.isArray(value) && value.length <= 100 &&
    value.every(item => typeof item === 'string' && item.length <= 2048);
// i18n: pairs (tools/i18n/check_pairs.py): texts in Win11's language (contracts C1, C3).
const TEXT = {
    product: {nl: 'Win11 voor GNOME', en: 'Win11 for GNOME'},
    taskManagerMissing: {nl: 'Taakbeheer is nog niet geïnstalleerd.', en: 'Task Manager isn’t installed yet.'},
};
// The media-keys daemon owns this shortcut by default. Keep every other logout
// shortcut, including a user's custom shortcuts, and preserve its default state.
const isAltTab = value => /^<(?:alt|mod1)>tab$/i.test(value);
const isCtrlAltDelete = value => {
    const parts = value.toLowerCase().match(/<[^>]+>|[^<>]+/g) ?? [];
    if (parts.pop() !== 'delete') return false;
    return parts.length === 2 && parts.includes('<alt>') &&
        (parts.includes('<control>') || parts.includes('<ctrl>') || parts.includes('<primary>'));
};

/** start.windowsKey from the text of Win11's settings.json: only an explicit false turns the Windows key off. */
export function windowsKeyOpensStart(text) {
    try {
        return JSON.parse(text)?.start?.windowsKey !== false;
    } catch {
        return true;
    }
}

function readWindowsKey() {
    try {
        const path = GLib.build_filenamev([GLib.get_user_config_dir(), 'win11-gnome', 'settings.json']);
        const [ok, bytes] = Gio.File.new_for_path(path).load_contents(null);
        return ok ? windowsKeyOpensStart(new TextDecoder().decode(bytes)) : true;
    } catch {
        return true;
    }
}

export class ShortcutBridge {
    constructor(extension, toggleStart, openTaskView = () => {}, language = () => 'nl') {
        this._settings = extension.getSettings('org.gnome.shell.extensions.win11-gnome');
        this._language = language;
        this._toggleStart = toggleStart;
        this._openTaskView = openTaskView;
        this._wm = new Gio.Settings({schema_id: 'org.gnome.desktop.wm.keybindings'});
        this._wmKeys = this._wm.settings_schema.list_keys();
        this._wmId = this._taskViewHiddenId = this._taskViewId = this._taskManagerId = 0;
        this._taskViewBinding = this._changingTaskView = false;
        this._enabled = false;
        this._overlayId = this._originalOverlayId = this._startupId = this._overviewHiddenId = 0;
        this._pendingStart = false;
        this._binding = false;
        this._changingLogout = false;
        this._logout = new Gio.Settings({schema_id: 'org.gnome.settings-daemon.plugins.media-keys'});
        this._a11y = new Gio.Settings({schema_id: 'org.gnome.desktop.a11y.keyboard'});
    }

    enable() {
        this._enabled = true;
        if (Main.layoutManager._startingUp) {
            this._startupId = Main.layoutManager.connect('startup-complete', () => {
                Main.layoutManager.disconnect(this._startupId);
                this._startupId = 0;
                this._bindOverlay();
            });
        } else {
            this._bindOverlay();
        }
        this._binding = Main.wm.addKeybinding('task-manager', this._settings,
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT, MODES, () => this._openTaskManager()) !== Meta.KeyBindingAction.NONE;
        if (this._binding) {
            this._logoutId = this._logout.connect('changed::logout', () => this._takeLogout());
            this._taskManagerId = this._settings.connect('changed::task-manager', () => this._syncLogout());
            this._syncLogout();
        } else {
            this._restoreLogout();
            console.warn('Win11 Task Manager shortcut could not be registered.');
        }
        this._taskViewBinding = Main.wm.addKeybinding('task-view', this._settings,
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT, MODES, () => this._requestTaskView()) !== Meta.KeyBindingAction.NONE;
        if (this._taskViewBinding) {
            this._wmId = this._wm.connect('changed', (_settings, key) => this._takeTaskView(key));
            this._taskViewId = this._settings.connect('changed::task-view', () => this._syncTaskView());
            this._syncTaskView();
        } else this._restoreTaskView();
    }

    /** Whether task-view holds <Alt>Tab now (amendment 13: held per accelerator). */
    _holdsAltTab() {
        return this._settings.get_strv('task-view').some(isAltTab);
    }

    _holdsCtrlAltDelete() {
        return this._settings.get_strv('task-manager').some(isCtrlAltDelete);
    }

    /** task-view changed: take <Alt>Tab from the wm keys while it holds it, give it back when it no longer does. */
    _syncTaskView() {
        if (!this._enabled) return;
        if (this._holdsAltTab()) for (const key of this._wmKeys) this._takeTaskView(key);
        else this._restoreTaskView();
    }

    _syncLogout() {
        if (!this._enabled) return;
        if (this._holdsCtrlAltDelete()) this._takeLogout();
        else this._restoreLogout();
    }

    _requestTaskView() {
        if (!this._isUserSession()) return;
        if (!Main.overview.visible) { this._openTaskView(); return; }
        if (!this._taskViewHiddenId) {
            this._taskViewHiddenId = Main.overview.connect('hidden', () => {
                Main.overview.disconnect(this._taskViewHiddenId);
                this._taskViewHiddenId = 0;
                if (this._isUserSession()) this._openTaskView();
            });
        }
        Main.overview.hide();
    }

    _savedTaskView() {
        try {
            const value = JSON.parse(this._settings.get_string('task-view-restore'));
            if (value && typeof value === 'object' && !Array.isArray(value))
                return Object.fromEntries(Object.entries(value).filter(([key, entry]) =>
                    this._wmKeys.includes(key) && entry && strings(entry.owned) &&
                    (entry.user === null || strings(entry.user))));
        } catch { /* No previous takeover. */ }
        return {};
    }

    _saveTaskView(value) {
        if (!Object.keys(value).length) { this._settings.reset('task-view-restore'); return true; }
        return this._settings.set_string('task-view-restore', JSON.stringify(value));
    }

    _takeTaskView(key) {
        if (!this._enabled || this._changingTaskView || !this._holdsAltTab()) return;
        const current = this._wm.get_strv(key);
        const saved = this._savedTaskView();
        if (saved[key] && same(current, saved[key].owned)) return;
        const owned = current.filter(value => !isAltTab(value));
        this._changingTaskView = true;
        try {
            if (same(current, owned)) { delete saved[key]; this._saveTaskView(saved); return; }
            saved[key] = {user: this._wm.get_user_value(key)?.deep_unpack() ?? null, owned};
            if (!this._saveTaskView(saved)) return;
            Gio.Settings.sync();
            if (!this._wm.set_strv(key, owned)) { delete saved[key]; this._saveTaskView(saved); }
        } finally { this._changingTaskView = false; }
    }

    _restoreTaskView() {
        for (const [key, saved] of Object.entries(this._savedTaskView())) {
            if (!same(this._wm.get_strv(key), saved.owned)) continue;
            if (saved.user === null) this._wm.reset(key);
            else this._wm.set_strv(key, saved.user);
        }
        this._settings.reset('task-view-restore');
    }

    _isUserSession() {
        return this._enabled && Main.sessionMode.hasWindows &&
            !Main.sessionMode.isLocked && !Main.sessionMode.isGreeter && (Main.actionMode & MODES);
    }

    _bindOverlay() {
        if (!this._enabled || this._overlayId) return;
        // GNOME connects this in OverviewControls, which is created during startup.
        // Block only that existing connection and release exactly our block later.
        this._originalOverlayId = GObject.signal_handler_find(global.display, {signalId: 'overlay-key'});
        if (this._originalOverlayId)
            GObject.signal_handler_block(global.display, this._originalOverlayId);
        this._overlayId = global.display.connect('overlay-key', () => {
            if (!this._isUserSession() || this._a11y.get_boolean('stickykeys-enable') || !readWindowsKey()) return;
            if (!Main.overview.visible) {
                this._toggleStart();
                return;
            }
            // Wait until the GNOME overview releases keyboard focus, including when
            // Super is pressed immediately after login. Repeated presses cancel.
            this._pendingStart = !this._pendingStart;
            if (!this._overviewHiddenId) {
                this._overviewHiddenId = Main.overview.connect('hidden', () => {
                    Main.overview.disconnect(this._overviewHiddenId);
                    this._overviewHiddenId = 0;
                    const pending = this._pendingStart;
                    this._pendingStart = false;
                    if (pending && this._isUserSession()) this._toggleStart();
                });
            }
            Main.overview.hide();
        });
    }

    _openTaskManager() {
        if (!this._isUserSession()) return;
        Main.overview.hide();
        const app = Shell.AppSystem.get_default().lookup_app(TASK_MANAGER);
        if (app) app.activate();
        else Main.notify(this._t('product'), this._t('taskManagerMissing'));
    }

    _t(key) {
        const pair = TEXT[key];
        return pair[this._language?.()] ?? pair.nl;
    }

    _savedLogout() {
        try {
            const value = JSON.parse(this._settings.get_string('logout-restore'));
            if (strings(value.owned) && (value.user === null || strings(value.user))) return value;
        } catch { /* No previous ownership, or invalid recovery state. */ }
        return null;
    }

    _takeLogout() {
        if (this._changingLogout || !this._enabled || !this._holdsCtrlAltDelete()) return;
        const current = this._logout.get_strv('logout');
        const saved = this._savedLogout();
        // A crash/restarted Shell can leave the owned value on disk. Reuse its
        // original backup rather than mistaking that temporary value for a default.
        if (saved && same(current, saved.owned)) return;
        const owned = current.filter(value => !isCtrlAltDelete(value));
        this._changingLogout = true;
        try {
            if (same(current, owned)) {
                this._settings.reset('logout-restore');
                return;
            }
            const user = this._logout.get_user_value('logout')?.deep_unpack() ?? null;
            if (!this._settings.set_string('logout-restore', JSON.stringify({user, owned}))) return;
            // Persist the recovery record before changing the existing shortcut.
            Gio.Settings.sync();
            if (!this._logout.set_strv('logout', owned)) this._settings.reset('logout-restore');
        } finally {
            this._changingLogout = false;
        }
    }

    _restoreLogout() {
        const saved = this._savedLogout();
        if (saved && same(this._logout.get_strv('logout'), saved.owned)) {
            if (saved.user === null) this._logout.reset('logout');
            else this._logout.set_strv('logout', saved.user);
        }
        this._settings.reset('logout-restore');
    }

    disable() {
        this._enabled = false;
        if (this._startupId) Main.layoutManager.disconnect(this._startupId);
        if (this._overviewHiddenId) Main.overview.disconnect(this._overviewHiddenId);
        this._startupId = this._overviewHiddenId = 0;
        this._pendingStart = false;
        if (this._taskViewHiddenId) Main.overview.disconnect(this._taskViewHiddenId);
        this._taskViewHiddenId = 0;
        if (this._taskViewBinding) Main.wm.removeKeybinding('task-view');
        this._taskViewBinding = false;
        if (this._wmId) this._wm.disconnect(this._wmId);
        if (this._taskViewId) this._settings.disconnect(this._taskViewId);
        this._wmId = this._taskViewId = 0;
        this._restoreTaskView();
        if (this._overlayId) global.display.disconnect(this._overlayId);
        if (this._originalOverlayId && GObject.signal_handler_is_connected(global.display, this._originalOverlayId))
            GObject.signal_handler_unblock(global.display, this._originalOverlayId);
        this._overlayId = this._originalOverlayId = 0;
        if (this._binding) Main.wm.removeKeybinding('task-manager');
        this._binding = false;
        if (this._logoutId) this._logout.disconnect(this._logoutId);
        if (this._taskManagerId) this._settings.disconnect(this._taskManagerId);
        this._logoutId = this._taskManagerId = 0;
        this._restoreLogout();
        this._settings = this._logout = this._a11y = this._wm = this._toggleStart = this._openTaskView = this._language = null;
    }
}
