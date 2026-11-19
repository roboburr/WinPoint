// SPDX-License-Identifier: MIT
// Settings parity (notes/claude/settings-parity.md §5.5, §6.6), batch D: GNOME Shell's part of Settings' Beeldscherm
// pages. The host (host/src/services/display.rs) calls these; Settings never talks to the shell itself.
//
//   display.identify {labels: {connector: n}, seconds?}  Identificeren: GNOME's own monitor labels
//                       (Main.osdMonitorLabeler, as GNOME Settings' ShowMonitorLabels) for a few seconds (default 3);
//                       event display.identified {shown, connectors?}
//   display.expectConfirm {timeout?} -> {id}             a persistent display.apply follows: GNOME Shell's own
//                       "Keep these display settings?" (windowManager.js _confirmDisplayChange) is held back for that
//                       one change (amendment 32); the next confirm-display-change becomes the event
//                       display.confirmChange {timeout, id} for Settings, which shows Windows' 15 s dialog
//                       (amendments 64, 69). Without a change within 5 s the hold ends by itself.
//   display.release {id}                                 the apply failed: end the hold now
//   display.confirm {keep, id?} -> {pending}             Settings' answer: global.window_manager.complete_display_change
//
// Only the one change the host announced is held: Super+P, GNOME Settings or any other client keeps GNOME Shell's own
// dialog, and when nobody answers mutter reverts after its own 20 s. panelBridge.js routes `display.*` here
// (integration step I, section 8 of the note).
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import GObject from 'gi://GObject';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// i18n: pairs (tools/i18n/check_pairs.py): texts in Win11's language (contracts C1, C3).
const TEXT = {
    unknownMethod: {nl: 'Deze functie bestaat nog niet: {method}', en: 'This function doesn’t exist yet: {method}'},
    badArgs: {nl: 'Ongeldige invoer.', en: 'Invalid input.'},
    noHold: {nl: 'De vraag van GNOME Shell kan hier niet worden overgenomen.', en: 'GNOME Shell’s question can’t be taken over here.'},
    noLabels: {nl: 'Beeldschermen identificeren is nu niet beschikbaar.', en: 'Identifying displays isn’t available right now.'},
};
// a persistent apply follows the hold within this time
const HOLD_MS = 5000;
// mutter reverts after 20 s; an answer after that has nothing to complete
const PENDING_MS = 25000;
const CONNECTOR = /^[\x21-\x7e]{1,64}$/;

function failure(code, message) {
    return {ok: false, error: {code, message}};
}

// The last keep-question id. Ids are unique for the whole session, not per bridge: the extension is disabled and
// enabled again at every screen lock (session-modes "user"), which builds a new DisplayBridge, and Settings asks once
// per id. They rise within this module (it stays loaded across a disable and an enable) and start from the
// milliseconds since boot, so they rise across a reload of the shell's modules too.
let lastId = 0;

function nextId() {
    lastId = Math.max(lastId + 1, Math.floor(GLib.get_monotonic_time() / 1000));
    return lastId;
}

export class DisplayBridge {
    constructor(emit, language = () => 'nl') {
        this._emit = emit;
        this._language = language;
        this._enabled = false;
        this._shellHandler = 0;
        this._handler = 0;
        this._blocked = false;
        this._held = 0;
        this._pending = 0;
        this._timeout = 15;
        this._holdTimer = 0;
        this._pendingTimer = 0;
        this._labelsTimer = 0;
    }

    _t(key, params = {}) {
        const language = this._language() === 'en' ? 'en' : 'nl';
        return (TEXT[key][language] ?? TEXT[key].nl)
            .replace(/\{(\w+)\}/g, (all, name) => Object.hasOwn(params, name) ? String(params[name]) : all);
    }

    enable() {
        this._enabled = true;
        const wm = global.window_manager;
        // GNOME Shell's WindowManager connected its handler at startup; find it before ours exists
        this._shellHandler = GObject.signal_handler_find(wm, {signalId: 'confirm-display-change'});
        this._handler = wm.connect('confirm-display-change', () => this._onConfirmChange());
    }

    disable() {
        this._enabled = false;
        this._clear('_holdTimer');
        this._clear('_pendingTimer');
        if (this._labelsTimer) {
            this._clear('_labelsTimer');
            Main.osdMonitorLabeler?.hide(this._client());
        }
        this._held = this._pending = 0;
        this._unblock();
        if (this._handler)
            global.window_manager.disconnect(this._handler);
        this._handler = this._shellHandler = 0;
    }

