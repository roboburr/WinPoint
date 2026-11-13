// SPDX-License-Identifier: MIT
// Observe GNOME's existing notification server. Never replace its D-Bus owner or DND settings.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as MessageTray from 'resource:///org/gnome/shell/ui/messageTray.js';

const LEASE_US = 6_000_000;
function fail(code, message) { const e = new Error(message); e.code = code; throw e; }
// i18n: pairs (tools/i18n/check_pairs.py): texts in Win11's language (contracts C1, C3). The three
// test-notification pairs and the two Dutch failures are 3b898e6's own; the rest are listed in
// tests/i18n/BASELINE-CHANGES.ext.
const TEXT = {
    offline: {nl: 'De meldingenkoppeling staat uit.', en: 'The notification link is turned off.'},
    notificationGone: {nl: 'Deze melding bestaat niet meer.', en: 'This notification no longer exists.'},
    actionGone: {nl: 'Deze actie bestaat niet meer.', en: 'This action no longer exists.'},
    unknownMethod: {nl: 'Onbekende meldingsfunctie: {method}', en: 'Unknown notification function: {method}'},
    testTitle: {nl: 'Zo ziet een melding eruit', en: 'This is what a notification looks like'},
    testBody: {nl: 'Meldingen van je apps verschijnen hier en blijven bewaard in het meldingencentrum.', en: 'Notifications from your apps appear here and stay in the notification centre.'},
    testAction: {nl: 'Prima', en: 'Got it'},
};
const tr = (key, language, params = {}) => (TEXT[key][language] ?? TEXT[key].nl)
    .replace(/\{(\w+)\}/g, (all, name) => Object.hasOwn(params, name) ? String(params[name]) : all);
function clean(text, limit) {
    // GNOME notifications may contain Pango markup. The original UI expects plain text.
    return String(text ?? '').replace(/<br\s*\/?\s*>/gi, '\n').replace(/<[^>]{0,500}>/g, '')
        .replace(/&(#x[\da-f]+|#\d+|amp|lt|gt|quot|apos);/gi, (all, name) => {
            const named = {amp: '&', lt: '<', gt: '>', quot: '"', apos: "'"};
            if (named[name]) return named[name];
            const n = name[1]?.toLowerCase() === 'x' ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10);
            return n > 0 && n <= 0x10ffff && !(n >= 0xd800 && n <= 0xdfff) ? String.fromCodePoint(n) : '\ufffd';
        }).trim().slice(0, limit);
}
function iconRef(icon) {
    if (icon instanceof Gio.FileIcon) return icon.get_file().get_path();
    if (icon instanceof Gio.ThemedIcon) return icon.get_names()[0] ?? null;
    if (icon instanceof Gio.BytesIcon) {
        const bytes = icon.get_bytes();
        if (bytes.get_size() <= 512 * 1024)
            return `data:image/png;base64,${GLib.base64_encode(bytes.get_data())}`;
    }
    return null;
}

export class NativeNotificationsBridge {
    constructor(emit, panelReady, language = () => 'nl') {
        this._emit = emit;
        this._panelReady = panelReady;
        this._language = language;
        this._enabled = false;
    }

