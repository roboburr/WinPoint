// Taakbeheer — Sunburst chart (SVG) for the disk usage explorer.
//
//   const sb = Sunburst({ rings: 3, onDrill(node), onUp(), onHover(node|null), onPick(node) });
//   sb.setRoot(node, { direction: 'in'|'out'|null })   sb.highlight(node|null)
//
// Rings show up to `rings` levels below the current folder; each top-level child gets
// its own hue, deeper levels are lighter tints of it. Hover = tooltip + highlight of the
// arc and its ancestors; click a folder = drill in; click the centre = one level up.

import { h, clear, animate } from '../../../core/dom.js';
import { bytes, number } from '../../../core/format.js';
import { makeT } from '../../../core/i18n.js';
import { RING_COLORS } from '../util.js';

const NS = 'http://www.w3.org/2000/svg';
const TAU = Math.PI * 2;
const MIN_SWEEP = 0.006; // radians (~0.35°)

const t = makeT({
  nl: { files: { one: '{n} bestand', other: '{n} bestanden' }, ofParent: '{p} van {name}', up: 'Klik om een niveau omhoog te gaan', other: 'Overige', drill: 'Klik om te openen' },
  en: { files: { one: '{n} file', other: '{n} files' }, ofParent: '{p} of {name}', up: 'Click to go up one level', other: 'Other', drill: 'Click to open' },
});

export const filesText = (n) => t('files', { n: number(n), count: n });

function svgEl(tag, attrs = {}) {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) if (v != null) e.setAttribute(k, v);
  return e;
}

function arcPath(cx, cy, r0, r1, a0, a1) {
  const sweep = a1 - a0;
  const f = (n) => n.toFixed(2);
  if (sweep >= TAU - 1e-4) {
    // full ring: two half circles
    const m = a0 + Math.PI;
    return `M${f(cx + r1 * Math.cos(a0))} ${f(cy + r1 * Math.sin(a0))}`
      + `A${r1} ${r1} 0 1 1 ${f(cx + r1 * Math.cos(m))} ${f(cy + r1 * Math.sin(m))}`
      + `A${r1} ${r1} 0 1 1 ${f(cx + r1 * Math.cos(a0))} ${f(cy + r1 * Math.sin(a0))}`
      + `M${f(cx + r0 * Math.cos(a0))} ${f(cy + r0 * Math.sin(a0))}`
      + `A${r0} ${r0} 0 1 0 ${f(cx + r0 * Math.cos(m))} ${f(cy + r0 * Math.sin(m))}`
      + `A${r0} ${r0} 0 1 0 ${f(cx + r0 * Math.cos(a0))} ${f(cy + r0 * Math.sin(a0))}Z`;
  }
  const large = sweep > Math.PI ? 1 : 0;
  return `M${f(cx + r1 * Math.cos(a0))} ${f(cy + r1 * Math.sin(a0))}`
    + `A${r1} ${r1} 0 ${large} 1 ${f(cx + r1 * Math.cos(a1))} ${f(cy + r1 * Math.sin(a1))}`
    + `L${f(cx + r0 * Math.cos(a1))} ${f(cy + r0 * Math.sin(a1))}`
    + `A${r0} ${r0} 0 ${large} 0 ${f(cx + r0 * Math.cos(a0))} ${f(cy + r0 * Math.sin(a0))}Z`;
}

/** Colour for an arc: top-level hue, lighter with depth; "Overige" is neutral. */
export function arcColor(topIndex, depth, node) {
  if (node?.other) return 'var(--tm-cat-other)';
  const base = RING_COLORS[topIndex % RING_COLORS.length];
  if (depth <= 1) return base;
  return `color-mix(in srgb, ${base} var(--tm-sb-d${Math.min(4, depth)}), var(--tm-sb-mix))`;
}

