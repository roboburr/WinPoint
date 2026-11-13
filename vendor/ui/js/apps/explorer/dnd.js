// Win11 Shell — Verkenner: shared drag & drop state + the "Verplaatsen naar …" hint under the cursor.

import { h } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { uriToPath } from './util.js';

export const DND_MIME = 'application/x-win11shell-paths';

/** The drag that started inside this window (dataTransfer data is unreadable during dragover). */
export const dnd = { current: null };

export function startDrag(paths, srcDir) { dnd.current = { paths: paths.slice(), srcDir }; }
export function endDrag() { dnd.current = null; hideHint(); }

/** Does this drag carry files we can accept? */
export function isFileDrag(e) {
  if (dnd.current) return true;
  const types = Array.from(e.dataTransfer?.types || []);
  return types.includes(DND_MIME) || types.includes('text/uri-list') || types.includes('Files');
}

/** Paths of a drop (internal drag, our MIME type, or file:// URIs from another app). */
export function dropPaths(e) {
  if (dnd.current) return { paths: dnd.current.paths.slice(), external: false, srcDir: dnd.current.srcDir };
  const dt = e.dataTransfer;
  try {
    const own = dt.getData(DND_MIME);
    if (own) { const p = JSON.parse(own); if (Array.isArray(p) && p.length) return { paths: p, external: false }; }
  } catch { /* ignore */ }
  const uris = (dt.getData('text/uri-list') || '').split(/\r?\n/).filter((l) => l && !l.startsWith('#'));
  const paths = uris.map(uriToPath).filter(Boolean);
  return { paths, external: true };
}

let hintEl = null;
export function showHint(x, y, text, kind = 'move') {
  if (!hintEl) {
    hintEl = h('div', { class: 'xp-drophint acrylic surface-flyout', role: 'status', 'data-region': '' },
      h('span', { class: 'xp-drophint__icon' }), h('span', { class: 'xp-drophint__text' }));
  }
  if (!hintEl.isConnected) document.body.append(hintEl);
  hintEl.dataset.theme = document.documentElement.dataset.theme;
  const ic = hintEl.firstChild;
  if (ic.dataset.kind !== kind) {
    ic.replaceChildren(icon(kind === 'copy' ? 'plus' : (kind === 'trash' ? 'delete' : 'arrow-right'), { size: 12 }));
    ic.dataset.kind = kind;
  }
  hintEl.lastChild.textContent = text;
  const w = hintEl.offsetWidth, hh = hintEl.offsetHeight;
  const left = Math.min(innerWidth - w - 4, x + 14);
  const top = Math.min(innerHeight - hh - 4, y + 22);
  hintEl.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
}

export function hideHint() { hintEl?.remove(); }

/** Small custom drag image: icon (or thumbnail) with a count badge, like Windows. */
export function dragImage(e, iconEl, count) {
  try {
    const ghost = h('div', { class: 'xp-dragghost' }, iconEl, count > 1 ? h('span', { class: 'xp-dragghost__badge' }, String(count)) : null);
    ghost.dataset.theme = document.documentElement.dataset.theme;
    document.body.append(ghost);
    e.dataTransfer.setDragImage(ghost, 40, 40);
    setTimeout(() => ghost.remove(), 0);
  } catch { /* the default drag image is fine */ }
}

addEventListener('dragend', () => endDrag(), true);
addEventListener('drop', () => setTimeout(hideHint, 0), true);
