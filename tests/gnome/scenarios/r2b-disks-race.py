"""R2-B: Schijven after a language switch that follows a disks.list within the same second, on R2-B's host.

A copy of GE1a's ge1a-disks-race.py (removed with WebKit; git keeps it) for R2-B's two Disks changes:
* the host's 1 s disks.list snapshot is now kept per language (host/src/services/disks.rs snapshot()), so the
  host evidence of step 2 is a check here: a disks.list right after the switch is already in the new language
  (GE1a recorded cachedKeepsOldLanguage: true with the 3b898e6 and r1 hosts);
* the English body under the heading "Disk Management doesn’t work" no longer repeats the heading: it is
  "UDisks isn’t available on this computer." (row disks.unavailableBody, typographic apostrophe, D-G23). GE1a's
  copy looks for the 3b898e6 wording "UDisks isn't available on this computer" (ASCII apostrophe), so its English
  checks cannot pass on this host; this copy looks for the new text and checks that the heading is not repeated.
Dutch is unchanged ("Schijfbeheer werkt niet: UDisks is op deze computer niet beschikbaar.").
13-11: Schijven is the native app (win11-gtk-disks): its text and language come from its dev state
(native/kit/src/dev.rs; the language from its close button, harness app_lang), and disks.list goes to its own
backend (`win11-app --app disks --http-only`, h.backend) as the app's calls do.

1. Schijven open in the start language (the private session has no UDisks, so the app shows the host's
   "unavailable" text).
2. Host: disks.list from the app's backend, an atomic rename to the other language, at once disks.list again (must
   be in that language now) and disks.list {refresh: true}.
3. The user's case, both directions: disks.list on the app's backend (the host cache is now < 1 s old), the file is
   renamed at once, and the app must show the host text in the new language within SETTLE s.

Run (from the worktree, with R2-B's build):
    tools/heavy.sh --min-mb 2560 --tag r2b-disks-race -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --scenario tests/gnome/scenarios/r2b-disks-race.py \\
        --out tests/gnome/scenarios/r2b-disks-race.out
"""
import time

SETTLE = 3.0
HOST_TEXT = {'nl': 'Schijfbeheer werkt niet: UDisks is op deze computer niet beschikbaar.',
             'en': 'UDisks isn’t available on this computer.'}
OLD_EN = "UDisks isn't available on this computer"          # 3b898e6 / r1 wording of the English body
HEADING = {'nl': 'Schijfbeheer werkt niet', 'en': 'Disk Management doesn’t work'}


def host_list(h, refresh=False):
    """disks.list on Schijven's own backend: {ok, error (the host text), supported}, or {ok: false, error}."""
    try:
        r = h.backend('disks', 'disks.list', {'refresh': True} if refresh else {}, timeout=10) or {}
    except Exception as error:   # noqa: BLE001 - recorded as the answer
        return {'ok': False, 'error': f'{type(error).__name__}: {error}'}
    if isinstance(r.get('error'), dict):
        return {'ok': False, 'error': r['error'].get('message'), 'code': r['error'].get('code')}
    return {'ok': True, 'error': r.get('error'), 'supported': r.get('supported')}


def wait_text(h, lang):
    """Seconds until Schijven shows the host text and the heading in ``lang`` (and not the other language's)."""
    other = 'en' if lang == 'nl' else 'nl'
    t0, last = time.monotonic(), None
    while time.monotonic() - t0 < SETTLE:
        try:
            last = h.text('disks')
            text = last.get('text', '')
            if last.get('lang') == lang and HOST_TEXT[lang] in text and HEADING[lang] in text \
                    and HOST_TEXT[other] not in text and HEADING[other] not in text:
                return round(time.monotonic() - t0, 2), last
        except Exception:   # noqa: BLE001 - keep polling
            pass
        time.sleep(0.1)
    return None, last


def run(h):
    start = h.lang
    other = 'en' if start == 'nl' else 'nl'
    h.open('disks')
    seconds, value = wait_text(h, start)
    h.require(f'Schijven shows the host text in {start}', seconds is not None, text=(value or {}).get('text', '')[:600])

    # 2. Host evidence: is the 1 s snapshot kept per language?
    before = host_list(h)
    h.hm.write_settings({'language': other})
    h.s.lang = other
    cached = host_list(h)
    fresh = host_list(h, refresh=True)
    stale = HOST_TEXT[start] in str(cached.get('error'))
    h.record('host.disksListAroundSwitch', {'before': before, 'rightAfter': cached, 'refresh': fresh,
                                            'cachedKeepsOldLanguage': stale})
    h.check(f'host: disks.list right after the switch is already {other} (the 1 s snapshot is kept per language)',
            not stale and cached.get('error') == HOST_TEXT[other], rightAfter=cached)
    english = fresh if other == 'en' else before
    h.check('host: the English body does not repeat the heading and uses ’',
            english.get('error') == HOST_TEXT['en'] and 'Disk Management' not in str(english.get('error'))
            and OLD_EN not in str(english.get('error')), english=english)
    seconds, value = wait_text(h, other)
    h.require(f'Schijven reaches {other} after the evidence step', seconds is not None,
              text=(value or {}).get('text', '')[:600])
    h.sleep(1.2)                       # the host snapshot is now older than 1 s

    # 3. The user's case: a load within the second before the switch, both directions.
    for target in (start, other, start):
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
