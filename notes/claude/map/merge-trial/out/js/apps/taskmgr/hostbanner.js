// Taakbeheer — "Win11 Shell reageert niet": an inline banner above every page while the host does not answer
// (stuck or restarting). The page below keeps showing the last data it had — or live data, while Taakbeheer's own
// backend (the sysmon worker) still answers directly (hostcall.js "direct routes"); the button restarts the shell
// through the app window (hostcall.js), so it also works when the host itself is stuck. When the host answers
// again the banner goes away by itself and every page refreshes (store.js).

import { h } from '../../core/dom.js';
import { InfoBar, Button, ProgressRing, Toast } from '../../core/components.js';
import { makeT, onLangChange } from '../../core/i18n.js';
import { time } from '../../core/format.js';
import { health, direct } from './hostcall.js';
import { restartShell } from './actions.js';

const t = makeT({
  nl: {
    downTitle: 'Win11 Shell reageert niet',
    downData: 'Taakbeheer blijft het proberen. Wat je hieronder ziet, is van {time}.',
    downNoData: 'Taakbeheer blijft het proberen en maakt vanzelf weer verbinding.',
    downLive: 'Taakbeheer werkt gewoon door: de processen en prestaties blijven bijgewerkt en je kunt taken beëindigen.',
    gaveUp: 'Win11 Shell is nog steeds niet terug. Probeer het nog een keer.',
    restart: 'Win11 Shell opnieuw starten',
    restartingTitle: 'Win11 Shell wordt opnieuw gestart…',
    restartingMsg: 'Even geduld. Taakbeheer maakt vanzelf weer verbinding.',
    back: 'Win11 Shell reageert weer', restarted: 'Win11 Shell is opnieuw gestart',
  },
  en: {
    downTitle: 'Win11 Shell is not responding',
    downData: 'Task Manager keeps trying. What you see below is from {time}.',
    downNoData: 'Task Manager keeps trying and reconnects by itself.',
    downLive: 'Task Manager keeps working: processes and performance stay up to date and you can end tasks.',
    gaveUp: 'Win11 Shell is still not back. Please try once more.',
    restart: 'Restart Win11 Shell',
    restartingTitle: 'Restarting Win11 Shell…',
    restartingMsg: 'Please wait. Task Manager reconnects by itself.',
    back: 'Win11 Shell is responding again', restarted: 'Win11 Shell has restarted',
  },
});

export function createHostBanner(ctx) {
  const btn = Button({ label: t('restart'), icon: 'restart', variant: 'accent', onClick: () => restartShell(ctx) });
  const ring = h('span', { class: 'tm-hostbar__busy' }, ProgressRing({ size: 20, indeterminate: true }));
  const bar = InfoBar({ severity: 'error', closable: false, open: false });
  bar.classList.add('tm-hostbar');
  bar.setAttribute('aria-live', 'assertive');
  let gaveUp = false;
  let shown = '';

  function render() {
    const st = health.state;
    ctx.frame?.el.classList.toggle('tm-host-down', st !== 'ok');
    if (st === 'ok') { bar.hidden = true; shown = ''; return; }
    const key = `${st}:${gaveUp}`;
    if (st === 'restarting') {
      if (shown !== key) { bar.setSeverity('info'); bar.setAction(ring); }
      bar.setTitle(t('restartingTitle'));
      bar.setMessage(t('restartingMsg'));
    } else {
      if (shown !== key) { bar.setSeverity('error'); bar.setAction(btn); }
      bar.setTitle(t('downTitle'));
      const at = health.lastOkAt;
      bar.setMessage(gaveUp ? t('gaveUp') : direct.anyLive ? t('downLive')
        : at ? t('downData', { time: time(at, { seconds: true }) }) : t('downNoData'));
      btn.setLabel(t('restart'));
    }
    shown = key;
    bar.hidden = false;
  }

  // while the host is down the message follows whether Taakbeheer's own backend still answers (direct routes)
  let refresh = 0;
  health.onChange((st, info) => {
    if (st === 'down') gaveUp = !!info.gaveUp;
    if (st === 'ok') gaveUp = false;
    clearInterval(refresh);
    refresh = st === 'down' ? setInterval(render, 3000) : 0;
    render();
    if (st === 'ok' && info.prev && info.prev !== 'ok' && !info.cancelled) {
      Toast.show({ title: info.restarted ? t('restarted') : t('back'), severity: 'success', timeout: 3500 });
    }
  });
  onLangChange(render);
  render();

  return {
    el: bar,
    render,
    /** Put the banner on top of the page content (after frame.setContent cleared it). */
    place(frame) { if (bar.parentNode !== frame.body || frame.body.firstChild !== bar) frame.body.prepend(bar); },
  };
}
