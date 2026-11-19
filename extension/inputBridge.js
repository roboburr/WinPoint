// SPDX-License-Identifier: MIT
// Settings parity (notes/claude/settings-parity.md §6.6), batch A: the pad OSD of Knoppen toewijzen (input.padOsd), the key capture of Sneltoets instellen (input.capture: Main.pushModal, one key event, release; amendment 44) and the input sources of the tray's input indicator (amendment 15).
// The shape every settings bridge has: constructor(emit, language), enable(), disable(), call(method, args).
// panelBridge.js routes `input.*` here (integration step I, section 8 of the note).
//
//   input.padOsd {vidpid}   GNOME Shell's pad OSD in edition mode for the pad of the tablet <vid>:<pid> (the ids as
//                           Mutter writes them, %.4x:%.4x), as GNOME Settings' "Map Buttons" opens it through
//                           org.gnome.Shell.Wacom.PadOsd: global.display.request_pad_osd(pad, true). The OSD writes
//                           the pad-button keys (tablets/<vid>:<pid>/<control>/) itself. Event input.padOsd.
//   input.capture {timeout} A promise: a modal grab on the stage (Shell.ActionMode.NONE, so no shell or mutter
//                           keybinding fires and the Windows key opens nothing), then the first key press that is
//                           not a modifier on its own: {keyval, mods} (the host names the keysym, libxkbcommon),
//                           {disabled: true} for a plain Backspace, {cancelled: true} for a plain Esc, a click, the
//                           timeout (ms, 1000-120000, default 30000) or input.capture.cancel. The grab ends with the
//                           answer. Events input.capture {state: started|ended}.
//   input.capture.cancel {} Ends a waiting capture ({cancelled: true}); {} also when none waits.
//   input.sources.current {} The session's input sources as GNOME Shell's InputSourceManager has them: {sources:
//                           [{index, type, id, name, short}], current}. Events input.sourceChanged {current, id,
//                           short} and input.sourcesChanged {}.
//   input.sources.activate {index} Switches to that source, as the top bar's keyboard menu does.
//
// input.capture answers late, so the D-Bus Call that carries it has to wait for the promise: panelBridge.js's Call
// is synchronous today, which turns the promise into {}; the host then sends input.capture.cancel and KeyCapture
// listens itself (host/src/services/input/capture.rs). The patch that makes Call wait is a note for I
// (notes/claude/settings-a.md).
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {getInputSourceManager} from 'resource:///org/gnome/shell/ui/status/keyboard.js';

// i18n: pairs (tools/i18n/check_pairs.py): texts in Win11's language (contracts C1, C3).
const TEXT = {
    unknownMethod: {nl: 'Deze functie bestaat nog niet: {method}', en: 'This function doesn’t exist yet: {method}'},
    badTablet: {nl: 'Deze tablet is niet geldig.', en: 'This tablet isn’t valid.'},
    noPad: {nl: 'De knoppen van deze tablet zijn niet gevonden.', en: 'The buttons of this tablet weren’t found.'},
    noGrab: {nl: 'Het toetsenbord kan nu niet worden gelezen.', en: 'The keyboard can’t be read right now.'},
    badSource: {nl: 'Dit toetsenbord is niet ingesteld.', en: 'This keyboard isn’t set up.'},
};
// a vendor and product id as Mutter writes them in its settings paths
const VIDPID = /^[0-9a-f]{4}:[0-9a-f]{4}$/;
const CAPTURE_MS = {min: 1000, max: 120000, default: 30000};
// keys that are a modifier on their own: the capture waits for the key they go with
const MODIFIER_KEYS = new Set([
    Clutter.KEY_Shift_L, Clutter.KEY_Shift_R, Clutter.KEY_Control_L, Clutter.KEY_Control_R,
    Clutter.KEY_Alt_L, Clutter.KEY_Alt_R, Clutter.KEY_Super_L, Clutter.KEY_Super_R, Clutter.KEY_Meta_L,
    Clutter.KEY_Meta_R, Clutter.KEY_Hyper_L, Clutter.KEY_Hyper_R, Clutter.KEY_ISO_Level3_Shift,
    Clutter.KEY_ISO_Level5_Shift, Clutter.KEY_Caps_Lock, Clutter.KEY_Num_Lock,
]);
// GTK's modifier names (gtk_accelerator_name), from Clutter's state
const MODIFIERS = [
    [Clutter.ModifierType.SHIFT_MASK, 'Shift'], [Clutter.ModifierType.CONTROL_MASK, 'Control'],
    [Clutter.ModifierType.MOD1_MASK, 'Alt'], [Clutter.ModifierType.SUPER_MASK, 'Super'],
    [Clutter.ModifierType.MOD4_MASK, 'Super'], [Clutter.ModifierType.HYPER_MASK, 'Hyper'],
    [Clutter.ModifierType.META_MASK, 'Meta'],
];

function failure(code, message) {
    return {ok: false, error: {code, message}};
}

/** The modifier names of a key event's state, each once, in GTK's order. */
export function modifierNames(state) {
    const names = [];
    for (const [mask, name] of MODIFIERS)
        if ((state & mask) && !names.includes(name)) names.push(name);
    return names;
}

/** What a key press means for the capture: null to keep waiting, else the capture's answer. */
export function captureAnswer(keyval, state) {
    if (MODIFIER_KEYS.has(keyval)) return null;
    const mods = modifierNames(state);
    if (!mods.length && keyval === Clutter.KEY_Escape) return {cancelled: true};
    if (!mods.length && keyval === Clutter.KEY_BackSpace) return {disabled: true};
    return {keyval, mods};
}

export class InputBridge {
    constructor(emit, language = () => 'nl') {
        this._emit = emit;
        this._language = language;
        this._enabled = false;
        this._capture = null;
        this._sourceSignals = [];
    }