export function Sunburst({ rings = 3, size = 440, onDrill, onUp, onHover, onPick } = {}) {
  const VB = size;
  const cx = VB / 2, cy = VB / 2;
  const R = VB / 2 - 2;
  const r0 = R * 0.34;
  const ringW = (R - r0) / rings;

  const svg = svgEl('svg', { viewBox: `0 0 ${VB} ${VB}`, class: 'tm-sb__svg', role: 'img' });
  const g = svgEl('g', { class: 'tm-sb__arcs' });
  const hole = svgEl('circle', { cx, cy, r: r0 - 3, class: 'tm-sb__hole' });
  svg.append(g, hole);
  const cName = h('div', { class: 'tm-sb__name' });
  const cSize = h('div', { class: 'tm-sb__size' });
  const cMeta = h('div', { class: 'tm-sb__meta' });
  const upHint = h('div', { class: 'tm-sb__up' });
  const center = h('button', { type: 'button', class: 'tm-sb__center', 'aria-label': t('up') }, upHint, cName, cSize, cMeta);
  const tip = h('div', { class: 'tm-sb__tip', role: 'tooltip', hidden: true });
  const root = h('div', { class: 'tm-sb', style: { '--tm-sb-hole': `${((r0 - 3) * 2 / VB) * 100}%` } }, svg, center, tip);

  let current = null;
  let currentLabel = '';
  let arcs = [];   // {path, node, top, depth, parentIdx}
  let canUp = false;
  let hot = null;

  function build(node) {
    clear(g);
    arcs = [];
    const walk = (n, a0, a1, depth, top, parentIdx) => {
      const kids = (n.children || []).filter((c) => c.size > 0);
      let a = a0;
      const span = a1 - a0;
      const sum = Math.max(1, n.size || kids.reduce((s, c) => s + c.size, 0));
      kids.forEach((c, i) => {
        const sweep = (c.size / sum) * span;
        const b0 = a, b1 = a + sweep;
        a = b1;
        if (sweep < (depth === 1 ? MIN_SWEEP : MIN_SWEEP * 2)) return;
        const topIdx = depth === 1 ? i : top;
        const ri = r0 + (depth - 1) * ringW + (depth === 1 ? 0 : 1);
        const ro = r0 + depth * ringW;
        const pad = Math.min(0.004, sweep / 6);
        const path = svgEl('path', {
          d: arcPath(cx, cy, ri, ro, b0 + pad / 2, b1 - pad / 2),
          class: ['tm-sb__arc', c.other && 'is-other', !c.isDir && 'is-file'].filter(Boolean).join(' '),
        });
        path.style.fill = arcColor(topIdx, depth, c);
        const idx = arcs.length;
        path.dataset.idx = String(idx);
        arcs.push({ path, node: c, top: topIdx, depth, parentIdx });
        g.append(path);
        if (depth < rings && c.children?.length) walk(c, b0, b1, depth + 1, topIdx, idx);
      });
    };
    walk(node, -Math.PI / 2, -Math.PI / 2 + TAU, 1, 0, -1);
    svg.setAttribute('aria-label', `${node.name}: ${bytes(node.size)}`);
  }

  function setCenter(node, label) {
    cName.textContent = label || node.name;
    cSize.textContent = bytes(node.size);
    cMeta.textContent = node.files != null ? filesText(node.files) : '';
    upHint.hidden = !canUp;
    center.disabled = !canUp;
    center.dataset.tooltip = canUp ? t('up') : '';
    if (!canUp) delete center.dataset.tooltip;
  }

  function setHot(idx) {
    const a = idx != null ? arcs[idx] : null;
    if (a === hot) return;
    for (const x of arcs) x.path.classList.remove('is-hot', 'is-trail');
    hot = a;
    root.classList.toggle('is-hovering', !!a);
    if (a) {
      a.path.classList.add('is-hot');
      let p = a.parentIdx;
      while (p >= 0) { arcs[p].path.classList.add('is-trail'); p = arcs[p].parentIdx; }
    }
    onHover?.(a ? a.node : null);
  }

  function showTip(a, ev) {
    if (!a) { tip.hidden = true; return; }
    const pct = current?.size ? (a.node.size / current.size) * 100 : 0;
    clear(tip).append(
      h('div', { class: 'tm-sb__tip-name' }, a.node.other ? t('other') : a.node.name),
      h('div', { class: 'tm-sb__tip-size' }, bytes(a.node.size)),
      h('div', { class: 'tm-sb__tip-meta' }, t('ofParent', { p: `${number(pct, pct < 10 ? 1 : 0)}%`, name: currentLabel || current?.name || '' })),
      a.node.isDir && a.node.files ? h('div', { class: 'tm-sb__tip-meta' }, filesText(a.node.files)) : null,
    );
    tip.hidden = false;
    const r = root.getBoundingClientRect();
    let x = ev.clientX - r.left + 14, y = ev.clientY - r.top + 14;
    const tw = tip.offsetWidth, th = tip.offsetHeight;
    if (x + tw > r.width) x = ev.clientX - r.left - tw - 10;
    if (y + th > r.height) y = ev.clientY - r.top - th - 10;
    tip.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
  }

  svg.addEventListener('pointermove', (e) => {
    const p = e.target.closest?.('.tm-sb__arc');
    const idx = p ? Number(p.dataset.idx) : null;
    setHot(idx);
    showTip(idx != null ? arcs[idx] : null, e);
  });
  svg.addEventListener('pointerleave', () => { setHot(null); tip.hidden = true; });
  svg.addEventListener('click', (e) => {
    const p = e.target.closest?.('.tm-sb__arc');
    if (!p) return;
    const a = arcs[Number(p.dataset.idx)];
    if (!a || a.node.other) return;
    tip.hidden = true;
    if (a.node.isDir) onDrill?.(a.node);
    else onPick?.(a.node);
  });
  center.addEventListener('click', () => { if (canUp) onUp?.(); });

  root.setRoot = (node, { direction = null, up = false, label = '' } = {}) => {
    current = node;
    currentLabel = label;
    canUp = !!up;
    hot = null;
    root.classList.remove('is-hovering');
    tip.hidden = true;
    build(node);
    setCenter(node, label);
    if (direction) {
      const from = direction === 'in' ? 0.82 : 1.12;
      animate(g, [{ transform: `scale(${from})`, opacity: 0.2 }, { transform: 'scale(1)', opacity: 1 }], { duration: 333, easing: 'cubic-bezier(0,0,0,1)' });
      animate(center, [{ opacity: 0 }, { opacity: 1 }], { duration: 250 });
    }
  };
  /** Highlight the arc for a node (from the side list), or clear. */
  root.highlight = (node) => {
    if (!node) { setHot(null); return; }
    const i = arcs.findIndex((a) => a.node === node);
    setHot(i >= 0 ? i : null);
  };
  root.colorOf = (node) => {
    const a = arcs.find((x) => x.node === node);
    return a ? a.path.style.fill : 'var(--tm-cat-other)';
  };
  return root;
}