    call(method, args) {
        if (method === 'display.identify')
            return this._identify(args);
        if (method === 'display.expectConfirm')
            return this._expect(args);
        if (method === 'display.release')
            return this._release(args);
        if (method === 'display.confirm')
            return this._confirm(args);
        return failure('unknown_method', this._t('unknownMethod', {method}));
    }

    _clear(name) {
        if (this[name])
            GLib.source_remove(this[name]);
        this[name] = 0;
    }

    // the shell's own bus name: the labeler watches its client and hides the labels when it goes away
    _client() {
        return Gio.DBus.session.get_unique_name();
    }

    _shellHandlerConnected() {
        return this._shellHandler !== 0 &&
            GObject.signal_handler_is_connected(global.window_manager, this._shellHandler);
    }

    _unblock() {
        if (this._blocked && this._shellHandlerConnected())
            GObject.signal_handler_unblock(global.window_manager, this._shellHandler);
        this._blocked = false;
    }

    _identify(args) {
        const labels = args.labels;
        if (!labels || typeof labels !== 'object' || Array.isArray(labels))
            return failure('bad_args', this._t('badArgs'));
        const params = {};
        for (const [connector, n] of Object.entries(labels)) {
            if (!CONNECTOR.test(connector) || !Number.isInteger(n) || n < 1 || n > 99)
                return failure('bad_args', this._t('badArgs'));
            params[connector] = new GLib.Variant('i', n);
        }
        const labeler = Main.osdMonitorLabeler;
        if (!this._enabled || !labeler)
            return failure('unsupported', this._t('noLabels'));
        const seconds = Number.isInteger(args.seconds) ? Math.min(Math.max(args.seconds, 1), 10) : 3;
        const client = this._client();
        this._clear('_labelsTimer');
        // shows nothing while GNOME Settings' own labels are up (another client)
        labeler.show(client, params);
        this._labelsTimer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, seconds, () => {
            this._labelsTimer = 0;
            labeler.hide(client);
            this._emit('display.identified', {shown: false});
            return GLib.SOURCE_REMOVE;
        });
        this._emit('display.identified', {shown: true, connectors: Object.keys(labels)});
        return {ok: true, shown: Object.keys(labels).length};
    }

    _expect(args) {
        if (!this._enabled || !this._shellHandlerConnected())
            return failure('unsupported', this._t('noHold'));
        if (!this._blocked) {
            GObject.signal_handler_block(global.window_manager, this._shellHandler);
            this._blocked = true;
        }
        const id = nextId();
        this._held = id;
        this._timeout = Number.isInteger(args.timeout) && args.timeout > 0 && args.timeout <= 20 ? args.timeout : 15;
        this._clear('_holdTimer');
        this._holdTimer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, HOLD_MS, () => {
            this._holdTimer = 0;
            if (this._held === id) {
                this._held = 0;
                this._unblock();
            }
            return GLib.SOURCE_REMOVE;
        });
        return {id};
    }

    _release(args) {
        if (this._held && args.id === this._held) {
            this._held = 0;
            this._clear('_holdTimer');
            this._unblock();
        }
        return {ok: true};
    }

    _onConfirmChange() {
        // not ours: GNOME Shell's own handler (not blocked) asks
        if (!this._held)
            return;
        const id = this._held;
        this._held = 0;
        this._clear('_holdTimer');
        this._pending = id;
        // the next change of anyone else gets GNOME Shell's dialog again, after this emission
        GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            if (!this._held)
                this._unblock();
            return GLib.SOURCE_REMOVE;
        });
        this._clear('_pendingTimer');
        this._pendingTimer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PENDING_MS, () => {
            this._pendingTimer = 0;
            if (this._pending === id)
                this._pending = 0;
            return GLib.SOURCE_REMOVE;
        });
        this._emit('display.confirmChange', {timeout: this._timeout, id});
    }

    _confirm(args) {
        if (typeof args.keep !== 'boolean')
            return failure('bad_args', this._t('badArgs'));
        if (!this._pending || (args.id !== undefined && args.id !== this._pending))
            return {pending: false};
        this._pending = 0;
        this._clear('_pendingTimer');
        global.window_manager.complete_display_change(args.keep);
        return {pending: true};
    }
}
