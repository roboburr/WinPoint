// GE1c extension text dump: every text the extension shows or returns, in the language of the fake settings.json.
//   node --import ./register.mjs dump.mjs EXTENSION_DIR CONFIG_DIR [--switch-to en|nl] [--rewrite-in-place|--burst]
// CONFIG_DIR is the fake XDG config dir (its win11-gnome/settings.json decides the language). Output lines:
//   id<TAB>value      (ids are stable: tests/i18n/BASELINE-CHANGES.ext names them as key:ext.<id>)
// With --switch-to the dump is taken again after the settings file is replaced by rename (the store's way) and
// the directory monitor fires; the second half's ids start with "after.". --burst: a slow writer rewrites the
// file in place in three steps (half-written JSON first); each event must restart the 100 ms timer (debounce,
// ge1c-review m4), so the one read sees the finished file.
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {world, runTimers, Main, Button, Label, FileMonitorEvent} from './world.mjs';

const [extArg, configArg, ...rest] = process.argv.slice(2);
const extDir = path.resolve(extArg), configDir = path.resolve(configArg);
const option = name => { const i = rest.indexOf(name); return i >= 0 ? rest[i + 1] : null; };
world.configDir = configDir;
world.home = path.join(configDir, '..', 'home');
const out = [];
const emit = (id, value) => out.push(`${id}\t${String(value).replace(/\n/g, '\\n')}`);

const ENTRY = {nl: 'Win11 Verkenner', en: 'Win11 File Explorer'};
const fileLanguage = () => {
    try {
        return JSON.parse(fs.readFileSync(path.join(configDir, 'win11-gnome/settings.json'), 'utf8'))?.language === 'en' ? 'en' : 'nl';
    } catch {
        return 'nl';
    }
};
world.explorerName = ENTRY[fileLanguage()];
// What win11-panel does after a switch (C6): rewrite Name= of the own entries; GNOME then emits installed-changed.
const panelRewrite = () => {
    world.explorerName = ENTRY[fileLanguage()];
    world.appSystem.emit('installed-changed');
    runTimers();
};
const {default: Win11Taskbar} = await import(pathToFileURL(path.join(extDir, 'extension.js')).href);
const {NativePanelBridge} = await import(pathToFileURL(path.join(extDir, 'panelBridge.js')).href);
const ext = new Win11Taskbar({path: extDir, uuid: 'win11-gnome@win11shell.local'});
ext.enable();
runTimers();

function bar(prefix) {
    const walk = (actor, depth) => {
        for (const child of actor.get_children()) {
            if (child instanceof Button) emit(`${prefix}button.${out.filter(l => l.startsWith(`${prefix}button.`)).length}`, child.accessible_name);
            if (child instanceof Label && child !== ext._clock) emit(`${prefix}label.${out.filter(l => l.startsWith(`${prefix}label.`)).length}`, child.text);
            walk(child, depth + 1);
        }
    };
    walk(ext._bar, 0);
    emit(`${prefix}clock`, ext._clock.text);
    emit(`${prefix}clockFormat`, world.log.filter(e => e.kind === 'clock').at(-1)?.fmt);
    emit(`${prefix}ctrlAltTab`, world.groups.map(g => g.name).join(' | '));
}

const notices = since => world.log.slice(since).filter(e => e.kind === 'notify').map(e => `${e.title} / ${e.body}`);

function shows(prefix) {
    let mark = world.log.length;
    ext._launch('missing.app.desktop');
    emit(`${prefix}notify.appMissing`, notices(mark).join(' | '));
    mark = world.log.length;
    ext._shortcuts._openTaskManager();
    emit(`${prefix}notify.taskManagerMissing`, notices(mark).join(' | '));
}

