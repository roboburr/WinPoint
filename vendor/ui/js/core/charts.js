// Win11 Shell — charts (owned by ui-core). Canvas, devicePixelRatio- and theme-aware.
//
//   const cpu = LineChart(canvas, {series: [values…], max: 100, color: CHART_COLORS.cpu})
//   cpu.push(42)                         // append one sample (streaming, newest at the right)
//   cpu.update([{values, color, dashed, fill}, …])
//   Sparkline(canvas, {values, color})   Donut({segments:[{value, color, label}], label, sublabel})
//   StackedBar({segments:[{value, color, label}], total, legend:true, format})
//
// Colours may be CSS colours or var(--token); they are resolved on the element itself,
// so charts inside a scoped [data-theme] container use that theme. Charts redraw
// automatically when the theme or accent changes.

import { h, clear } from './dom.js';

export const CHART_COLORS = {
  cpu: 'var(--chart-cpu)',
  memory: 'var(--chart-memory)',
  disk: 'var(--chart-disk)',
  network: 'var(--chart-network)',
  gpu: 'var(--chart-gpu)',
  accent: 'var(--accent)',
};
export const PALETTE = ['var(--chart-1)', 'var(--chart-2)', 'var(--chart-3)', 'var(--chart-4)',
  'var(--chart-5)', 'var(--chart-6)', 'var(--chart-7)', 'var(--chart-8)'];

/* ---------------------------------------------------------------- colours */

let probe = null;
function probeCtx() {
  if (!probe) probe = document.createElement('canvas').getContext('2d');
  return probe;
}

/** Resolve 'var(--x)' / '--x' / any CSS colour to a concrete colour string for el. */
export function resolveColor(el, color, fallback = '#0078d4') {
  if (!color) return fallback;
  let c = String(color).trim();
  const m = c.match(/^var\((--[\w-]+)(?:\s*,\s*(.+))?\)$/) || (c.startsWith('--') ? [null, c] : null);
  if (m) {
    c = getComputedStyle(el || document.documentElement).getPropertyValue(m[1]).trim() || m[2] || fallback;
    if (c.startsWith('var(')) return resolveColor(document.documentElement, c, fallback);
  }
  return c;
}

/** withAlpha('#0078d4', .2) → 'rgba(0,120,212,0.2)' (works for any canvas-parsable colour). */
export function withAlpha(color, alpha) {
  const ctx = probeCtx();
  ctx.fillStyle = '#000';
  ctx.fillStyle = color;
  const v = ctx.fillStyle;
  let r = 0, g = 0, b = 0, a = 1;
  if (v.startsWith('#')) {
    const n = parseInt(v.slice(1), 16);
    r = (n >> 16) & 255; g = (n >> 8) & 255; b = n & 255;
  } else {
    const p = v.match(/[\d.]+/g) || [];
    [r, g, b] = p.map(Number);
    a = p[3] != null ? Number(p[3]) : 1;
  }
  return `rgba(${r},${g},${b},${+(a * alpha).toFixed(4)})`;
}

/* ---------------------------------------------------------- redraw on theme */

const live = new Set();
let themeObserver = null;
function watchTheme(chart) {
  live.add(chart);
  if (!themeObserver) {
    themeObserver = new MutationObserver(() => {
      for (const c of [...live]) {
        if (c.canvas.isConnected) c.redraw();
      }
    });
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'style'] });
  }
}

function setupCanvas(canvas, draw) {
  const ro = 'ResizeObserver' in window ? new ResizeObserver(() => draw()) : null;
  ro?.observe(canvas);
  return () => ro?.disconnect();
}

function sizeCanvas(canvas) {
  const dpr = Math.min(3, window.devicePixelRatio || 1);
  const w = canvas.clientWidth, hh = canvas.clientHeight;
  if (!w || !hh) return null;
  const bw = Math.round(w * dpr), bh = Math.round(hh * dpr);
  if (canvas.width !== bw || canvas.height !== bh) { canvas.width = bw; canvas.height = bh; }
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, hh);
  return { ctx, w, h: hh, dpr };
}

