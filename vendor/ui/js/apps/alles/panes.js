// Alles — right-hand panes: preview (thumbnail + details) and search tips.

import { h, clear } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bridge } from '../../core/bridge.js';
import { Button, IconButton } from '../../core/components.js';
import { bytes, fileTime, number } from '../../core/format.js';
import { tr } from '../../core/i18n.js';
import { t, typeName } from './i18n.js';

const THUMB_EXT = new Set(['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'heif', 'avif', 'tif', 'tiff', 'ico', 'jxl']);

function bigIcon(item, size = 64) {
  const px = Math.round(size * Math.min(2, Math.max(1, devicePixelRatio || 1)));
  const url = item.isDir ? bridge.iconUrl('folder', px) : bridge.mimeIconUrl(item.mime || 'application/octet-stream', px);
  const img = h('img', { class: 'alles-pv__icon', src: url || '', width: size, height: size, alt: '', draggable: false });
  img.addEventListener('error', () => { img.replaceWith(h('span', { class: 'alles-pv__glyph' }, icon(item.isDir ? 'folder' : 'file', { size: 48 }))); }, { once: true });
  return img;
}

/** PreviewPane({onOpen(item), onOpenFolder(item)}) → {el, show(item), showMany(count, sizeOrNull)} */
export function PreviewPane({ onOpen, onOpenFolder } = {}) {
  const media = h('div', { class: 'alles-pv__media' });
  const body = h('div', { class: 'alles-pv__body' });
  const el = h('aside', { class: 'alles-pane alles-preview', 'aria-label': t('preview') }, media, body);
  let current = null;

  function empty() {
    current = null;
    clear(media).append(h('span', { class: 'alles-pv__glyph is-muted' }, icon('view', { size: 40 })));
    media.classList.add('is-empty');
    clear(body).append(h('p', { class: 'alles-pv__hint' }, t('pv_none')));
  }

  function row(label, value, opts = {}) {
    return h('div', { class: 'alles-pv__row' }, h('dt', null, label),
      h('dd', { class: ['selectable', opts.cls] }, value));
  }

  function show(item) {
    if (!item) { empty(); return; }
    if (current && current.path === item.path && current.modified === item.modified && current.size === item.size) return;
    current = item;
    media.classList.remove('is-empty');
    clear(media);
    const thumb = !item.isDir && THUMB_EXT.has(item.ext) ? bridge.thumbUrl(item.path, 512) : '';
    if (thumb) {
      const img = h('img', { class: 'alles-pv__thumb', src: thumb, alt: '', draggable: false, decoding: 'async' });
      img.addEventListener('error', () => { clear(media).append(bigIcon(item)); }, { once: true });
      media.append(img);
    } else media.append(bigIcon(item));
    const dir = item.dir || '/';
    const parts = dir.split('/').flatMap((p, i) => (i ? [h('wbr'), '/' + p] : [p]));
    const loc = h('button', { type: 'button', class: 'alles-pv__link', dataset: { tooltip: dir } },
      icon('folder-open', { size: 14 }), h('span', null, parts));
    loc.addEventListener('click', () => onOpenFolder?.(item));
    clear(body).append(
      h('div', { class: 'alles-pv__name selectable' }, item.name),
      h('div', { class: 'alles-pv__type' }, typeName(item)),
      h('dl', { class: 'alles-pv__props' },
        row(item.isDir ? t('pv_contains') : t('pv_size'), item.isDir && !item.size ? '–' : bytes(item.size)),
        row(t('pv_modified'), item.modified ? fileTime(item.modified) : '–'),
        h('div', { class: 'alles-pv__row is-block' }, h('dt', null, t('pv_location')), h('dd', null, loc))),
      h('div', { class: 'alles-pv__actions' },
        Button({ label: t('pv_open'), variant: 'accent', icon: 'open', onClick: () => onOpen?.(item) }),
        Button({ label: t('pv_openFolder'), icon: 'folder-open', onClick: () => onOpenFolder?.(item) })),
    );
  }

  function showMany(n, size) {
    current = null;
    media.classList.add('is-empty');
    clear(media).append(h('span', { class: 'alles-pv__stack' }, icon('copy', { size: 40 })));
    clear(body).append(
      h('div', { class: 'alles-pv__name' }, t('pv_multi', { n, c: number(n) })),
      size != null ? h('dl', { class: 'alles-pv__props' }, row(t('pv_total'), bytes(size))) : null);
  }

  empty();
  return { el, show, showMany, clear: empty };
}

/* --------------------------------------------------------------- search tips */

// Examples per language ({nl, en}: the search understands the words of both, so each language shows its own);
// the tip texts (tp_*) quote them.
const TIP_GROUPS = [
  ['tg_basic', [[{ nl: 'foto', en: 'photo' }, 'tp_word'], [{ nl: 'vakantie 2024', en: 'holiday 2024' }, 'tp_and'],
    [{ nl: '"mijn document"', en: '"my document"' }, 'tp_exact']]],
  ['tg_combine', [['jpg|png', 'tp_or'], [{ nl: 'rapport !concept', en: 'report !draft' }, 'tp_not'],
    [{ nl: '<foto|video> 2024', en: '<photo|video> 2024' }, 'tp_group']]],
  ['tg_wild', [['*.pdf', 'tp_star'], ['IMG_00??.jpg', 'tp_q']]],
  ['tg_filters', [['ext:mp3;flac', 'tp_ext'], ['size:>1gb', 'tp_size'], [{ nl: 'dm:vandaag', en: 'dm:today' }, 'tp_dm'],
    ['dm:2026-10-01..2026-10-25', 'tp_dmRange'], [{ nl: 'folder:vakantie', en: 'folder:holiday' }, 'tp_folder'],
    [{ nl: 'file: notities', en: 'file: notes' }, 'tp_file'], ['pic:', 'tp_cat'], ['empty:', 'tp_empty']]],
  ['tg_places', [['parent:~/Downloads', 'tp_parent'], [{ nl: 'in:~/Documenten factuur', en: 'in:~/Documents invoice' }, 'tp_in'],
    [{ nl: 'path:werk offerte', en: 'path:work quote' }, 'tp_path']]],
  ['tg_expert', [['case:IMG', 'tp_case'], [{ nl: 'ww:map', en: 'ww:folder' }, 'tp_ww'], ['regex:^IMG_\\d+', 'tp_regex']]],
];

const KEYS = [['Enter', 'tk_open'], ['Ctrl+Enter', 'tk_folder'], ['Ctrl+C', 'tk_copy'], ['Ctrl+F', 'tk_search'], ['Alt+Enter', 'tk_props'], ['Delete', 'tk_trash']];

/** TipsPane({onExample(query), onClose}) → {el} */
export function TipsPane({ onExample, onClose } = {}) {
  const groups = TIP_GROUPS.map(([g, rows]) => h('section', { class: 'alles-tips__group' },
    h('h3', { class: 'alles-tips__heading' }, t(g)),
    rows.map(([ex, d]) => {
      const q = tr(ex);
      const chip = h('button', { type: 'button', class: 'alles-chip alles-chip--code', dataset: { q } }, q);
      chip.addEventListener('click', () => onExample?.(q));
      return h('div', { class: 'alles-tips__row' }, chip, h('p', { class: 'alles-tips__desc' }, t(d)));
    })));
  const keys = h('section', { class: 'alles-tips__group' },
    h('h3', { class: 'alles-tips__heading' }, t('tipsKeys')),
    h('dl', { class: 'alles-tips__keys' }, KEYS.map(([k, d]) => h('div', { class: 'alles-tips__key' },
      h('dt', null, k.split('+').map((p, i) => [i ? h('span', { class: 'alles-kbd-plus' }, '+') : null, h('kbd', null, p)])),
      h('dd', null, t(d))))));
  const close = IconButton({ icon: 'close', label: t('close'), size: 32, iconSize: 14, onClick: () => onClose?.() });
  const el = h('aside', { class: 'alles-pane alles-tips', 'aria-label': t('tipsTitle') },
    h('div', { class: 'alles-tips__head' }, h('h2', { class: 'alles-tips__title' }, t('tipsTitle')), close),
    h('div', { class: 'alles-tips__scroll' },
      h('p', { class: 'alles-tips__intro' }, t('tipsIntro')),
      groups, keys));
  return { el };
}