    enable() {
        if (this._enabled) return;
        this._enabled = true;
        this._sources = new Map();
        this._items = new Map();
        this._ids = new WeakMap();
        this._next = 1;
        this._leaseUntil = 0;
        this._leasePid = 0;
        this._dnd = false;
        this._signals = [
            Main.messageTray.connect('source-added', (_tray, source) => this._source(source, true)),
            Main.messageTray.connect('source-removed', (_tray, source) => this._unwatchSource(source)),
            Main.messageTray.connect('destroy', () => this.disable()),
        ];
        for (const source of Main.messageTray.getSources()) this._source(source, true);
        this._timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 500, () => {
            this.sync();
            return GLib.SOURCE_CONTINUE;
        });
    }

    _source(source, initial = false) {
        if (this._sources.has(source)) return;
        const signals = [source.connect('notification-added', (_s, n) => this._track(n, false)),
            source.connect('notification-request-banner', (_s, n) => this._track(n, false))];
        this._sources.set(source, signals);
        for (const n of source.notifications) this._track(n, initial);
    }

    _unwatchSource(source) {
        const signals = this._sources.get(source);
        if (!signals) return;
        for (const id of signals) source.disconnect(id);
        this._sources.delete(source);
        for (const record of [...this._items.values()]) {
            if (record.source === source)
                this._remove(record, MessageTray.NotificationDestroyedReason.SOURCE_CLOSED);
        }
    }

    _track(n, initial) {
        if (this._ids.has(n)) return;
        const id = this._next++;
        this._ids.set(n, id);
        const record = {n, id, source: n.source, signals: [], idle: 0, announced: initial, delivered: false};
        this._items.set(id, record);
        const update = () => {
            if (record.idle || !this._enabled) return;
            record.idle = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                record.idle = 0;
                if (!this._items.has(id)) return GLib.SOURCE_REMOVE;
                const item = this._serialize(record);
                if (record.announced) item.replaced = true;
                record.announced = true;
                // Transfer banner ownership once to the ready consumer before emitting.
                // An ACK lost after painting must not replay the same banner in GNOME
                // when this lease expires. Acknowledgement retains both histories.
                if (this._block && this._active()) {
                    record.forwarded = true;
                    n.acknowledged = true;
                }
                this._emit('notifications.new', {notification: item});
                return GLib.SOURCE_REMOVE;
            });
        };
        record.signals.push(n.connect('notify', (_n, prop) => {
            if (!['acknowledged', 'datetime'].includes(prop.name)) update();
        }));
        record.signals.push(n.connect('action-added', update));
        record.signals.push(n.connect('action-removed', update));
        record.signals.push(n.connect('destroy', (_n, reason) => this._remove(record, reason)));
        if (!initial) update();
        while (this._items.size > 100) {
            // Bound our mirrors without dismissing notifications in another client's tray.
            const oldest = this._items.values().next().value;
            this._remove(oldest, MessageTray.NotificationDestroyedReason.EXPIRED);
        }
    }

    _remove(record, reason) {
        if (!this._items.delete(record.id)) return;
        this._ids.delete(record.n);
        if (record.idle) GLib.source_remove(record.idle);
        for (const signal of record.signals) record.n.disconnect(signal);
        this._emit('notifications.closed', {id: record.id, reason});
    }

    _serialize({n, id}) {
        const source = n.source;
        const app = source?.app;
        const appInfo = app?.get_app_info?.();
        let icon = iconRef(appInfo?.get_icon?.()) ?? iconRef(source?.icon) ?? iconRef(n.gicon);
        const image = iconRef(n.gicon);
        if (icon?.startsWith('data:')) icon = 'preferences-system-notifications';
        const critical = n.urgency === MessageTray.Urgency.CRITICAL;
        // C5: a source without a name gets an empty appName; the UI shows its own localized label.
        return {id, appName: clean(source?.title || app?.get_name?.() || '', 100),
            appIcon: icon || 'preferences-system-notifications', summary: clean(n.title, 300),
            body: clean(n.body, 3000), actions: n.actions.slice(0, 3).map((a, i) => ({key: `action:${i}`, label: clean(a.label, 60)})),
            hasDefault: true, time: n.datetime?.to_unix?.() * 1000 || Date.now(),
            urgency: critical ? 'critical' : n.urgency === MessageTray.Urgency.LOW ? 'low' : 'normal',
            image: image?.startsWith('/') || image?.startsWith('data:') ? image : null,
            desktopId: app?.get_id?.() ?? null, category: null, transient: n.isTransient,
            resident: n.resident, value: null, expireTimeout: -1,
            silent: !this._block || !this._active() || n.acknowledged || (!critical && (this._dnd || n.urgency === MessageTray.Urgency.LOW || !source?.policy?.showBanners)), source: 'gnome'};
    }

    _active() {
        return this._enabled && GLib.get_monotonic_time() < this._leaseUntil && this._panelReady(this._leasePid);
    }

    // bannerBlocked has a setter, but no getter, in GNOME 50. Preserve writes from other
    // extensions during our lease, rather than restoring a stale value or altering GSettings.
    _acquire() {
        if (this._block) return;
        const tray = Main.messageTray;
        const own = Object.getOwnPropertyDescriptor(tray, 'bannerBlocked');
        let owner = tray;
        let descriptor = own;
        while (!descriptor && (owner = Object.getPrototypeOf(owner)))
            descriptor = Object.getOwnPropertyDescriptor(owner, 'bannerBlocked');
        if (!descriptor?.set || own?.configurable === false) return;
        const original = descriptor.set;
        const state = {own, original, desired: Boolean(tray._bannerBlocked)};
        state.get = () => tray._bannerBlocked;
        state.set = value => { state.desired = Boolean(value); original.call(tray, true); };
        Object.defineProperty(tray, 'bannerBlocked', {configurable: true, enumerable: descriptor.enumerable,
            get: state.get, set: state.set});
        this._block = state;
        original.call(tray, true);
    }

    _release() {
        if (!this._block) return;
        const tray = Main.messageTray;
        const state = this._block;
        this._block = null;
        const current = Object.getOwnPropertyDescriptor(tray, 'bannerBlocked');
        // Another extension replacing our accessor owns the new property; leave it alone.
        if (current?.set !== state.set || current?.get !== state.get) return;
        if (state.own) Object.defineProperty(tray, 'bannerBlocked', state.own);
        else delete tray.bannerBlocked;
        state.original.call(tray, state.desired);
    }

    sync() {
        if (this._active()) this._acquire();
        else this._release();
    }

    call(method, args) {
        const t = (key, params) => tr(key, this._language(), params);
        if (!this._enabled) return fail('offline', t('offline'));
        if (method === 'notifications.consumerReady') {
            if (!Number.isInteger(args.pid) || !this._panelReady(args.pid) || args.ready !== true)
                return {ready: false};
            this._leasePid = args.pid;
            this._leaseUntil = GLib.get_monotonic_time() + LEASE_US;
            this._dnd = Boolean(args.dnd);
            this.sync();
            return {ready: Boolean(this._block), leaseMs: 6000};
        }
        if (method === 'notifications.consumerGone') {
            if (args.pid === this._leasePid) { this._leaseUntil = 0; this.sync(); }
            return {};
        }
        if (method === 'notifications.delivered') {
            const r = this._items.get(args.id);
            if (this._active() && args.pid === this._leasePid && r) {
                r.delivered = true;
                r.n.acknowledged = true;
            }
            return {};
        }
        if (method === 'notifications.list')
            return {supported: true, dnd: Boolean(args.dnd), items: [...this._items.values()].reverse().map(r => this._serialize(r))};
        if (method === 'notifications.test') {
            // The host passes its own language; without one, Win11's language (C1).
            const language = args.language === 'en' || args.language === 'nl' ? args.language : this._language();
            const source = new MessageTray.Source({title: 'Win11 Shell', iconName: 'preferences-system-notifications'});
            Main.messageTray.add(source);
            const n = new MessageTray.Notification({source,
                title: tr('testTitle', language), body: tr('testBody', language)});
            n.addAction(tr('testAction', language), () => {});
            source.addNotification(n);
            return {notification: this._serialize(this._items.get(this._ids.get(n)))};
        }
        if (method === 'notifications.clear') {
            const records = [...this._items.values()];
            for (const r of records) r.n.destroy(MessageTray.NotificationDestroyedReason.DISMISSED);
            return {cleared: records.length};
        }
        const record = this._items.get(args.id);
        if (method === 'notifications.dismiss') {
            if (record) record.n.destroy(MessageTray.NotificationDestroyedReason.DISMISSED);
            return {ok: Boolean(record)};
        }
        if (method === 'notifications.invokeAction') {
            if (!record) return fail('not_found', t('notificationGone'));
            if (args.key === 'default') record.n.activate();
            else {
                const index = /^action:([0-2])$/.exec(args.key)?.[1];
                const action = index === undefined ? null : record.n.actions[Number(index)];
                if (!action) return fail('not_found', t('actionGone'));
                action.activate();
            }
            this._emit('notifications.action', {id: args.id, key: args.key});
            return {};
        }
        return fail('unknown_method', t('unknownMethod', {method}));
    }

    disable() {
        if (!this._enabled) return;
        this._leaseUntil = 0;
        this._release();
        this._enabled = false;
        if (this._timer) GLib.source_remove(this._timer);
        for (const signal of this._signals) Main.messageTray.disconnect(signal);
        for (const source of [...this._sources.keys()]) this._unwatchSource(source);
        for (const r of [...this._items.values()]) {
            if (r.idle) GLib.source_remove(r.idle);
            for (const signal of r.signals) r.n.disconnect(signal);
        }
        this._items.clear();
    }
}