function ensureCanvas(canvas, opts) {
  if (canvas instanceof HTMLCanvasElement) return [canvas, opts || {}];
  return [h('canvas', { class: 'fl-chart-canvas' }), canvas || {}];
}

/* ---------------------------------------------------------------- LineChart */

/**
 * LineChart(canvas, {series, max=100, min=0, fill=true, gridlines=true, color, points=60,
 *                    border=true, fillAlpha=.12, gridRows='auto'|n, lineWidth=1}) → chart
 * series: number[] (one series) or [{values:number[], color?, dashed?, fill?}]
 */
export function LineChart(canvasArg, optsArg) {
  const [canvas, opts] = ensureCanvas(canvasArg, optsArg);
  let { series = [], max = 100, min = 0, fill = true, gridlines = true, color = CHART_COLORS.cpu,
    points = 60, border = true, fillAlpha = 0.12, gridRows = 'auto', lineWidth = 1, scrollGrid = true } = opts;
  let tick = 0;
  canvas.classList.add('fl-chart-canvas');

  const norm = (s) => {
    if (!Array.isArray(s)) return [];
    if (!s.length || typeof s[0] === 'number' || s[0] == null) return [{ values: s }];
    return s;
  };
  let data = norm(series);

  function draw() {
    const sz = sizeCanvas(canvas);
    if (!sz) return;
    const { ctx, w, h: H } = sz;
    const base = resolveColor(canvas, color);
    const range = Math.max(1e-9, max - min);
    const stepX = w / Math.max(1, points - 1);

    if (gridlines) {
      ctx.strokeStyle = withAlpha(base, 0.16);
      ctx.lineWidth = 1;
      ctx.beginPath();
      const rows = gridRows === 'auto' ? Math.max(2, Math.min(10, Math.round(H / 26))) : gridRows;
      const cell = H / rows;
      for (let i = 1; i < rows; i++) {
        const y = Math.round(i * cell) + 0.5;
        ctx.moveTo(0, y); ctx.lineTo(w, y);
      }
      const offset = scrollGrid ? (tick * stepX) % cell : 0;
      for (let x = w - offset; x > 0; x -= cell) {
        const xx = Math.round(x) + 0.5;
        ctx.moveTo(xx, 0); ctx.lineTo(xx, H);
      }
      ctx.stroke();
    }

    for (const s of data) {
      const vals = (s.values || []).slice(-points);
      if (!vals.length) continue;
      const c = resolveColor(canvas, s.color || color);
      const x0 = w - (vals.length - 1) * stepX;
      const y = (v) => H - ((Math.min(max, Math.max(min, Number(v) || 0)) - min) / range) * (H - 1) - 0.5;
      ctx.beginPath();
      vals.forEach((v, i) => {
        const px = x0 + i * stepX, py = y(v);
        if (i === 0) ctx.moveTo(px, py); else ctx.lineTo(px, py);
      });
      if ((s.fill ?? fill) && !s.dashed) {
        ctx.save();
        ctx.lineTo(w, H);
        ctx.lineTo(x0, H);
        ctx.closePath();
        ctx.fillStyle = withAlpha(c, fillAlpha);
        ctx.fill();
        ctx.restore();
        ctx.beginPath();
        vals.forEach((v, i) => {
          const px = x0 + i * stepX, py = y(v);
          if (i === 0) ctx.moveTo(px, py); else ctx.lineTo(px, py);
        });
      }
      ctx.setLineDash(s.dashed ? [4, 3] : []);
      ctx.strokeStyle = c;
      ctx.lineWidth = s.lineWidth || lineWidth;
      ctx.lineJoin = 'round';
      ctx.stroke();
      ctx.setLineDash([]);
    }

    if (border) {
      ctx.strokeStyle = base;
      ctx.lineWidth = 1;
      ctx.strokeRect(0.5, 0.5, w - 1, H - 1);
    }
  }

  const unobserve = setupCanvas(canvas, draw);
  const chart = {
    canvas,
    redraw: draw,
    update(s) { data = norm(s); draw(); },
    /** push(value) for a single series, or push([v1, v2…]) for several */
    push(v) {
      const vals = Array.isArray(v) ? v : [v];
      if (!data.length) data = vals.map(() => ({ values: [] }));
      vals.forEach((x, i) => {
        if (!data[i]) data[i] = { values: [] };
        data[i].values.push(x);
        if (data[i].values.length > points * 2) data[i].values.splice(0, data[i].values.length - points);
      });
      tick++;
      draw();
    },
    setMax(m) { max = m; draw(); },
    setColor(c) { color = c; draw(); },
    setPoints(n) { points = n; draw(); },
    destroy() { unobserve(); live.delete(chart); },
  };
  watchTheme(chart);
  requestAnimationFrame(draw);
  return chart;
}

