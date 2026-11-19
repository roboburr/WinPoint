// Node module hooks for the GE1c extension text harness (tests/installer/ext/README.md).
// gi:// and resource:/// imports resolve to the fake GNOME Shell world in world.mjs; the extension's own .js files
// load as ES modules. Nothing of GNOME Shell, GJS or a display is used.
const STUB = 'ge1c-stub:';
const MAIN = ['layoutManager', 'overview', 'sessionMode', 'messageTray', 'wm', 'activateWindow', 'ctrlAltTabManager',
    'notify', 'panel', 'actionMode', 'extensionManager'];
const TRAY = ['Notification', 'NotificationDestroyedReason', 'Source', 'Urgency'];

export async function resolve(specifier, context, next) {
    if (specifier.startsWith('gi://'))
        return {url: `${STUB}gi/${specifier.slice(5)}`, shortCircuit: true};
    if (specifier === 'resource:///org/gnome/shell/ui/main.js')
        return {url: `${STUB}main`, shortCircuit: true};
    if (specifier === 'resource:///org/gnome/shell/ui/status/keyboard.js')
        return {url: `${STUB}keyboard`, shortCircuit: true};
    if (specifier === 'resource:///org/gnome/shell/ui/messageTray.js')
        return {url: `${STUB}messageTray`, shortCircuit: true};
    if (specifier === 'resource:///org/gnome/shell/extensions/extension.js')
        return {url: `${STUB}extension`, shortCircuit: true};
    return next(specifier, context);
}

export async function load(url, context, next) {
    if (url.startsWith(STUB)) {
        const what = url.slice(STUB.length);
        let source;
        if (what.startsWith('gi/'))
            source = `export default globalThis.__ge1c.gi[${JSON.stringify(what.slice(3))}];`;
        else if (what === 'main')
            source = MAIN.map(n => `export const ${n} = globalThis.__ge1c.Main.${n};`).join('\n');
        else if (what === 'keyboard')
            source = 'export const getInputSourceManager = () => globalThis.__ge1c.inputSourceManager;';
        else if (what === 'messageTray')
            source = TRAY.map(n => `export const ${n} = globalThis.__ge1c.MessageTray.${n};`).join('\n');
        else
            source = 'export const Extension = globalThis.__ge1c.Extension;';
        return {format: 'module', source, shortCircuit: true};
    }
    if (url.startsWith('file:') && url.endsWith('.js')) {
        const result = await next(url, {...context, format: 'module'});
        return {...result, format: 'module'};
    }
    return next(url, context);
}
