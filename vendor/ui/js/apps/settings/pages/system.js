// Instellingen › Systeem (overview): device hero + links to the System sub-pages.

import { h } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { relTime } from '../../../core/format.js';
import { HyperlinkButton, SettingsGroup, Dialog } from '../../../core/components.js';
import { wifiIcon } from '../../../core/icons.js';
import { get, linkCard, DeviceArt, Hero, HeroTile, copyText, osLabel } from '../common.js';

const t = makeT({
  nl: {
    thisPc: 'Deze pc',
    rename: 'Naam wijzigen',
    renameTitle: 'De naam van deze pc wijzigen',
    renameBody: 'Deze pc heet nu “{name}”. Je wijzigt de naam in Ubuntu: open de app Instellingen van Ubuntu en ga naar Systeem › Info › Apparaatnaam. Ubuntu vraagt daarbij om je wachtwoord.',
    copyName: 'Naam kopiëren',
    network: 'Netwerk',
    connected: 'Verbonden',
    notConnected: 'Niet verbonden',
    updates: 'Updates',
    upToDate: 'Up-to-date',
    available: { one: '1 update beschikbaar', other: '{n} updates beschikbaar' },
    checked: 'Gecontroleerd {when}',
  },
  en: {
    thisPc: 'This PC',
    rename: 'Rename',
    renameTitle: 'Rename this PC',
    renameBody: 'This PC is called “{name}”. To change the name, open Ubuntu’s Settings app and go to System › About › Device Name. Ubuntu will ask for your password.',
    copyName: 'Copy name',
    network: 'Network',
    connected: 'Connected',
    notConnected: 'Not connected',
    updates: 'Updates',
    upToDate: 'Up to date',
    available: { one: '1 update available', other: '{n} updates available' },
    checked: 'Checked {when}',
  },
});

/** Explains how to rename the computer (we never change the hostname ourselves). */
export async function renameHint(name) {
  const r = await Dialog.show({
    title: t('renameTitle'), content: t('renameBody', { name: name || t('thisPc') }),
    primary: 'OK', secondary: t('copyName'), close: false,
  });
  if (r === 'secondary') copyText(name || '');
}

export function networkTileStatus(net) {
  if (!net) return null;
  const p = net.primary || {};
  if (p.type === 'none' || !p.type) return { icon: 'wifi-off', title: t('network'), status: t('notConnected') };
  return { icon: p.type === 'wifi' ? wifiIcon(p.strength ?? 70) : 'ethernet', title: p.name || t('network'), status: t('connected') };
}

export function updateTileStatus(u) {
  if (!u) return null;
  const n = u.available;
  const status = n == null ? '' : n > 0 ? t('available', { n }) : t('upToDate');
  return { icon: 'update', title: t('updates'), status: [status, u.checkedAt ? t('checked', { when: relTime(u.checkedAt) }) : ''].filter(Boolean) };
}

/** Device hero shared by Startpagina and Systeem. */
export function deviceHero(ctx, { info, net, upd, small = false }) {
  const art = DeviceArt({ small });
  ctx.onSettings((s) => art.update(s));
  const tiles = [];
  const ns = networkTileStatus(net);
  if (ns) tiles.push(HeroTile({ ...ns, onClick: () => ctx.go('network') }));
  const us = updateTileStatus(upd);
  let updTile = us ? HeroTile({ ...us, onClick: () => ctx.go('update') }) : null;
  if (updTile) tiles.push(updTile);
  const aside = tiles.length ? h('div', { class: 'st-hero__tiles' }, tiles) : null;
  // keep the update count current when the host re-checks (system.updates.changed)
  if (aside) {
    ctx.on('system.updates.changed', (d) => {
      const s = updateTileStatus(d);
      if (!s) return;
      const nt = HeroTile({ ...s, onClick: () => ctx.go('update') });
      if (updTile?.isConnected) updTile.replaceWith(nt); else aside.append(nt);
      updTile = nt;
    });
  }
  return Hero({
    art,
    title: info?.hostname || t('thisPc'),
    lines: [info?.deviceModel || osLabel(info?.os, info?.osVersion)],
    actions: HyperlinkButton({ label: t('rename'), onClick: () => renameHint(info?.hostname) }),
    aside,
  });
}

export const pages = {
  async system(ctx) {
    const [info, net, upd] = await Promise.all([get('system.info'), get('system.network.get'), get('system.updates')]);
    return [
      deviceHero(ctx, { info, net, upd }),
      SettingsGroup({
        children: ['display', 'sound', 'notifications', 'power', 'storage', 'clipboard', 'about'].map((id) => linkCard(ctx, id)),
      }),
    ];
  },
};