/* ---------------------------------------------------------------- Sparkline */

/** Sparkline(canvas?, {values, color, max, min=0, fill=true, lineWidth=1.25}) */
export function Sparkline(canvasArg, optsArg) {
  const [canvas, opts] = ensureCanvas(canvasArg, optsArg);
  let { values = [], color = 'var(--accent)', max = null, min = 0, fill = true, lineWidth = 1.25, points = 30 } = opts;
  canvas.classList.add('fl-chart-canvas');
  function draw() {
    const sz = sizeCanvas(canvas);
    if (!sz) return;
    const { ctx, w, h: H } = sz;
    const vals = values.slice(-points);
    if (vals.length < 2) return;
    const c = resolveColor(canvas, color);
    const top = max ?? Math.max(...vals, min + 1);
    const range = Math.max(1e-9, top - min);
    const stepX = (w - 2) / Math.max(1, points - 1);
    const x0 = w - 1 - (vals.length - 1) * stepX;
    const y = (v) => H - 1 - ((Math.min(top, Math.max(min, v)) - min) / range) * (H - 2);
    ctx.beginPath();
    vals.forEach((v, i) => (i ? ctx.lineTo(x0 + i * stepX, y(v)) : ctx.moveTo(x0, y(v))));
    if (fill) {
      ctx.save();
      ctx.lineTo(w - 1, H);
      ctx.lineTo(x0, H);
      ctx.closePath();
      ctx.fillStyle = withAlpha(c, 0.14);
      ctx.fill();
      ctx.restore();
      ctx.beginPath();
      vals.forEach((v, i) => (i ? ctx.lineTo(x0 + i * stepX, y(v)) : ctx.moveTo(x0, y(v))));
    }
    ctx.strokeStyle = c;
    ctx.lineWidth = lineWidth;
    ctx.lineJoin = 'round';
    ctx.stroke();
  }
  const unobserve = setupCanvas(canvas, draw);
  const chart = {
    canvas,
    redraw: draw,
    update(v) { values = v || []; draw(); },
    push(v) { values.push(v); if (values.length > points * 2) values.splice(0, values.length - points); draw(); },
    setColor(c) { color = c; draw(); },
    destroy() { unobserve(); live.delete(chart); },
  };
  watchTheme(chart);
  requestAnimationFrame(draw);
  return chart;
}

/* ---------------------------------------------------------------- Donut (SVG) */

