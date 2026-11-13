// Instellingen › Startpagina: device hero, recommended settings, Bluetooth devices,
// personalisation quick switch, storage at a glance.

import { h, clear } from '../../../core/dom.js';
import { makeT, tr } from '../../../core/i18n.js';
import { bytes } from '../../../core/format.js';
import { icon } from '../../../core/icons.js';
import { ToggleSwitch, HyperlinkButton, Button, iconNode, severityIcon } from '../../../core/components.js';
import { get, run, settingDropdown, UsageBar, recentPages, volumePercent, showError } from '../common.js';
import { settingsPage } from '../registry.js';
import { deviceHero } from './system.js';
import { ThemeGrid } from './themes.js';
import { deviceGlyph, deviceStatus, toggleDevice, addDeviceHelp, isNotResponding, resetBluetooth } from './bluetooth.js';

const t = makeT({
  nl: {
    recommended: 'Aanbevolen instellingen',
    recommendedDesc: 'Recente en veelgebruikte instellingen',
    bt: 'Bluetooth-apparaten',
    btDesc: 'Apparaten beheren, toevoegen en verbinden',
    btOff: 'Bluetooth staat uit',
    btNone: 'Nog geen gekoppelde apparaten',
    btNA: 'Geen Bluetooth gevonden op deze computer',
    allDevices: 'Alle apparaten weergeven',
    add: 'Apparaat toevoegen',
    connect: 'Verbinden', disconnect: 'Verbreken',
    btStuck: 'Bluetooth reageert niet',
    btStuckDesc: 'Je apparaten kunnen nu geen verbinding maken.',
    btReset: 'Opnieuw starten',
    btResetFailed: 'Bluetooth opnieuw starten lukte niet',
    pers: 'Persoonlijke instellingen',
    persDesc: 'Kies een thema, of pas je achtergrond en kleuren aan',
    colorMode: 'Kleurmodus', light: 'Licht', dark: 'Donker',
    browse: 'Meer achtergronden, kleuren en thema’s',
    storage: 'Opslag',
    storageDesc: 'Ruimte op je systeemschijf',
    free: '{f} vrij van {t}',
    cleanup: 'Ruimte vrijmaken',
    details: 'Details',
  },
  en: {
    recommended: 'Recommended settings',
    recommendedDesc: 'Recent and commonly used settings',
    bt: 'Bluetooth devices',
    btDesc: 'Manage, add and connect devices',
    btOff: 'Bluetooth is off',
    btNone: 'No paired devices yet',
    btNA: 'No Bluetooth found on this computer',
    allDevices: 'View all devices',
    add: 'Add device',
    connect: 'Connect', disconnect: 'Disconnect',
    btStuck: 'Bluetooth is not responding',
    btStuckDesc: 'Your devices can’t connect right now.',
    btReset: 'Restart',
    btResetFailed: 'Couldn’t restart Bluetooth',
    pers: 'Personalise your device',
    persDesc: 'Pick a theme, or change your background and colours',
    colorMode: 'Colour mode', light: 'Light', dark: 'Dark',
    browse: 'Browse more backgrounds, colours and themes',
    storage: 'Storage',
    storageDesc: 'Space on your system drive',
    free: '{f} free of {t}',
    cleanup: 'Free up space',
    details: 'Details',
  },
});

const DEFAULT_RECOMMENDED = ['display', 'sound', 'taskbar', 'background'];

function homeCard({ title, desc, body, foot, cls }) {
  return h('section', { class: ['st-hcard', cls] },
    h('div', { class: 'st-hcard__head' }, h('h2', { class: 'st-hcard__title' }, title), desc ? h('p', { class: 'st-hcard__desc' }, desc) : null),
    h('div', { class: 'st-hcard__body' }, body),
    foot ? h('div', { class: 'st-hcard__foot' }, foot) : null);
}

function rowButton({ icon: ic, title, desc, trailing, onClick }) {
  return h('button', { type: 'button', class: 'st-hrow', onClick },
    h('span', { class: 'st-hrow__icon' }, iconNode(ic, 20)),
    h('span', { class: 'st-hrow__text' }, h('span', { class: 'st-hrow__title' }, title), desc ? h('span', { class: 'st-hrow__desc' }, desc) : null),
    trailing || h('span', { class: 'st-hrow__chev' }, icon('chevron-right', { size: 12 })));
}

