"""GE1a fix round: Schijven after a language switch that follows a disks.list within the same second.

Found by tests/gnome/scenarios/ge1a-switch.py (run 1 of the GE1a fix round, 2026-11-05 16:24): after en -> nl,
Schijven kept the host's English "Disk management doesn't work: UDisks isn't available on this computer." for the
whole 12 s settle window. The host answers disks.list from a snapshot of up to 1 s old that is not kept per language
(host/src/services/disks.rs snapshot()), and the page's reload on a switch used that cache, so the old-language text
stayed until the next 30 s poll. The fix (vendor/ui/js/apps/disks/main.js) reloads with refresh: true on a switch.

1. Schijven open in Dutch (the private session has no UDisks, so the page shows the host's "unavailable" text).
2. Host evidence (no check, recorded for R2-B): disks.list from the page, an atomic rename to English, at once
   disks.list again (still the cached old-language text?) and disks.list {refresh: true} (the new language).
3. The user's case, both directions: the page calls disks.list (the host cache is now < 1 s old), the file is renamed
   at once, and the page must show the host text in the new language within SETTLE s (before: up to 30 s).

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag ge1a-fix-disks -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --scenario tests/gnome/scenarios/ge1a-disks-race.py
Results: tests/gnome/scenarios/ge1a-disks-race.out/.
"""
import time

SETTLE = 3.0
HOST_TEXT = {'nl': 'UDisks is op deze computer niet beschikbaar', 'en': 'UDisks isn’t available on this computer'}   # round 2: R2-B's en body (’)
HEADING = {'nl': 'Schijfbeheer werkt niet', 'en': 'Disk Management doesn’t work'}
# the page's own bridge module (the same module instance the app uses: same URL)
LIST_JS = ("import(new URL('js/core/bridge.js', document.baseURI).href).then(m => m.bridge.call('disks.list', {args}))"
           ".then(r => ({{ok: true, error: r && r.error, supported: r && r.supported}}), e => ({{ok: false, error: e.message, code: e.code}}))")
TEXT_JS = "({lang: document.documentElement.lang, text: (document.body ? document.body.innerText : '').slice(0, 3000)})"


def host_list(h, refresh=False):
    return h.js('disks', LIST_JS.format(args='{refresh: true}' if refresh else '{}'), timeout=10)


def wait_text(h, lang):
    """Seconds until Schijven shows the host text and the heading in ``lang`` (and not the other language's)."""
    other = 'en' if lang == 'nl' else 'nl'
    t0, last = time.monotonic(), None
    while time.monotonic() - t0 < SETTLE:
        try:
            last = h.js('disks', TEXT_JS, timeout=4)
            text = last.get('text', '')
            if last.get('lang') == lang and HOST_TEXT[lang] in text and HEADING[lang] in text \
                    and HOST_TEXT[other] not in text and HEADING[other] not in text:
                return round(time.monotonic() - t0, 2), last
        except Exception:   # noqa: BLE001 - keep polling
            pass
        time.sleep(0.1)
    return None, last


def run(h):
    h.require('session starts in Dutch', h.lang == 'nl', lang=h.lang)
    h.open('disks')
    seconds, value = wait_text(h, 'nl')
    h.require('Schijven shows the host text in Dutch', seconds is not None, text=(value or {}).get('text', '')[:600])

    # 2. Host evidence: is the 1 s snapshot kept per language?
    before = host_list(h)
    h.hm.write_settings({'language': 'en'})
    h.s.lang = 'en'
    cached = host_list(h)
    fresh = host_list(h, refresh=True)
    h.record('host.disksListAroundSwitch', {'before': before, 'rightAfter': cached, 'refresh': fresh,
                                            'cachedKeepsOldLanguage': HOST_TEXT['nl'] in str(cached.get('error'))})
    seconds, value = wait_text(h, 'en')
    h.require('Schijven reaches English after the evidence step', seconds is not None,
              text=(value or {}).get('text', '')[:600])
    h.sleep(1.2)                       # the host snapshot is now older than 1 s

    # 3. The user's case: a load within the second before the switch, both directions.
    for target in ('nl', 'en', 'nl'):
        primed = host_list(h)             # fills the host's 1 s snapshot in the current language
        h.hm.write_settings({'language': target})
        h.s.lang = target
        seconds, value = wait_text(h, target)
        h.check(f'Schijven shows the host text in {target} within {SETTLE} s although disks.list ran just before '
                f'the switch', seconds is not None, seconds=seconds, primedError=primed.get('error'),
                text=(value or {}).get('text', '')[:600])
        h.screenshot(f'disks-{target}', surface='disks', settle=0.2)
        h.sleep(1.2)

    failed = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.require('every check passed', not failed, failed=failed)
