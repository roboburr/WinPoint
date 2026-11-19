// SPDX-License-Identifier: MIT
// Settings parity (notes/claude/settings-parity.md §6.6): start a program as a child of GNOME Shell, so no Python or
// GTK 3 setup tool runs under a WinPoint process: an IBus engine's own setup (input.engineSetup, batch A) and
// Ubuntu's Security Center (privacy.securityCenter.open, batch F). Only the programs below are started.
//
//   launch.app {id}            a desktop id: Ubuntu's Security Center (APPS)
//   launch.ibusSetup {engine}  an IBus engine's Preferences, as GNOME Settings opens them (cc-input-source-ibus.c,
//                              cc-input-list-box.c): the desktop file ibus-setup-<engine name before ':'>.desktop,
//                              started with IBUS_ENGINE_NAME=<engine> (libpinyin's setup tells pinyin from bopomofo by
//                              it; mozc's desktop file runs mozc_tool --mode=config_dialog)
//
// Event launch.done {method, ok}. panelBridge.js routes `launch.*` here (integration step I, section 8 of the note).
import Gio from 'gi://Gio';
import Shell from 'gi://Shell';

// i18n: pairs (tools/i18n/check_pairs.py): texts in Win11's language (contracts C1, C3).
const TEXT = {
    notAllowed: {nl: 'Dit programma kan hier niet worden gestart.', en: 'This program can’t be started here.'},
    notInstalled: {nl: 'Dit programma is niet geïnstalleerd.', en: 'This program isn’t installed.'},
    unknownMethod: {nl: 'Onbekende startfunctie: {method}', en: 'Unknown launch function: {method}'},
};
// Ubuntu's Security Center (the desktop-security-center snap)
const APPS = new Set([
    'desktop-security-center_desktop-security-center.desktop',
    'security-center_security-center.desktop',
]);
// an IBus engine name, optionally with its layout ("anthy", "libpinyin", "table:wubi-jidian86", "m17n:hi:inscript")
const IBUS_ENGINE = /^[\w.+-]{1,64}(?::[\w.+-]{1,64}){0,2}$/;

function failure(code, message) {
    return {ok: false, error: {code, message}};
}

export class LaunchBridge {
    constructor(emit, language = () => 'nl') {
        this._emit = emit;
        this._language = language;
        this._enabled = false;
    }

    _t(key, params = {}) {
        const language = this._language() === 'en' ? 'en' : 'nl';
        return (TEXT[key][language] ?? TEXT[key].nl)
            .replace(/\{(\w+)\}/g, (all, name) => Object.hasOwn(params, name) ? String(params[name]) : all);
    }

    enable() {
        this._enabled = true;
    }

    disable() {
        this._enabled = false;
    }

    call(method, args) {
        if (method === 'launch.app')
            return this._app(args);
        if (method === 'launch.ibusSetup')
            return this._ibusSetup(args);
        return failure('unknown_method', this._t('unknownMethod', {method}));
    }

    _app(args) {
        const id = typeof args.id === 'string' ? args.id : '';
        if (!APPS.has(id))
            return failure('denied', this._t('notAllowed'));
        const app = Shell.AppSystem.get_default().lookup_app(id);
        if (!app)
            return failure('not_found', this._t('notInstalled'));
        app.activate();
        this._emit('launch.done', {method: 'launch.app', ok: true});
        return {ok: true};
    }

    _ibusSetup(args) {
        const engine = typeof args.engine === 'string' ? args.engine : '';
        if (!IBUS_ENGINE.test(engine) || engine.includes('..'))
            return failure('denied', this._t('notAllowed'));
        const info = Gio.DesktopAppInfo.new(`ibus-setup-${engine.split(':')[0]}.desktop`);
        if (!info)
            return failure('not_found', this._t('notInstalled'));
        // D-Bus calls have no Clutter input event: a compositor timestamp gives Mutter a valid startup sequence
        const context = global.create_app_launch_context(global.display.get_current_time_roundtrip(), -1);
        context.setenv('IBUS_ENGINE_NAME', engine);
        info.launch([], context);
        this._emit('launch.done', {method: 'launch.ibusSetup', ok: true});
        return {ok: true};
    }
}