    _t(key, params = {}) {
        const language = this._language() === 'en' ? 'en' : 'nl';
        return (TEXT[key][language] ?? TEXT[key].nl)
            .replace(/\{(\w+)\}/g, (all, name) => Object.hasOwn(params, name) ? String(params[name]) : all);
    }

    enable() {
        this._enabled = true;
        const manager = getInputSourceManager();
        this._sourceSignals = [
            manager.connect('current-source-changed', () => {
                const current = manager.currentSource;
                this._emit('input.sourceChanged', {current: current?.index ?? -1, id: current?.id ?? '',
                    short: current?.shortName ?? ''});
            }),
            manager.connect('sources-changed', () => this._emit('input.sourcesChanged', {})),
        ];
    }

    disable() {
        this._finishCapture({cancelled: true});
        const manager = getInputSourceManager();
        for (const id of this._sourceSignals) manager.disconnect(id);
        this._sourceSignals = [];
        this._enabled = false;
    }

    call(method, args) {
        args = args ?? {};
        if (method === 'input.padOsd')
            return this._padOsd(args);
        if (method === 'input.capture')
            return this._startCapture(args);
        if (method === 'input.capture.cancel') {
            this._finishCapture({cancelled: true});
            return {ok: true, result: {}};
        }
        if (method === 'input.sources.current')
            return this._sources();
        if (method === 'input.sources.activate')
            return this._activate(args);
        return failure('unknown_method', this._t('unknownMethod', {method}));
    }

    _padOsd(args) {
        const vidpid = typeof args.vidpid === 'string' ? args.vidpid : '';
        if (!VIDPID.test(vidpid))
            return failure('bad_args', this._t('badTablet'));
        const [vendor, product] = vidpid.split(':');
        const seat = Clutter.get_default_backend().get_default_seat();
        const pad = seat.list_devices().find(device =>
            device.get_device_type() === Clutter.InputDeviceType.PAD_DEVICE &&
            (device.get_vendor_id() ?? '').toLowerCase() === vendor &&
            (device.get_product_id() ?? '').toLowerCase() === product);
        if (!pad)
            return failure('not_found', this._t('noPad'));
        global.display.request_pad_osd(pad, true);
        this._emit('input.padOsd', {vidpid, shown: true});
        return {ok: true, result: {shown: true}};
    }

    /** input.capture: a promise of the answer (module comment). A capture already waiting is cancelled first. */
    _startCapture(args) {
        this._finishCapture({cancelled: true});
        const asked = Number.isInteger(args.timeout) ? args.timeout : CAPTURE_MS.default;
        const timeout = Math.min(Math.max(asked, CAPTURE_MS.min), CAPTURE_MS.max);
        return new Promise(resolve => {
            let grab;
            try {
                grab = Main.pushModal(global.stage, {actionMode: Shell.ActionMode.NONE});
            } catch (error) {
                console.warn(`Win11 key capture: ${error.message}`);
                resolve(failure('unsupported', this._t('noGrab')));
                return;
            }
            // GNOME 50's Clutter.Grab no longer exposes seat state; its own modal dialogs retain the grab
            // returned by Main.pushModal directly. Preserve the keyboard check on older Shell versions.
            if (!grab || (typeof grab.get_seat_state === 'function' &&
                (grab.get_seat_state() & Clutter.GrabState.KEYBOARD) === 0)) {
                if (grab) Main.popModal(grab);
                resolve(failure('unsupported', this._t('noGrab')));
                return;
            }
            const capture = {grab, resolve, eventId: 0, timeoutId: 0};
            capture.eventId = global.stage.connect('captured-event', (_stage, event) => this._captured(event));
            capture.timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, timeout, () => {
                capture.timeoutId = 0;
                this._finishCapture({cancelled: true});
                return GLib.SOURCE_REMOVE;
            });
            this._capture = capture;
            this._emit('input.capture', {state: 'started'});
        });
    }

    _captured(event) {
        const type = event.type();
        if (type === Clutter.EventType.BUTTON_PRESS || type === Clutter.EventType.TOUCH_BEGIN) {
            // the grab keeps the pointer from the Settings window, so a click there means Annuleren
            this._finishCapture({cancelled: true});
            return Clutter.EVENT_STOP;
        }
        if (type !== Clutter.EventType.KEY_PRESS)
            return Clutter.EVENT_STOP;
        const answer = captureAnswer(event.get_key_symbol(), event.get_state());
        if (answer)
            this._finishCapture(answer);
        return Clutter.EVENT_STOP;
    }

    _finishCapture(answer) {
        const capture = this._capture;
        if (!capture)
            return;
        this._capture = null;
        if (capture.eventId) global.stage.disconnect(capture.eventId);
        if (capture.timeoutId) GLib.source_remove(capture.timeoutId);
        try {
            Main.popModal(capture.grab);
        } catch (error) {
            console.warn(`Win11 key capture: ${error.message}`);
        }
        this._emit('input.capture', {state: 'ended'});
        capture.resolve({ok: true, result: answer});
    }

    _sources() {
        const manager = getInputSourceManager();
        const sources = Object.values(manager.inputSources ?? {})
            .sort((a, b) => a.index - b.index)
            .map(source => ({index: source.index, type: source.type, id: source.id, name: source.displayName,
                short: source.shortName}));
        return {ok: true, result: {sources, current: manager.currentSource?.index ?? -1}};
    }

    _activate(args) {
        const manager = getInputSourceManager();
        const source = Number.isInteger(args.index) ? manager.inputSources?.[args.index] : null;
        if (!source)
            return failure('not_found', this._t('badSource'));
        source.activate(true);
        return {ok: true, result: {current: source.index}};
    }
}