/** Donut({segments:[{value, color, label}], size=120, thickness=12, gap=2, total, label, sublabel}) → <svg> with .update() */
export function Donut({ segments = [], size = 120, thickness = 12, gap = 2, total, label = '', sublabel = '' } = {}) {
  const NS = 'http://www.w3.org/2000/svg';
  const r = (size - thickness) / 2;
  const c = size / 2;
  const circ = 2 * Math.PI * r;
  const svg = h('svg', { class: 'fl-donut', viewBox: `0 0 ${size} ${size}`, width: size, height: size, role: 'img' });
  const track = h('circle', { class: 'fl-donut__track', cx: c, cy: c, r, fill: 'none', 'stroke-width': thickness });
  const segG = h('g', { transform: `rotate(-90 ${c} ${c})` });
  const t1 = h('text', { class: 'fl-donut__label', x: c, y: sublabel ? c - 2 : c + size * 0.06, 'text-anchor': 'middle', 'font-size': Math.round(size * 0.17) });
  const t2 = h('text', { class: 'fl-donut__sub', x: c, y: c + size * 0.13, 'text-anchor': 'middle', 'font-size': Math.max(10, Math.round(size * 0.095)) });
  svg.append(track, segG, t1, t2);

  function render(segs, opts = {}) {
    while (segG.firstChild) segG.removeChild(segG.firstChild);
    const sum = opts.total ?? total ?? segs.reduce((a, s) => a + Math.max(0, s.value || 0), 0);
    let offset = 0;
    const visible = segs.filter((s) => s.value > 0);
    for (const s of visible) {
      const len = sum > 0 ? (s.value / sum) * circ : 0;
      const g = visible.length > 1 ? Math.min(gap, len / 2) : 0;
      const seg = document.createElementNS(NS, 'circle');
      seg.setAttribute('class', 'fl-donut__seg');
      seg.setAttribute('cx', c); seg.setAttribute('cy', c); seg.setAttribute('r', r);
      seg.setAttribute('fill', 'none');
      seg.setAttribute('stroke-width', thickness);
      seg.style.stroke = s.color || 'var(--accent)';
      seg.style.strokeDasharray = `${Math.max(0, len - g)} ${circ}`;
      seg.style.strokeDashoffset = `${-offset}`;
      if (s.label) { const tt = document.createElementNS(NS, 'title'); tt.textContent = s.label; seg.appendChild(tt); }
      segG.appendChild(seg);
      offset += len;
    }
    t1.textContent = opts.label ?? label;
    t2.textContent = opts.sublabel ?? sublabel;
    t1.setAttribute('y', (opts.sublabel ?? sublabel) ? c - 2 : c + size * 0.06);
    svg.setAttribute('aria-label', [opts.label ?? label, opts.sublabel ?? sublabel].filter(Boolean).join(' '));
  }
  render(segments);
  svg.update = (segs, opts) => render(segs || segments, opts);
  return svg;
}

/* ---------------------------------------------------------------- StackedBar */

/** StackedBar({segments:[{value, color, label}], total, height=8, legend=false, format}) → element with .update() */
export function StackedBar({ segments = [], total, height = 8, legend = false, format = (v) => String(v) } = {}) {
  const bar = h('div', { class: 'fl-stacked__bar', role: 'img', style: { '--bar-h': `${height}px` } });
  const leg = legend ? h('div', { class: 'fl-stacked__legend' }) : null;
  const root = h('div', { class: 'fl-stacked' }, bar, leg);
  function render(segs, tot) {
    clear(bar);
    const sum = tot ?? total ?? segs.reduce((a, s) => a + Math.max(0, s.value || 0), 0);
    for (const s of segs) {
      if (!(s.value > 0)) continue;
      const pct = sum > 0 ? Math.min(100, (s.value / sum) * 100) : 0;
      bar.append(h('div', { class: 'fl-stacked__seg', title: s.label ? `${s.label}: ${format(s.value)}` : null, style: { width: `${pct}%`, background: s.color || 'var(--accent)' } }));
    }
    bar.setAttribute('aria-label', segs.map((s) => `${s.label || ''} ${format(s.value)}`).join(', '));
    if (leg) {
      clear(leg);
      for (const s of segs) {
        leg.append(h('span', { class: 'fl-stacked__key' },
          h('span', { class: 'fl-stacked__dot', style: { background: s.color || 'var(--accent)' } }),
          h('span', null, s.label || ''), h('span', { class: 'fl-legend-value' }, format(s.value))));
      }
    }
  }
  render(segments);
  root.update = (segs, tot) => { segments = segs; render(segs, tot); };
  return root;
}