const b = ext._panelBridge;
// Current bridges reply asynchronously; the frozen historical bridge still has Call.
const bridgeCall = async (bridge, method, args) => {
    const text = typeof args === 'string' ? args : JSON.stringify(args);
    if (typeof bridge.CallAsync !== 'function') return JSON.parse(bridge.Call(method, text));
    return new Promise(resolve => bridge.CallAsync([method, text], {
        return_value: value => resolve(JSON.parse(value.deep_unpack()[0])),
    }));
};
const call = (method, args) => bridgeCall(b, method, args);
async function failure(prefix, id, method, args, setup = () => {}, teardown = () => {}) {
    setup();
    let value;
    try {
        const reply = await call(method, args);
        value = reply?.ok === false ? `${reply.error.code}\t${reply.error.message}` : `ok\t${JSON.stringify(reply)}`;
    } finally {
        teardown();
    }
    emit(`${prefix}failure.${id}`, value);
}
function fakeWindow({pid = 4242, app = 'io.github.win11shell.Explorer', mapped = true, maximized = false} = {}) {
    const rect = {x: 10, y: 10, width: 800, height: 600};
    return {
        connect: () => 1, disconnect() {}, get_pid: () => pid, get_gtk_application_id: () => app, get_wm_class: () => app,
        get_window_type: () => 0, skip_taskbar: false, get_stable_sequence: () => 1, get_id: () => 77,
        get_compositor_private: () => ({mapped}), is_maximized: () => maximized, is_fullscreen: () => false,
        get_frame_rect: () => rect, can_minimize: () => false, can_close: () => false, can_maximize: () => false,
        allows_resize: () => false, unmaximize() {}, maximize() {}, move_resize_frame() {}, minimize() {}, delete() {},
        get_monitor: () => 0, get_work_area_for_monitor: () => ({x: 0, y: 0, width: 1440, height: 852}),
        get_workspace: () => null, get_title: () => 'x', minimized: false, located_on_workspace: () => true,
        get_client_type: () => 0, is_on_all_workspaces: () => false, urgent: false, demands_attention: false,
        unminimize() {}, focus() {}, stage_to_protocol_rect: r => r,
    };
}
const session = on => () => { Main.sessionMode.hasWindows = on; };
const windows = list => () => { world.actors = list.map(w => ({meta_window: w})); };
const reset = () => { Main.sessionMode.hasWindows = true; world.actors = []; };

async function failures(prefix) {
    const f = (id, method, args, setup, teardown = reset) => failure(prefix, id, method, args, setup, teardown);
    await f('argsObject', 'windows.list', '[1]');
    await f('offline', 'windows.list', {}, () => {}, reset);
    out.pop();
    const idle = new NativePanelBridge({language: () => ext._language?.language ?? 'nl'});
    const offline = await bridgeCall(idle, 'windows.list', '{}');
    emit(`${prefix}failure.offline`, `${offline.error.code}\t${offline.error.message}`);
    await f('sessionStart', 'panel.toggleStart', {}, session(false));
    await f('sessionApps', 'apps.activationToken', {app: 'explorer'}, session(false));
    await f('panelMissing', 'panel.keyboard', {exclusive: true}, session(false));
    await f('sessionWindows', 'windows.focus', {}, session(false));
    await f('sessionUnavailable', 'windows.showDesktop', {}, session(false));
    await f('startMissing', 'panel.toggleStart', {});
    await f('appMissing', 'apps.activationToken', {app: 'nope'});
    await f('keyboardMode', 'panel.keyboard', {}, () => { b._panel = fakeWindow({app: 'io.github.win11shell.Panel'}); },
        () => { b._panel = null; reset(); });
    // panel.power is newer than 3b898e6 (an unknown method there): its own key, outside the unchanged-code rule
    const power = await call('panel.power', {action: 'zap'});
    emit(`${prefix}added.powerAction`, power?.ok === false ? `${power.error.code}\t${power.error.message}` : `ok\t${JSON.stringify(power)}`);
    // windows.hostState and windows.hostRestore went with the WebKit apps' window state (commit 6ebf169f), and
    // their texts with them: test_extension_text.py REMOVED_KEYS
    await f('windowGone', 'windows.focus', {id: 'nope'}, windows([fakeWindow()]));
    await f('explorerGone', 'explorer.tabPosition', {pid: 1});
    await f('cantMinimize', 'windows.minimize', {id: '77'}, windows([fakeWindow()]));
    await f('cantClose', 'windows.close', {id: '77'}, windows([fakeWindow()]));
    await f('cantMaximize', 'windows.maximize', {id: '77'}, windows([fakeWindow()]));
    await f('unknownMethod', 'windows.zap', {id: '77'}, windows([fakeWindow()]));
    await f('badSnap', 'windows.snap', {id: '77', edge: 'bogus'}, windows([fakeWindow()]));
    await f('cantResize', 'windows.snap', {id: '77', edge: 'left'}, windows([fakeWindow()]));
    await f('badRects', 'windows.setIconRects', {rects: 'x'});
    await f('badPid', 'desktop.ready', {pid: -1});
    await f('badIdentity', 'nativeBackdrop.configure', {kind: 'zzz', pid: 1});
    const w = fakeWindow();
    await f('identityMismatch', 'nativeBackdrop.configure', {kind: 'settings', pid: 4242},
        () => { b._windows.set(w, []); }, () => { b._windows.delete(w); reset(); });
    for (const app of ['explorer', 'settings', 'taskmgr', 'alles', 'disks', 'defaultapps', 'installer']) {
        const mark = world.log.length;
        await call('apps.activationToken', {app});
        emit(`${prefix}appInfo.${app}`, world.log.slice(mark).filter(e => e.kind === 'appInfo').map(e => e.name).join(' | '));
    }
    for (const [id, args] of [['nl', {language: 'nl'}], ['en', {language: 'en'}], ['none', {}]]) {
        const mark = world.log.length;
        const reply = await call('notifications.test', args);
        const shown = world.log.slice(mark).filter(e => e.kind === 'notification')
            .map(e => `${e.source}: ${e.title} / ${e.body} [${e.actions.join(', ')}]`);
        emit(`${prefix}notificationTest.${id}`, shown.join(' | '));
        emit(`${prefix}notificationTest.${id}.appName`, reply.notification?.appName ?? '');
        if (id === 'none') world.lastNotification = reply.notification?.id;
    }
    await f('notificationGone', 'notifications.invokeAction', {id: 999999});
    await f('actionGone', 'notifications.invokeAction', {id: world.lastNotification, key: 'action:2'});
    await f('notificationUnknown', 'notifications.zap', {});
    // A GNOME notification whose source has no name (contracts C5: appName "" for the UI's own label).
    const {Source, Notification} = globalThis.__ge1c.MessageTray;
    const source = new Source({title: ''});
    Main.messageTray.add(source);
    const n = new Notification({source, title: 'GE1c', body: 'untitled source'});
    source.addNotification(n);
    const listed = (await call('notifications.list', {})).items?.find(item => item.summary === 'GE1c');
    emit(`${prefix}notificationUntitled.appName`, listed ? JSON.stringify(listed.appName) : '(not listed)');
    n.destroy(3);
}

