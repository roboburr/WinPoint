// Instellingen › Systeem › Info: device and OS specifications with "Kopiëren".

import { h } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { getShellInfo } from '../../../core/theme.js';
import { bytes, duration } from '../../../core/format.js';
import { SettingsCard, SettingsGroup, SettingsExpander, Button, Toast } from '../../../core/components.js';
import { get, run, copyText, notice, linkCard, DeviceArt, ct, osLabel } from '../common.js';
import { renameHint } from './system.js';

const t = makeT({
  nl: {
    renamePc: 'Naam van deze pc wijzigen',
    device: 'Apparaatspecificaties',
    os: 'Besturingssysteem',
    copy: 'Kopiëren',
    deviceName: 'Apparaatnaam', model: 'Model', cpu: 'Processor', ram: 'Geïnstalleerd RAM', gpu: 'Videokaart', disks: 'Opslag',
    edition: 'Editie', kernel: 'Kernel', desktop: 'Bureaublad', shell: 'Win11 Shell', uptime: 'Actief sinds opstarten', user: 'Gebruiker',
    cores: '{cores}, {threads}', nCores: { one: '{n} kern', other: '{n} kernen' }, nThreads: { one: '{n} thread', other: '{n} threads' },
    na: 'Niet alle informatie is beschikbaar op deze computer.',
    related: 'Gerelateerd',
    perf: 'Prestaties bekijken',
    perfDesc: 'Processor, geheugen en schijven live in Taakbeheer',
    mode_layer: 'bureaublad', mode_preview: 'voorbeeldvenster', mode_serve: 'browser', mode_app: 'los venster',
    testMode: 'testmodus',
  },
  en: {
    renamePc: 'Rename this PC',
    device: 'Device specifications',
    os: 'Operating system',
    copy: 'Copy',
    deviceName: 'Device name', model: 'Model', cpu: 'Processor', ram: 'Installed RAM', gpu: 'Graphics card', disks: 'Storage',
    edition: 'Edition', kernel: 'Kernel', desktop: 'Desktop', shell: 'Win11 Shell', uptime: 'Up time', user: 'User',
    cores: '{cores}, {threads}', nCores: { one: '{n} core', other: '{n} cores' }, nThreads: { one: '{n} thread', other: '{n} threads' },
    na: 'Not all information is available on this computer.',
    related: 'Related',
    perf: 'View performance',
    perfDesc: 'Processor, memory and disks live in Task Manager',
    testMode: 'test mode',
    mode_layer: 'desktop', mode_preview: 'preview window', mode_serve: 'browser', mode_app: 'separate window',
  },
});

function specList(rows) {
  return h('dl', { class: 'st-specs selectable' }, rows.filter((r) => r[1]).map(([k, v]) => [h('dt', null, k), h('dd', null, v)]));
}

function copyBtn(rows, title) {
  return Button({
    label: t('copy'),
    onClick: async () => {
      const text = `${title}\n` + rows.filter((r) => r[1]).map(([k, v]) => `${k}\t${v}`).join('\n');
      const ok = await copyText(text);
      Toast.show({ severity: ok ? 'success' : 'error', title: ok ? ct('copied') : ct('copyFailed'), timeout: 2200 });
    },
  });
}

export const pages = {
  async about(ctx) {
    const [info, upd] = await Promise.all([get('system.info'), get('system.updates')]);
    const sh = getShellInfo() || {};
    const out = [];
    if (!info) out.push(notice({ message: t('na') }));

    const art = DeviceArt({ small: true });
    ctx.onSettings((s) => art.update(s));
    out.push(h('section', { class: 'st-about-head' },
      art,
      h('div', { class: 'st-about-head__text' },
        h('div', { class: 'st-about-head__name' }, info?.hostname || '–'),
        h('div', { class: 'st-about-head__model' }, info?.deviceModel || '')),
      Button({ label: t('renamePc'), onClick: () => renameHint(info?.hostname) })));

    const devRows = [
      [t('deviceName'), info?.hostname],
      [t('model'), info?.deviceModel],
      [t('cpu'), info?.cpu?.model ? `${info.cpu.model}${info.cpu.cores ? ` (${t('cores', { cores: t('nCores', { n: info.cpu.cores }), threads: t('nThreads', { n: info.cpu.threads || info.cpu.cores }) })})` : ''}` : null],
      [t('ram'), info?.memory?.total ? bytes(info.memory.total) : null],
      [t('gpu'), (info?.gpus || []).join(', ') || null],
      [t('disks'), (info?.disks || []).map((d) => `${d.model || ''} (${bytes(d.size)})`.trim()).join(', ') || null],
    ];
    const modeName = (m) => { const k = `mode_${m}`; const v = t(k); return v === k ? m : v; };
    const osRows = [
      [t('edition'), osLabel(info?.os, info?.osVersion) || null],
      [t('kernel'), info?.kernel],
      [t('desktop'), info?.desktop || (sh.compositor ? sh.compositor[0].toUpperCase() + sh.compositor.slice(1) : null)],
      [t('shell'), sh.version ? `${sh.version}${sh.mode ? ` · ${modeName(sh.mode)}` : ''}${sh.dryRun ? ` · ${t('testMode')}` : ''}` : null],
      [t('uptime'), info?.uptime != null ? duration(info.uptime, { long: true }) : null],
      [t('user'), info?.user ? `${info.user.realName || info.user.name} (${info.user.name})` : null],
    ];

    out.push(SettingsGroup({ children: [
      SettingsExpander({ icon: 'pc', title: t('device'), expanded: true, control: copyBtn(devRows, t('device')), children: [specList(devRows)] }),
      SettingsExpander({ icon: 'info', title: t('os'), expanded: true, control: copyBtn(osRows, t('os')), children: [specList(osRows)] }),
    ] }));

    out.push(SettingsGroup({ title: t('related'), children: [
      linkCard(ctx, 'storage'),
      SettingsCard({ icon: 'chart', title: t('perf'), description: t('perfDesc'), chevron: 'external',
        onClick: () => run('shell.openApp', { app: 'taskmgr', params: { page: 'performance' } }, { quietDry: true }) }),
      upd ? linkCard(ctx, 'update') : null,
    ].filter(Boolean) }));
    return out;
  },
};
