// Win11 Shell — router (owned by ui-core). SPEC §7 / §11.
//   index.html?surface=app&app=settings|taskmgr|explorer|alles|disks|defaultapps|installer
// Only the HTML apps are served from here (D-G70): the taskbar, Start, the desktop and their flyouts are the
// native GTK 4 shell (native/shell, win11-shell), so the old surfaces preview, desktop, shell and gallery and
// their modules are gone and any surface but 'app' gets the unknown-view placeholder.
// The theme is applied first (initTheme) so nothing flashes; each app module is
// imported dynamically and a friendly placeholder replaces anything that fails to load.

import { initTheme } from './core/theme.js';
import { bridge } from './core/bridge.js';
import { h, clear } from './core/dom.js';
import { installTooltips, Button, severityIcon } from './core/components.js';
import { makeT } from './core/i18n.js';

const t = makeT({
  nl: {
    failTitle: 'Dit onderdeel kon niet worden geladen',
    failBody: 'Er ging iets mis bij het starten van {what}. Probeer het opnieuw; helpt dat niet, start Win11 Shell dan opnieuw op.',
    retry: 'Opnieuw proberen',
    unknownApp: 'Deze app bestaat niet',
    unknownAppBody: 'De app “{app}” is niet gevonden.',
    unknownSurface: 'Onbekende weergave',
    desktop: 'het bureaublad', shell: 'de taakbalk', wm: 'de vensters', app: 'deze app', gallery: 'de componentengalerij',
  },
  en: {
    failTitle: 'This part couldn’t be loaded',
    failBody: 'Something went wrong while starting {what}. Try again; if that doesn’t help, restart Win11 Shell.',
    retry: 'Try again',
    unknownApp: 'This app doesn’t exist',
    unknownAppBody: 'The app “{app}” couldn’t be found.',
    unknownSurface: 'Unknown view',
    desktop: 'the desktop', shell: 'the taskbar', wm: 'the windows', app: 'this app', gallery: 'the component gallery',
  },
});

const APPS = ['settings', 'taskmgr', 'explorer', 'alles', 'disks', 'defaultapps', 'installer'];
const surface = bridge.params.get('surface') || 'preview';
const root = document.getElementById('root') || document.body.appendChild(h('div', { id: 'root' }));

if (bridge.params.get('debug') === '1') installDebugLog();

function placeholder(container, what, error, { compact = false, title = t('failTitle'), body } = {}) {
  const details = error ? String(error?.stack || error?.message || error).split('\n').slice(0, 4).join('\n') : '';
  const card = h('div', { class: ['fl-placeholder', compact && 'is-compact'], role: 'alert', 'data-region': compact ? '' : null },
    severityIcon('warning', 32),
    h('div', { class: 'subtitle' }, title),
    h('div', { class: 'text-2', style: { maxWidth: '460px' } }, body || t('failBody', { what: t(what) })),
    Button({ label: t('retry'), variant: 'accent', onClick: () => location.reload() }),
    details && bridge.params.get('debug') === '1' ? h('div', { class: 'fl-placeholder__details' }, details) : null);
  if (compact) {
    Object.assign(card.style, {
      position: 'fixed', left: '50%', bottom: '64px', transform: 'translateX(-50%)', height: 'auto',
      width: 'min(480px, calc(100% - 32px))', borderRadius: '8px', boxShadow: 'var(--shadow-flyout)',
      border: '1px solid var(--stroke-flyout)', padding: '24px',
    });
  }
  if (!compact) clear(container);
  container.append(card);
  return card;
}

async function load(path, exportName, container, args, what, opts) {
  try {
    const mod = await import(path);
    const fn = mod[exportName];
    if (typeof fn !== 'function') throw new Error(`${path} exports no ${exportName}()`);
    return await fn(container, ...args);
  } catch (e) {
    console.error(`[main] ${path} → ${exportName}() failed`, e);
    placeholder(container, what, e, opts);
    return null;
  }
}

async function start() {
  await initTheme(surface);
  installTooltips();

  switch (surface) {
    case 'app': {
      const app = bridge.params.get('app');
      if (!APPS.includes(app)) {
        placeholder(root, 'app', null, { title: t('unknownApp'), body: t('unknownAppBody', { app: app || '' }) });
        break;
      }
      const params = Object.fromEntries(bridge.params.entries());
      await load(`./apps/${app}/main.js`, 'mountApp', root, [params], 'app');
      break;
    }
    default:
      placeholder(root, 'app', null, { title: t('unknownSurface'), body: surface });
  }
  document.documentElement.dataset.ready = 'true';
}

/** ?debug=1 — collects errors in a <pre id="debug-log"> (visible in --dump-dom output). */
function installDebugLog() {
  const pre = h('pre', {
    id: 'debug-log',
    style: { position: 'fixed', right: '8px', top: '8px', zIndex: 99999, maxWidth: '50vw', maxHeight: '40vh', overflow: 'auto', margin: 0, padding: '6px 8px', font: '11px/1.4 monospace', background: 'rgba(196,43,28,.92)', color: '#fff', borderRadius: '4px', pointerEvents: 'none', whiteSpace: 'pre-wrap' },
    hidden: true,
  });
  const add = (msg) => { pre.hidden = false; pre.textContent += msg + '\n'; if (!pre.isConnected) document.body.appendChild(pre); };
  addEventListener('error', (e) => add(`error: ${e.message} @ ${e.filename}:${e.lineno}`));
  addEventListener('unhandledrejection', (e) => add(`rejection: ${e.reason?.stack || e.reason}`));
  const origErr = console.error.bind(console);
  console.error = (...a) => { origErr(...a); add('console.error: ' + a.map((x) => (x?.stack || x?.message || String(x))).join(' ')); };
  document.addEventListener('DOMContentLoaded', () => { if (!pre.hidden) document.body.appendChild(pre); });
}

start().catch((e) => {
  console.error('[main] start failed', e);
  placeholder(root, 'app', e);
});