bar('');
shows('');
await failures('');

const created = option('--create-later');
if (created) {
    // The config folder did not exist at enable(); the store creates it with its first save (C1).
    const dir = path.join(configDir, 'win11-gnome');
    fs.mkdirSync(dir, {recursive: true, mode: 0o700});
    fs.writeFileSync(path.join(dir, 'settings.json'), JSON.stringify({version: 1, language: created}, null, 2) + '\n');
    for (const monitor of world.monitors.filter(m => m.file.get_path() === configDir))
        monitor.fire('win11-gnome', null, FileMonitorEvent.CREATED);
    runTimers();
    panelRewrite();
    bar('after.');
    shows('after.');
}
const target = option('--switch-to');
if (target) {
    const dir = path.join(configDir, 'win11-gnome');
    const file = path.join(dir, 'settings.json');
    const data = {...JSON.parse(fs.readFileSync(file, 'utf8')), language: target};
    if (rest.includes('--burst')) {
        const text = JSON.stringify(data, null, 2) + '\n';
        const pending = () => [...world.timers].filter(([, t]) => t.kind === 'timeout' && t.ms === 100).map(([id]) => id);
        const seen = [], reads = () => world.log.filter(e => e.kind === 'read').length, readsAtStart = reads();
        for (const [part, type] of [[text.slice(0, 12), FileMonitorEvent.CHANGED], [text.slice(0, -20), FileMonitorEvent.CHANGED],
            [text, FileMonitorEvent.CHANGES_DONE_HINT]]) {
            fs.writeFileSync(file, part);
            for (const monitor of world.monitors.filter(m => m.file.get_path() === dir))
                monitor.fire('settings.json', null, type);
            seen.push(pending());
        }
        emit('meta.burstPending', seen.map(ids => ids.join(',')).join(' '));
        emit('meta.burstReadsDuring', reads() - readsAtStart);
    } else if (rest.includes('--rewrite-in-place')) {
        fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n');
        for (const monitor of world.monitors.filter(m => m.file.get_path() === dir))
            monitor.fire('settings.json', null, FileMonitorEvent.CHANGES_DONE_HINT);
    } else {
        fs.writeFileSync(path.join(dir, '.settings-ge1c.tmp'), JSON.stringify(data, null, 2) + '\n');
        fs.renameSync(path.join(dir, '.settings-ge1c.tmp'), file);
        for (const monitor of world.monitors.filter(m => m.file.get_path() === dir))
            monitor.fire('.settings-ge1c.tmp', 'settings.json', FileMonitorEvent.RENAMED);
    }
    runTimers();
    panelRewrite();
    bar('after.');
    shows('after.');
    await failures('after.');
}
emit('meta.monitors', world.monitors.filter(m => !m.cancelled).map(m => `${path.relative(configDir, m.file.get_path()) || '.'}:${m.flags}`).join(' '));
emit('meta.reads', [...new Set(world.log.filter(e => e.kind === 'read').map(e => path.relative(configDir, e.path)))].join(' '));
b._notificationsBridge.disable();
await failure('', 'notificationOffline', 'notifications.list', {});
ext.disable();
emit('meta.monitorsAfterDisable', world.monitors.filter(m => !m.cancelled).length);
emit('meta.timersAfterDisable', [...world.timers.values()].filter(t => t.kind !== 'timeout' || t.ms !== 500).length);
process.stdout.write(out.join('\n') + '\n');