export const pages = {
  async home(ctx) {
    const [info, net, upd, bt, vols] = await Promise.all([
      get('system.info'), get('system.network.get'), get('system.updates'), get('system.bluetooth.get'), get('storage.volumes'),
    ]);

    /* recommended */
    const rec = [...new Set([...recentPages(), ...DEFAULT_RECOMMENDED])].slice(0, 4);
    const recCard = homeCard({
      title: t('recommended'), desc: t('recommendedDesc'),
      body: h('div', { class: 'st-hlist' }, rec.map((id) => {
        const p = settingsPage(id);
        const parent = p.parent ? settingsPage(p.parent) : null;
        return rowButton({ icon: p.icon, title: tr(p.title), desc: parent ? tr(parent.title) : tr(p.description), onClick: () => ctx.go(id) });
      })),
    });

    /* bluetooth */
    const btBody = h('div', { class: 'st-hlist' });
    let btState = bt;
    let btNotReady = false;          // a connect answered "Bluetooth reageert niet" (until the host says otherwise)
    let btResetting = false;
    async function reloadBt() {
      const r = await get('system.bluetooth.get');
      if (r && ctx.alive()) { btState = r; if (r.stuck === false) btNotReady = false; paintBt(); }
    }
    async function resetBt() {
      if (btResetting) return;
      btResetting = true;
      paintBt();
      const r = await resetBluetooth();
      btResetting = false;
      if (!ctx.alive()) return;
      if (r.ok) { btNotReady = false; if (r.state) btState = r.state; }
      else showError({ message: r.error }, t('btResetFailed'));
      paintBt();
      if (r.ok && !r.state) reloadBt();
    }
    const btToggle = bt?.available ? ToggleSwitch({
      checked: !!bt.powered, ariaLabel: t('bt'),
      onChange: async (on) => {
        if (!(await run('system.bluetooth.setPowered', { powered: on }, { quietDry: true }))) btToggle.setChecked(!on);
        else { btState = { ...btState, powered: on }; paintBt(); }
      },
    }) : null;
    function paintBt() {
      clear(btBody);
      if (!btState?.available) { btBody.append(h('p', { class: 'st-hcard__empty' }, t('btNA'))); return; }
      btToggle?.setChecked(!!btState.powered);
      if (!btState.powered) { btBody.append(h('p', { class: 'st-hcard__empty' }, t('btOff'))); return; }
      const stuck = !!(btState.stuck || btNotReady);
      if (stuck || btResetting) {
        // the host's "Bluetooth reageert niet": restart it right here (same reset as the Bluetooth page)
        const resetBtn = Button({ label: t('btReset'), size: 'small', disabled: btResetting, onClick: (e) => { e.stopPropagation(); resetBt(); } });
        btBody.append(h('div', { class: 'st-hrow is-static', role: 'status' },
          h('span', { class: 'st-hrow__icon' }, severityIcon('warning', 20)),
          h('span', { class: 'st-hrow__text' }, h('span', { class: 'st-hrow__title' }, t('btStuck')), h('span', { class: 'st-hrow__desc' }, t('btStuckDesc'))),
          resetBtn));
      }
      const devs = [...(btState.devices || [])].sort((a, b) => b.connected - a.connected).slice(0, 3);
      if (!devs.length) { if (!stuck) btBody.append(h('p', { class: 'st-hcard__empty' }, t('btNone'))); return; }
      for (const d of devs) {
        const btn = Button({ label: d.connected && !stuck ? t('disconnect') : t('connect'), size: 'small', disabled: stuck || btResetting,
          onClick: async (e) => {
            e.stopPropagation();
            btn.disabled = true;
            // the host answers {ok:false, error, code} when it does not work: say so (a stuck adapter → the row above)
            const r = await toggleDevice(d);
            if (!ctx.alive()) return;
            if (!r.ok) {
              if (isNotResponding(r.error)) btNotReady = true;
              else showError(r.error);
            }
            await reloadBt();
            if (ctx.alive()) paintBt();
          } });
        btBody.append(h('div', { class: 'st-hrow is-static' },
          h('span', { class: 'st-hrow__icon' }, icon(deviceGlyph(d.icon || d.name), { size: 20 })),
          h('span', { class: 'st-hrow__text' }, h('span', { class: 'st-hrow__title' }, d.name), h('span', { class: 'st-hrow__desc' }, deviceStatus(d))),
          btn));
      }
    }
    paintBt();
    ctx.on('system.bluetooth.changed', (d) => { if (d) { btState = d; if (d.stuck === false) btNotReady = false; paintBt(); } });
    const btCard = homeCard({
      title: h('span', { class: 'st-hcard__title-row' }, t('bt'), btToggle),
      desc: t('btDesc'), body: btBody,
      foot: [HyperlinkButton({ label: t('allDevices'), onClick: () => ctx.go('bluetooth') }), bt?.available ? HyperlinkButton({ label: t('add'), onClick: addDeviceHelp }) : null],
    });

    /* personalisation */
    const persCard = homeCard({
      title: t('pers'), desc: t('persDesc'), cls: 'st-hcard--pers',
      body: [
        ThemeGrid(ctx, { compact: true }),
        h('div', { class: 'st-hrow is-static' },
          h('span', { class: 'st-hrow__icon' }, icon('theme', { size: 20 })),
          h('span', { class: 'st-hrow__text' }, h('span', { class: 'st-hrow__title' }, t('colorMode'))),
          settingDropdown(ctx, 'theme.mode', [{ value: 'light', label: t('light') }, { value: 'dark', label: t('dark') }], { width: 140 })),
      ],
      foot: HyperlinkButton({ label: t('browse'), onClick: () => ctx.go('personalization') }),
    });

    /* storage */
    const sys = Array.isArray(vols) ? (vols.find((v) => v.isHome) || vols.find((v) => v.isSystem) || vols[0]) : null;
    const cards = [recCard, btCard, persCard];
    if (sys) {
      const pct = volumePercent(sys);
      cards.push(homeCard({
        title: t('storage'), desc: t('storageDesc'),
        body: h('div', { class: 'st-hstore' },
          h('div', { class: 'st-hstore__row' },
            h('span', { class: 'st-hstore__icon' }, icon('drive', { size: 20 })),
            h('span', { class: 'st-hstore__name' }, `${sys.name} (${sys.mount})`),
            h('span', { class: 'st-hstore__pct tabular' }, `${Math.round(pct)}%`)),
          UsageBar({ pct }),
          h('div', { class: 'st-hstore__free' }, t('free', { f: bytes(sys.free), t: bytes(sys.total) }))),
        foot: [
          HyperlinkButton({ label: t('details'), onClick: () => ctx.go('storage') }),
          HyperlinkButton({ label: t('cleanup'), onClick: () => run('shell.openApp', { app: 'taskmgr', params: { page: 'storage' } }, { quietDry: true }) }),
        ],
      }));
    }
    return [
      deviceHero(ctx, { info, net, upd }),
      h('div', { class: 'st-home-grid' }, cards),
    ];
  },
};
