// Win11 Shell — Verkenner: details pane (preview / big icon, name, type and properties of the selection).

import { h, clear, debounce, appendChildren } from '../../core/dom.js';
import { bridge } from '../../core/bridge.js';
import { t } from './i18n.js';
import { fileIcon, typeName, dateTime, dirname, canThumb, sizeText } from './util.js';

const VIDEO_RE = /\.(mp4|m4v|mov|mkv|webm|avi|mpg|mpeg|ogv|wmv|flv|mts|m2ts)$/i;
const isVideo = (item) => !item.isDir && !item.volume && ((item.mime || '').startsWith('video/') || VIDEO_RE.test(item.name || item.path || ''));
const IMG_RE = /\.(png|jpe?g|gif|webp|bmp|svg|avif|ico|tiff?)$/i;

export function DetailsPane(app) {
  const el = h('aside', { class: 'xp-details', 'aria-label': t('detailsPane'), tabindex: -1 });
  let seq = 0;
  let mode = 'details';   // 'details' | 'preview' (Voorbeeldvenster, Alt+P)

  /** Voorbeeldvenster: a large picture of the selected image (thumbnail service), else Windows' messages. */
  function renderPreview() {
    ++seq;
    clear(el);
    const sel = app.tab ? app.selectedItems() : [];
    const item = sel.length === 1 ? sel[0] : null;
    if (!item || item.isDir || item.volume) {
      el.append(h('div', { class: 'xp-preview__msg' }, t('previewHint')));
      return;
    }
    const isImage = (item.mime || '').startsWith('image/') || IMG_RE.test(item.name || item.path || '');
    if (!isImage && !canThumb(item)) { el.append(h('div', { class: 'xp-preview__msg' }, t('noPreview'))); return; }
    const px = Math.round(Math.min(1024, Math.max(320, (el.clientWidth || 300) * 2 * (devicePixelRatio || 1))));
    const img = h('img', { class: 'xp-preview__img', alt: item.displayName || item.name || '', draggable: false, src: bridge.thumbUrl(item.path, px) });
    img.addEventListener('error', () => { img.replaceWith(h('div', { class: 'xp-preview__msg' }, t('noPreview'))); }, { once: true });
    el.append(h('div', { class: 'xp-preview' }, img));
  }

  const prop = (label, value) => h('div', { class: 'xp-dprop' },
    h('span', { class: 'xp-dprop__label' }, label), h('span', { class: 'xp-dprop__value selectable' }, value || '–'));

  function render() {
    if (mode === 'preview') { renderPreview(); return; }
    const my = ++seq;
    const tab = app.tab;
    clear(el);
    if (!tab) return;
    const sel = app.selectedItems();
    if (!sel.length) {
      const n = app.visibleCount();
      appendChildren(el, [h('div', { class: 'xp-details__preview is-icon' }, app.locIcon(tab.loc, 96)),
        h('div', { class: 'xp-details__name' }, app.locLabel(tab.loc)),
        n != null ? h('div', { class: 'xp-details__type' }, t('itemsInFolder', { n })) : null,
        tab.loc.kind === 'folder' || tab.loc.kind === 'search' ? null : h('div', { class: 'xp-details__hint' }, t('noSelection'))]);
      return;
    }
    if (sel.length > 1) {
      const stack = h('div', { class: 'xp-details__stack' }, sel.slice(0, 3).reverse().map((it) => fileIcon(it, 72, { thumb: true })));
      const total = sel.reduce((a, x) => a + (x.isDir ? 0 : (x.size || 0)), 0);
      appendChildren(el, [h('div', { class: 'xp-details__preview is-icon' }, stack),
        h('div', { class: 'xp-details__name' }, t('selected', { n: sel.length })),
        h('div', { class: 'xp-details__props' }, total ? prop(t('totalSize'), sizeText({ size: total })) : null)]);
      return;
    }
    const item = sel[0];
    const video = isVideo(item);
    const big = canThumb(item) && !video;
    const preview = h('div', { class: ['xp-details__preview', big ? 'is-thumb' : 'is-icon'] }, fileIcon(item, big ? 240 : 96, { thumb: big }));
    if (video && !el.hidden) {
      // Keep the existing file icon while the bounded poster worker runs. No video player starts.
      const px = Math.round(Math.min(1024, 240 * Math.min(3, Math.max(1, devicePixelRatio || 1))));
      const poster = h('img', { alt: item.displayName || item.name || '', draggable: false, decoding: 'async' });
      poster.addEventListener('load', () => {
        if (my !== seq) return;
        clear(preview).append(h('span', { class: 'xp-ficon is-thumb', style: { width: '240px', height: '240px' }, 'aria-hidden': 'true' }, poster));
        preview.classList.remove('is-icon'); preview.classList.add('is-thumb');
      }, { once: true });
      const url = bridge.thumbUrl(item.path, px);
      poster.src = url && !url.startsWith('data:') ? `${url}${url.includes('?') ? '&' : '?'}video=1&modified=${encodeURIComponent(String(item.modified || 0))}` : url;
    }
    const created = h('span', null, '…');
    const props = h('div', { class: 'xp-details__props' },
      item.isDir ? null : prop(t('size'), sizeText(item)),
      prop(t('modified'), dateTime(item.modified)),
      h('div', { class: 'xp-dprop' }, h('span', { class: 'xp-dprop__label' }, t('created')), h('span', { class: 'xp-dprop__value' }, created)),
      prop(t('location'), app.friendlyDir(item.inTrash || item.trashName ? app.origDir(item) : dirname(item.path))));
    appendChildren(el, [preview, h('div', { class: 'xp-details__name selectable' }, app.itemName(item)),
      h('div', { class: 'xp-details__type' }, item.volume ? t('localDisk') : typeName(item)), props]);
    if (item.volume) { created.parentElement.remove(); return; }
    app.call('files.properties', { path: item.path }).then((pr) => {
      if (my !== seq) return;
      created.textContent = dateTime(pr?.created) || '–';
      if (item.isDir && pr?.contains) {
        props.prepend(prop(t('contains'), t('containsVal', { files: pr.contains.files || 0, folders: pr.contains.folders || 0 })));
        if (pr.size != null) props.prepend(prop(t('size'), sizeText({ size: pr.size })));
      }
    }).catch(() => { if (my === seq) created.textContent = '–'; });
  }

  const update = debounce(render, 60);
  function setMode(m) {
    const next = m === 'preview' ? 'preview' : 'details';
    if (next === mode) return;
    mode = next;
    el.classList.toggle('is-preview', mode === 'preview');
    el.setAttribute('aria-label', mode === 'preview' ? t('previewPane') : t('detailsPane'));
  }
  return { el, update, render, setMode, get mode() { return mode; } };
}
