// Taakbeheer › Opslag — Categorieën: what takes space in the home folder, as a donut +
// coloured legend list (Documenten, Afbeeldingen, … Chat-apps, Ontwikkeltools, Cache).
// Each row opens that folder in the explorer.

import { h, clear } from '../../../core/dom.js';
import { Button, ProgressRing, InfoBar } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { Donut } from '../../../core/charts.js';
import { makeT } from '../../../core/i18n.js';
import { bytes, number, relTime } from '../../../core/format.js';
import { CATEGORY_COLORS, prettyPath } from '../util.js';

const t = makeT({
  nl: {
    title: 'Wat neemt ruimte in je persoonlijke map?', computing: 'Berekenen hoeveel ruimte elke categorie gebruikt…',
    refresh: 'Opnieuw berekenen', total: 'in totaal', home: 'Persoonlijke map', explore: 'Verkennen', failed: 'Kon de categorieën niet berekenen',
    hint: 'Klik op een categorie om te zien wat erin zit.', items: 'Onderdelen',
    progress: '{files} bestanden bekeken · {size}', updating: 'Bijwerken…', updated: 'Bijgewerkt {when}',
  },
  en: {
    title: 'What takes up space in your home folder?', computing: 'Working out how much space each category uses…',
    refresh: 'Calculate again', total: 'in total', home: 'Home folder', explore: 'Explore', failed: 'Couldn’t calculate the categories',
    hint: 'Click a category to see what’s in it.', items: 'Parts',
    progress: '{files} files checked · {size}', updating: 'Updating…', updated: 'Updated {when}',
  },
});

const CAT_ICONS = {
  documents: 'document', pictures: 'image', videos: 'video', music: 'music', downloads: 'download', desktop: 'desktop',
  trash: 'trash', browser: 'globe', chat: 'people', dev: 'code', appdata: 'apps', snaps: 'app', flatpak: 'app', cache: 'clean', other: 'more',
};

export function createCategories(ctx, { onExplore } = {}) {
  const { bridge } = ctx;
  let data = null;
  let loading = false;

  const donutWrap = h('div', { class: 'tm-cat__donut' });
  const legend = h('div', { class: 'tm-cat__legend', role: 'list' });
  const refreshBtn = Button({ label: t('refresh'), icon: 'refresh', variant: 'subtle', onClick: () => load(true) });
  const status = h('span', { class: 'tm-cat__status caption text-2' });
  const body = h('div', { class: 'tm-cat__body' });
  const el = h('div', { class: 'tm-cat' },
    h('div', { class: 'tm-cat__header' }, h('div', null, h('h3', { class: 'tm-section__title' }, t('title')), h('p', { class: 'caption text-2' }, t('hint'))),
      h('div', { class: 'tm-cat__right' }, status, refreshBtn)),
    body);
  let progressEl = null;

  bridge.on('storage.categoriesReady', (d) => { if (d?.data) { data = d.data; loading = false; status.textContent = ''; render(); } });
  bridge.on('storage.categoriesProgress', (d) => {
    const txt = t('progress', { files: number(d?.files || 0), size: bytes(d?.bytes || 0) });
    if (progressEl?.isConnected) progressEl.textContent = txt;
    else if (data) status.textContent = `${t('updating')} ${txt}`;
  });

  async function load(refresh = false) {
    loading = true;
    progressEl = h('div', { class: 'caption text-3' });
    if (!data) clear(body).append(h('div', { class: 'tm-card tm-scan' }, ProgressRing({ size: 40 }), h('div', { class: 'text-2' }, t('computing')), progressEl));
    else status.textContent = t('updating');
    try {
      const r = await bridge.call('storage.categories', { refresh });
      if (r?.data) { data = r.data; render(); }
      if (r?.ready) loading = false;
      status.textContent = r?.running ? t('updating') : (data?.scannedAt ? t('updated', { when: relTime(data.scannedAt) }) : '');
    } catch (e) {
      loading = false;
      clear(body).append(InfoBar({ severity: 'error', title: t('failed'), message: e.message || '', closable: false, action: { label: t('refresh'), onClick: () => load(true) } }));
    }
  }

  function render() {
    if (!data) return;
    const cats = (data.categories || []).filter((c) => c.size > 0).sort((a, b) => (a.id === 'other') - (b.id === 'other') || b.size - a.size);
    const total = data.total || cats.reduce((a, c) => a + c.size, 0);
    const donut = Donut({
      segments: cats.map((c) => ({ value: c.size, color: CATEGORY_COLORS[c.id] || 'var(--tm-cat-other)', label: `${c.name}: ${bytes(c.size)}` })),
      size: 220, thickness: 26, gap: 2, label: bytes(total), sublabel: t('total'),
    });
    clear(donutWrap).append(donut, h('div', { class: 'tm-cat__home' }, icon('home', { size: 14 }), h('span', null, prettyPath(data.home || '~'))));
    clear(legend);
    const max = Math.max(...cats.map((c) => c.size), 1);
    for (const c of cats) {
      const color = CATEGORY_COLORS[c.id] || 'var(--tm-cat-other)';
      const pct = total ? (c.size / total) * 100 : 0;
      const row = h('button', { type: 'button', class: 'tm-cat__row', role: 'listitem', dataset: { id: c.id, tooltip: prettyPath(c.path || '') } },
        h('span', { class: 'tm-cat__swatch', style: { background: color } }, icon(CAT_ICONS[c.id] || 'folder', { size: 14 })),
        h('span', { class: 'tm-cat__main' },
          h('span', { class: 'tm-cat__top' }, h('span', { class: 'tm-cat__name' }, c.name), h('span', { class: 'tm-cat__size' }, bytes(c.size))),
          h('span', { class: 'tm-cat__barline' },
            h('span', { class: 'tm-cat__bar' }, h('span', { style: { width: `${Math.max(1, (c.size / max) * 100)}%`, background: color } })),
            h('span', { class: 'tm-cat__pct' }, `${number(pct, pct < 10 ? 1 : 0)}%`))),
        icon('chevron-right', { size: 12 }));
      row.addEventListener('click', () => onExplore?.(c.path, c.name));
      legend.append(row);
      if (Array.isArray(c.items) && c.items.length) {
        const sub = h('div', { class: 'tm-cat__subs' });
        for (const it of c.items.filter((x) => x.size > 0).sort((a, b) => b.size - a.size)) {
          const s = h('button', { type: 'button', class: 'tm-cat__sub', dataset: { tooltip: prettyPath(it.path || '') } },
            h('span', { class: 'tm-cat__subdot', style: { background: color } }), h('span', { class: 'grow' }, it.name), h('span', { class: 'tm-cat__size' }, bytes(it.size)));
          s.addEventListener('click', () => onExplore?.(it.path, it.name));
          sub.append(s);
        }
        legend.append(sub);
      }
    }
    clear(body).append(h('div', { class: 'tm-card tm-cat__card' }, donutWrap, legend));
  }

  return {
    el,
    load() { if (!data && !loading) load(false); else if (data) render(); },
    refresh: () => load(true),
    get home() { return data?.home || null; },
  };
}
