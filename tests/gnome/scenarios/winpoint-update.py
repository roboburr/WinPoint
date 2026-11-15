"""WinPoint's own updater against a fake GitHub (D-G74a, phase 1: the check, the card and the notification; D-G74,
phase 2: the install after a click).

Run (from G), in each language, with --winpoint-feed and --strace:
    tools/heavy.sh --min-mb 2560 --tag winpoint-update -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --winpoint-feed \\
        --scenario tests/gnome/scenarios/winpoint-update.py --out tests/gnome/scenarios/winpoint-update.out \\
        --strace tests/gnome/scenarios/winpoint-update.out/strace.txt
    (and --lang en --out tests/gnome/scenarios/winpoint-update.en.out --strace …/winpoint-update.en.out/strace.txt)

--winpoint-feed gives every process the updater's test seam (WIN11_WINPOINT_TEST=/tmp/w11-winpoint: origin
http://127.0.0.1:47811/, running = one below this tree's version) and starts tests/gnome/lib/fake_github.py there, in
mode "newer": this tree's version V as a real release (scripts/package.py's bundle of the run's build dir, with
GitHub's digest and its .sha256), a higher draft and a higher prerelease. The owner, win11-shell's backend, checks 5 s
after it starts. Checked, in order:

1. that automatic check asked the fake once, with WinPoint's User-Agent and GitHub's API version header;
2. the private session bus activates nothing, so nobody owned org.freedesktop.Notifications then (in a real session
   D-Bus starts GNOME's notification service, org.gnome.Shell.Notifications, which hands every Notify to the shell).
   The scenario starts that service itself, then restarts the owner (the taskbar) with no state.json: its automatic
   check comes again 5 s later, and exactly one notification "WinPoint V is available" comes; activating it (its
   default action, as a click does) brings Settings, opened on another page, to Update & recovery (shell.openApp
   with the token);
3. the card at the top of Update & recovery: V (not the draft or the prerelease), its release date, Release notes
   and Download and install (no download link and no how-to: WinPoint installs it itself) (screenshot
   winpoint-available). Then state.json forgets which version was announced (`notified` null): "Check for updates"
   finds the same release, records `notified` and sends no notification. Only the trigger rule (a manual check never
   notifies) stops it there;
4. the fake answers 500: "Check for updates" gives "WinPoint can’t check for updates" with GitHub's error, the raw
   detail in Technical details, the last known release below it (winpoint-failed);
5. a NetworkManager stand-in on the private system bus says connectivity none: the button refuses without a request
   and the card says there is no internet connection (winpoint-offline);
6. the newest release is the running version (and the fake answers slowly): the card shows the check running, and
   still does 2.5 s later, after the check's own start raised state.json's rev (which ends the card's own
   "pending" after the click) and a card poll, while the harness finds check.lock taken: only winpoint.status's
   `checking` (the owner's lock, seen from Settings' host) keeps it there (winpoint-checking); then "WinPoint is up
   to date" with the time of the check (winpoint-uptodate);
7. Advanced options has the daily check's toggle, on; off writes update.winpointAutoCheck false and the card then says
   automatic checks are off; on again;
8. the fake saw one request per check (the two automatic ones, then the four of the button; the offline refusal
   made none), every one with the User-Agent, and no notification came after the first;
9. D-G74, the install: V again ("Check for updates"), then a click on Download and install. Before each click on
   Download and install or Retry the scenario marks the session trace (/usr/bin/true w11-winpoint-click-<n>), and it
   records which installer runs each click causes (records["winpoint installer runs"]: one --check and one install for
   this one, none for the two later clicks, whose jobs end before --check): harness.py's exec rule fails the run when
   a WinPoint process runs any python3 other than the release bundle's installer (/usr/bin/python3 -E -s -B
   <RUN>/cache/win11-gnome/winpoint/work/V/bundle/scripts/install.py …, --check or the install, with its frozen
   environment), one that no click of its own explains, or when the runs per click are not those. The prefix's
   VERSION is set to the running version first, and a stray file put in the payload, so the install can be seen. The
   card shows the download with its progress and Cancel (the fake spreads the tarball over 8 s;
   winpoint-downloading), then the installer running (winpoint-installing), then "WinPoint V is installed" with
   Restart WinPoint (the run's installed extension is this tree's: no sign-out) and the "installed" notification
   (winpoint-installed). The run prefix's share/win11-gnome/VERSION says V again, in a new payload (another inode,
   the stray file gone), install.log and check.log have no error, the fake saw the 302 hop to its CDN and the whole
   tarball;
10. Restart WinPoint (shell.restartAll {reason:'update'}): Settings and the shell restart; the new owner runs V, so it
    clears the job (job.json and work/ go) and the card offers V again;
11. a damaged download (mode damaged: the digest and the .sha256 are the real tarball's, one byte of what arrives
    differs): Download and install ends in "WinPoint V wasn’t installed" with "The download is damaged and was
    deleted", nothing changed, Retry, and no tarball or .part left in work/ (winpoint-damaged);
12. a release without GitHub's digest (mode nodigest): Retry fetches the list again and refuses it (noDigest: no Retry
    then, a check instead, and the release's page, Download on GitHub and the how-to line the reason points at);
    "Check for updates" shows phase 1's card for it: the reason, Release notes, Download on GitHub and the how-to line,
    no install button (winpoint-nodigest).

A manual check within 30 s of the last one is skipped ('recent'), so the scenario waits that long between checks (a
job's own list request counts too).
"""
import fcntl
import json
import os
import select
import subprocess
import time
import urllib.request

TEXT = {
    'nl': {'available': 'WinPoint {v} is beschikbaar', 'cant': 'WinPoint kan niet controleren op updates',
           'status500': 'GitHub gaf een fout (500)', 'offline': 'Geen internetverbinding',
           'uptodate': 'WinPoint is bijgewerkt', 'checking': 'Zoeken naar een nieuwe versie van WinPoint…',
           'autoOff': 'Automatisch controleren staat uit', 'notes': 'Opmerkingen bij de release',
           'download': 'Downloaden op GitHub', 'version': 'Versie {v}', 'checked': 'Laatst gecontroleerd:',
           'have': 'Je hebt versie {v}', 'released': 'Uitgebracht op', 'details': 'Technische details',
           'notifyBody': 'Open Instellingen › Update en herstel om hem te installeren.',
           'toggle': 'Automatisch controleren op nieuwe versies van WinPoint', 'howto': './scripts/install.sh',
           'downloading': 'WinPoint {v} downloaden: ', 'checkingDownload': 'De download controleren…',
           'installing': 'WinPoint {v} installeren…', 'installed': 'WinPoint {v} is geïnstalleerd',
           'restartLine': 'Start WinPoint opnieuw om af te ronden.',
           'installedBody': 'Start WinPoint opnieuw om de update af te ronden.',
           'failed': 'WinPoint {v} is niet geïnstalleerd', 'damaged': 'De download is beschadigd en is verwijderd',
           'nothingChanged': 'Er is niets gewijzigd.',
           'noDigest': 'Deze uitgave heeft geen controlegetal van GitHub; download hem via de link'},
    'en': {'available': 'WinPoint {v} is available', 'cant': 'WinPoint can’t check for updates',
           'status500': 'GitHub returned an error (500)', 'offline': 'No internet connection',
           'uptodate': 'WinPoint is up to date', 'checking': 'Checking for a new version of WinPoint…',
           'autoOff': 'Automatic checks are off', 'notes': 'Release notes', 'download': 'Download on GitHub',
           'version': 'Version {v}', 'checked': 'Last checked:', 'have': 'You have version {v}',
           'released': 'Released on', 'details': 'Technical details',
           'notifyBody': 'Open Settings › Update & recovery to install it.',
           'toggle': 'Check for new versions of WinPoint automatically', 'howto': './scripts/install.sh',
           'downloading': 'Downloading WinPoint {v}: ', 'checkingDownload': 'Checking the download…',
           'installing': 'Installing WinPoint {v}…', 'installed': 'WinPoint {v} is installed',
           'restartLine': 'Restart WinPoint to finish.', 'installedBody': 'Restart WinPoint to finish the update.',
           'failed': 'WinPoint {v} wasn’t installed', 'damaged': 'The download is damaged and was deleted',
           'nothingChanged': 'Nothing was changed.',
           'noDigest': 'This release has no checksum from GitHub; download it from the link'},
}
# GNOME's message tray: WinPoint's notifications, by their title (GNOME may file them under the Settings app of the
# desktop-entry hint rather than under the app name "WinPoint"), with their source and actions
NOTES_JS = ("(()=>{try{return Main.messageTray.getSources().flatMap(s=>(s.notifications||[]).filter(n=>/^WinPoint /"
            ".test(n.title||'')).map(n=>({title:n.title,body:n.body,source:s.title,"
            "actions:(()=>{try{return (n.actions||[]).map(a=>a.label);}catch(e){return String(e);}})()})));}"
            "catch(e){return 'error: '+e;}})()")
ACTIVATE_JS = ("(()=>{const n=Main.messageTray.getSources().flatMap(s=>s.notifications||[]).find(n=>n.title===%s);"
               "if(!n)return false;n.activate();return true;})()")
# A NetworkManager stand-in for the offline step (the private system bus has none): org.freedesktop.NetworkManager
# answering Properties.Get/GetAll with Connectivity (argv[1]) and Metered 4 (guess no); every other call fails. It
# runs as a child of this harness on the private system bus (allow own="*") and is stopped after the step.
NM_MOCK = r"""
import sys, warnings
warnings.simplefilter('ignore', DeprecationWarning)
from gi.repository import Gio, GLib
NM = 'org.freedesktop.NetworkManager'
PROPS = {'Connectivity': GLib.Variant('u', int(sys.argv[1])), 'Metered': GLib.Variant('u', 4),
         'State': GLib.Variant('u', 20)}
XML = ('<node><interface name="org.freedesktop.DBus.Properties"><method name="Get"><arg type="s" direction="in"/>'
       '<arg type="s" direction="in"/><arg type="v" direction="out"/></method><method name="GetAll">'
       '<arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method></interface></node>')
def call(conn, sender, path, iface, method, params, invocation):
    args = params.unpack()
    if method == 'Get' and args[0] == NM and args[1] in PROPS:
        invocation.return_value(GLib.Variant('(v)', (PROPS[args[1]],)))
    elif method == 'GetAll' and args[0] == NM:
        invocation.return_value(GLib.Variant('(a{sv})', (PROPS,)))
    else:
        invocation.return_dbus_error('org.freedesktop.DBus.Error.UnknownProperty', 'test stand-in')
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
bus.register_object('/org/freedesktop/NetworkManager', Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0], call, None, None)
loop = GLib.MainLoop()
Gio.bus_own_name_on_connection(bus, NM, Gio.BusNameOwnerFlags.NONE, lambda *a: print('owned', flush=True),
                               lambda *a: loop.quit())
loop.run()
"""
RECENT_S = 31          # services/winpoint/schedule.rs RECENT_MS (30 s) and a second
# GNOME's notification service (/usr/share/dbus-1/services/org.gnome.Shell.Notifications.service)
NOTIFY_SERVICE = ['/usr/bin/gjs', '-m', '/usr/share/gnome-shell/org.gnome.Shell.Notifications']


def fake(h, path):
    with urllib.request.urlopen(f'http://127.0.0.1:{h.hm.WINPOINT_PORT}{path}', timeout=15) as reply:
        return json.loads(reply.read())


def requests_(h):
    return fake(h, '/_fake/requests')


def downloads(h):
    return fake(h, '/_fake/downloads')


def mode(h, name, delay=0, slow=0):
    query = '&'.join(f'{k}={v}' for k, v in (('delay', delay), ('slow', slow)) if v)
    value = fake(h, f'/_fake/mode/{name}' + (f'?{query}' if query else ''))
    h.record(f'fake mode {name}', value)
    return value


def mark_click(h, n):
    """D-G74: mark the session trace right before a click on Download and install (harness.py's exec rule: no python3
    of WinPoint before the first mark)."""
    subprocess.run(['/usr/bin/true', f'{h.hm.WINPOINT_CLICK}-{n}'], check=False)
    h.record('click mark', n)


def state_dir(h):
    return h.hm.RUN / 'cache' / 'win11-gnome' / 'winpoint'


def load_job(h):
    """job.json, with the machine's boot id in sessionKey masked: the checks dump the record into results.json,
    which goes into the public repository."""
    try:
        job = json.loads((state_dir(h) / 'job.json').read_text())
    except (OSError, ValueError):
        return None
    if isinstance(job, dict) and isinstance(job.get('sessionKey'), str):
        job['sessionKey'] = '<boot id>:' + job['sessionKey'].rpartition(':')[2]
    return job


def notes(h):
    value = h.eval(NOTES_JS)
    if not isinstance(value, list):
        h.log(f'notifications: {value!r}')
        return []
    return value


def card(h):
    """The card's texts: the title and every label under update-winpoint."""
    try:
        tops = h.state('settings')
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:     # Settings restarting (shell.restartAll)
        return {'shown': False, 'title': None, 'labels': [], 'names': [], 'error': str(error)}
    found = h.find('settings', 'update-winpoint', state=tops)
    if not found:
        return {'shown': False, 'title': None, 'labels': [], 'names': []}
    nodes = h.hm.tree_nodes([{'tree': found[0]}])
    return {'shown': True, 'title': next((n.get('label') for n in nodes if n.get('name') == 'update-winpoint-title'), None),
            'labels': [str(n['label']) for n in nodes if n.get('label') and n.get('visible', True)],
            'names': sorted({n['name'] for n in nodes if str(n.get('name') or '').startswith('update-winpoint')
                             and n.get('visible', True)})}


def wait_card(h, label, test, timeout=30):
    seen = {}

    def probe():
        seen.update(card(h))
        return test(seen)
    try:
        h.until(probe, label, timeout, 0.4)
        ok = True
    except TimeoutError:
        ok = False
    h.check(label, ok, card=dict(seen))
    return dict(seen)


def wait_recent(h):
    """Until a manual check is no longer 'recent' (30 s after the last request the fake saw)."""
    last = max((r['at'] for r in requests_(h)), default=0) / 1000
    wait = last + RECENT_S - time.time()
    if wait > 0:
        h.sleep(wait)


def click_check(h):
    return h.click('settings', 'update-winpoint-check')


def start_nm(h, connectivity):
    log = open(h.hm.RUN / 'logs' / 'nm-standin.log', 'w')
    proc = subprocess.Popen(['python3', '-I', '-c', NM_MOCK, str(connectivity)], stdout=subprocess.PIPE, stderr=log,
                            text=True)
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline and proc.poll() is None:
        if select.select([proc.stdout], [], [], 0.5)[0] and proc.stdout.readline().strip() == 'owned':
            h.record('nm.standin', {'pid': proc.pid, 'connectivity': connectivity})
            return proc
    h.check('NetworkManager stand-in owns its name', False, exit=proc.poll())
    proc.kill()
    return None


def load_state(h):
    path = h.hm.RUN / 'cache' / 'win11-gnome' / 'winpoint' / 'state.json'
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return {}


def lock_taken(h):
    """Whether someone holds the owner's check.lock (a non-blocking flock of our own, dropped at once)."""
    path = h.hm.RUN / 'cache' / 'win11-gnome' / 'winpoint' / 'check.lock'
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError:
        return False
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        return True
    finally:
        os.close(fd)
    return False


def has_owner(name):
    from gi.repository import Gio, GLib
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    reply = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'NameHasOwner',
                          GLib.Variant('(s)', (name,)), None, Gio.DBusCallFlags.NONE, 3000, None)
    return bool(reply.unpack()[0])


def run(h):
    tx = TEXT[h.lang]
    seam = h.hm.WINPOINT_SEAM
    h.require('the run has --winpoint-feed (the seam and the fake GitHub)', h.hm.WINPOINT_FEED and seam.is_dir(),
              seam=str(seam))
    running = json.loads((seam / 'seam.json').read_text())['running']
    info = fake(h, '/_fake/info')
    newer = info['offered']
    # the hosts' own version (crate::VERSION) is this tree's, the one the fake offers
    agent = f'WinPoint/{newer}'
    h.record('versions', {'running': running, 'offered': newer, 'fake': info})
    h.check('the fake offers this tree as a real release above the running version',
            info['running'] == running and info['tarball'] == f'winpoint-{newer}-linux-x86_64.tar.gz'
            and info['size'] > 1_000_000, info=info)

    # 1. the owner's automatic check, 5 s after it started
    try:
        h.until(lambda: requests_(h), 'the automatic check reached the fake GitHub', 90, 1)
    except TimeoutError:
        pass
    first = requests_(h)
    h.record('requests after start', first)
    h.check('the automatic check asked the fake GitHub once', len(first) == 1, requests=first)
    h.check("the request carries WinPoint's User-Agent and GitHub's API version",
            bool(first) and str(first[0]['userAgent']).startswith(agent + ' (+https://github.com/roboburr/WinPoint)')
            and first[0]['apiVersion'] == '2022-11-28' and first[0]['accept'] == 'application/vnd.github+json',
            request=first[:1])

    # 2. GNOME's notification service, then the owner again with no state: one notification for the newer version,
    # whose action opens Update & recovery
    h.record('notifications owned before', has_owner('org.freedesktop.Notifications'))
    h.spawn('gnome-shell-notifications', NOTIFY_SERVICE)
    try:
        h.until(lambda: has_owner('org.freedesktop.Notifications'), "GNOME's notification service", 20, 0.5)
    except TimeoutError:
        pass
    h.require("GNOME's notification service owns org.freedesktop.Notifications",
              has_owner('org.freedesktop.Notifications'))
    state = h.hm.RUN / 'cache' / 'win11-gnome' / 'winpoint' / 'state.json'
    h.kill_panel()
    state.unlink(missing_ok=True)
    h.restart_panel()
    try:
        h.until(lambda: len(requests_(h)) >= 2, 'the restarted owner checked again', 60, 1)
    except TimeoutError:
        pass
    h.check('the restarted owner, with no state, checked again on its own', len(requests_(h)) == 2,
            requests=len(requests_(h)))
    title = tx['available'].format(v=newer)
    try:
        h.until(lambda: any(n['title'] == title for n in notes(h)), 'the notification is in the message tray', 30, 0.5)
    except TimeoutError:
        pass
    shown = notes(h)
    h.record('notifications', shown)
    h.record('message tray', h.notifications())
    h.check('exactly one WinPoint notification, for the newer version',
            len(shown) == 1 and shown[0]['title'] == title and shown[0]['body'] == tx['notifyBody'], notifications=shown)
    h.screenshot('winpoint-notification')
    h.open('settings:system')
    h.record('activate the notification', h.eval(ACTIVATE_JS % json.dumps(title)))
    seen = wait_card(h, 'the notification brings Settings to Update & recovery (the card shows the newer version)',
                     lambda c: c['title'] == title, 30)
    if seen.get('title') != title:
        h.open('settings:update')
    h.surface('settings').page = 'update'        # where the notification's action took it
    h.check('the notification is gone after its action', not notes(h), notifications=notes(h))

    # 3. the card: the newer release, not the draft or the prerelease
    c = wait_card(h, 'the card offers the newer version', lambda c: c['title'] == title)
    facts = next((l for l in c['labels'] if l.startswith(tx['have'].format(v=running))), '')
    h.check('the card names the running version and the release date', tx['released'] in facts, facts=facts)
    h.check('the card links to the release notes and offers Download and install',
            {'update-winpoint-notes', 'update-winpoint-install'} <= set(c['names']) and tx['notes'] in c['labels'],
            card=c)
    h.check('no download link and no how-to: WinPoint installs it itself (D-G74)',
            'update-winpoint-download' not in c['names'] and not any(tx['howto'] in l for l in c['labels']), card=c)
    h.screenshot('winpoint-available', surface='settings')

    # 3b. a manual check that finds a version nobody announced yet: it records `notified` and sends nothing. state.json
    # is changed in place (its mode stays 0600); check.rs reads it fresh for every check
    st = load_state(h)
    h.check('the automatic check recorded the announced version', st.get('notified') == newer, notified=st.get('notified'))
    h.check('no job and no work folder before a click', load_job(h) is None and not (state_dir(h) / 'work').exists())
    st['notified'] = None
    state.write_text(json.dumps(st))
    h.check('state.json stays 0600 after the edit', state.stat().st_mode & 0o777 == 0o600, mode=oct(state.stat().st_mode))
    wait_recent(h)
    before = len(requests_(h))
    h.record('click check (not announced)', click_check(h))
    try:
        h.until(lambda: len(requests_(h)) > before and load_state(h).get('notified') == newer,
                'the manual check recorded the newer version', 20, 0.5)
    except TimeoutError:
        pass
    h.check('the manual check found the newer version and recorded notified', load_state(h).get('notified') == newer
            and len(requests_(h)) == before + 1, notified=load_state(h).get('notified'), requests=len(requests_(h)))
    wait_card(h, 'the card still offers the newer version', lambda c: c['title'] == title)
    h.sleep(3)
    h.check('a manual check of a version not announced yet sends no notification', not notes(h),
            notifications=notes(h))

    # 4. a failed check: GitHub answers 500
    mode(h, 'error')
    wait_recent(h)
    h.record('click check (500)', click_check(h))
    c = wait_card(h, 'a failed check says why', lambda c: c['title'] == tx['cant'] and tx['status500'] in c['labels'])
    h.check('the last known release stays below the failure', title in c['labels'], labels=c['labels'])
    h.check('the raw detail is in Technical details', 'update-winpoint-details' in c['names'], names=c['names'])
    try:
        h.click('settings', 'update-winpoint-details')
        h.sleep(0.6)
    except LookupError as error:
        h.log(f'details: {error}')
    h.screenshot('winpoint-failed', surface='settings')

    # 5. offline: NetworkManager says connectivity none; the button refuses without a request
    before = len(requests_(h))
    nm = start_nm(h, 1)
    wait_recent(h)
    h.record('click check (offline)', click_check(h))
    c = wait_card(h, 'offline: the card says there is no internet connection',
                  lambda c: c['title'] == tx['cant'] and tx['offline'] in c['labels'])
    h.check('the offline refusal made no request', len(requests_(h)) == before, requests=len(requests_(h)), before=before)
    h.screenshot('winpoint-offline', surface='settings')
    if nm:
        nm.terminate()
        nm.wait(timeout=10)

    # 6. up to date: the newest release is the running one; the fake answers slowly, so the check is seen running
    mode(h, 'current', delay=6)
    wait_recent(h)
    rev = load_state(h).get('rev', 0)
    h.record('click check (current)', click_check(h))
    c = wait_card(h, 'the card shows the check running', lambda c: c['title'] == tx['checking'], 5)
    # past the check's start (its rev ends the card's own "pending") and a card poll (1 s), well inside the fake's 6 s
    h.sleep(2.5)
    later = load_state(h).get('rev', 0)
    h.check("the check's start is in state.json (its rev is past the click's)", later > rev, before=rev, after=later)
    h.check("the owner holds check.lock while GitHub answers", lock_taken(h))
    c = card(h)
    h.check("winpoint.status keeps the card checking (the owner's lock, not the click)", c['title'] == tx['checking'],
            card=c)
    h.check('the ring replaces the button while it checks', 'update-winpoint-checking' in c['names']
            and 'update-winpoint-check' not in c['names'], names=c['names'])
    h.screenshot('winpoint-checking', surface='settings', settle=0.2)
    c = wait_card(h, 'the card says WinPoint is up to date', lambda c: c['title'] == tx['uptodate'], 20)
    facts = next((l for l in c['labels'] if l.startswith(tx['version'].format(v=running))), '')
    h.check('up to date names the version and the time of the check', tx['checked'] in facts, facts=facts)
    h.check('no release links when up to date', 'update-winpoint-notes' not in c['names'], names=c['names'])
    h.screenshot('winpoint-uptodate', surface='settings')
    mode(h, 'current')

    # 7. the toggle in Advanced options
    h.open('settings:update-advanced')
    h.wait_text('settings', tx['toggle'])
    sw = h.find('settings', 'advanced-winpoint')
    h.check('Advanced options has the daily check, on', bool(sw) and sw[0].get('active') is True, switch=sw[:1])
    h.screenshot('winpoint-toggle', surface='settings')
    path = h.hm.RUN / 'config' / 'win11-gnome' / 'settings.json'
    auto = lambda: json.loads(path.read_text() or '{}').get('update', {}).get('winpointAutoCheck')
    if sw:
        h.click('settings', 'advanced-winpoint')
        try:
            h.until(lambda: auto() is False, 'update.winpointAutoCheck false in settings.json', 10, 0.3)
        except TimeoutError:
            pass
        h.check('the toggle writes update.winpointAutoCheck false', auto() is False, value=auto())
        h.open('settings:update')
        wait_card(h, 'the card says automatic checks are off', lambda c: tx['autoOff'] in c['labels'], 15)
        h.open('settings:update-advanced')
        h.click('settings', 'advanced-winpoint')
        try:
            h.until(lambda: auto() is True, 'update.winpointAutoCheck true again', 10, 0.3)
        except TimeoutError:
            pass
        h.check('the toggle writes update.winpointAutoCheck true again', auto() is True, value=auto())

    # 8. one request per check, and no notification after the first
    final = requests_(h)
    h.record('requests', final)
    h.check('one request per check: two automatic, not announced, 500, current (none offline)',
            [r['status'] for r in final] == [200, 200, 200, 500, 200], statuses=[r['status'] for r in final])
    h.check('every request has the User-Agent', all(str(r['userAgent']).startswith(agent) for r in final))
    h.check('no notification after the first (every later check was manual)', not notes(h), notifications=notes(h))
    h.check('no download before a click', not downloads(h), downloads=downloads(h))

    install_flow(h, tx, newer)
    h.close('settings')


def install_flow(h, tx, newer):
    """Steps 9 to 12 (D-G74): Download and install, Restart WinPoint, a damaged download, a release without a digest."""
    title = tx['available'].format(v=newer)
    work = state_dir(h) / 'work' / newer
    tarball = f'winpoint-{newer}-linux-x86_64.tar.gz'

    # 9. the release again, then the click; the tarball takes about 8 s, so the download can be seen. The installer
    # runs each click causes (harness.py's exec rule compares them with the trace): click 1 installs, the jobs of
    # clicks 2 (damaged) and 3 (Retry without a digest) end before --check
    h.record('winpoint installer runs', {'1': ['check', 'install'], '2': [], '3': []})
    mode(h, 'newer', slow=8)
    h.open('settings:update')
    wait_recent(h)
    h.record('click check (newer)', click_check(h))
    c = wait_card(h, 'the card offers the release with Download and install',
                  lambda c: c['title'] == title and 'update-winpoint-install' in c['names'], 30)
    before = len(requests_(h))
    # the prefix says the running version, and the payload has a file no release has: after the install VERSION says
    # V again, in another payload directory (install.py swaps a new one in), without that file
    version = h.hm.RUN / 'prefix' / 'share' / 'win11-gnome' / 'VERSION'
    stray = version.with_name('.w11-preinstall-sentinel')
    version.write_text(h.hm.winpoint_running() + '\n')
    stray.write_text('not part of any release\n')
    payload_inode = version.parent.stat().st_ino
    h.record('before the install', {'version': version.read_text(), 'payloadInode': payload_inode})
    mark_click(h, 1)
    h.record('click Download and install', h.click('settings', 'update-winpoint-install'))
    c = wait_card(h, 'the card shows the download with its progress and Cancel',
                  lambda c: str(c['title'] or '').startswith(tx['downloading'].format(v=newer))
                  and {'update-winpoint-progress', 'update-winpoint-cancel'} <= set(c['names']), 30)
    # the progress events patch the card in place: wait for a share above 0 %
    downloading = tx['downloading'].format(v=newer)
    c = wait_card(h, 'the download moves on (the card patched in place)',
                  lambda c: str(c['title'] or '').startswith(downloading)
                  and str(c['title'])[len(downloading):].rstrip('%') not in ('', '0'), 20)
    h.record('download share', c.get('title'))
    h.screenshot('winpoint-downloading', surface='settings', settle=0.3)
    h.check('the job asked GitHub for the release list again first', len(requests_(h)) == before + 1,
            requests=len(requests_(h)), before=before)
    seen = set()

    def phase(c):
        seen.add(c['title'])
        return c['title'] == tx['installing'].format(v=newer)
    c = wait_card(h, 'the card shows the installer running (no Cancel)', phase, 180)
    h.screenshot('winpoint-installing', surface='settings', settle=0.2)
    h.check('no Cancel while it installs', 'update-winpoint-cancel' not in c['names'], names=c['names'])
    h.record('titles until installing', sorted(t for t in seen if t))
    c = wait_card(h, 'the card says the release is installed, with Restart WinPoint',
                  lambda c: c['title'] == tx['installed'].format(v=newer), 300)
    h.check('installed: restart WinPoint to finish (the extension did not change)',
            'update-winpoint-restart' in c['names'] and tx['restartLine'] in c['labels'], card=c)
    job = load_job(h) or {}
    h.record('job', {k: job.get(k) for k in ('state', 'version', 'restart', 'outcome', 'error', 'installer')})
    h.check('job.json: installed with restart', job.get('state') == 'installed' and job.get('restart') == 'restart'
            and job.get('error') is None, job={k: job.get(k) for k in ('state', 'restart', 'error', 'outcome')})
    h.check("the run prefix's VERSION is the release's (it said the running version before)",
            version.is_file() and version.read_text() == newer + '\n',
            version=version.read_text() if version.is_file() else None)
    h.check('the installer swapped in a new payload: another directory, without the stray file',
            version.parent.stat().st_ino != payload_inode and not stray.exists(),
            inode=version.parent.stat().st_ino, before=payload_inode, stray=stray.exists())
    installed_inode = version.parent.stat().st_ino
    for name in ('install.log', 'check.log'):
        text = (work / name).read_text(errors='replace') if (work / name).is_file() else None
        bad = [l for l in (text or '').splitlines() if any(w in l for w in ('Traceback', 'Error', 'error', 'FAIL'))]
        h.check(f'{name} is there and has no error', text is not None and not bad, lines=bad[:5],
                tail=(text or '')[-600:])
    h.check('the installed release left no tarball and no bundle in work/',
            not (work / tarball).exists() and not (work / 'bundle').exists(),
            left=sorted(p.name for p in work.iterdir()) if work.is_dir() else None)
    got = downloads(h)
    h.record('downloads', got)
    h.check('the fake saw the .sha256, the 302 hop to its CDN and the whole tarball',
            [d['status'] for d in got] == [200, 302, 200] and got[-1]['bytes'] == fake(h, '/_fake/info')['size']
            and got[0]['path'].endswith('.sha256') and got[2]['path'].startswith('/cdn/'), downloads=got)
    installed = tx['installed'].format(v=newer)
    try:
        h.until(lambda: any(n['title'] == installed for n in notes(h)), 'the installed notification', 20, 0.5)
    except TimeoutError:
        pass
    shown = [n for n in notes(h) if n['title'] == installed]
    h.check('one "installed" notification that says how to finish', len(shown) == 1
            and shown[0]['body'] == tx['installedBody'], notifications=notes(h))
    h.screenshot('winpoint-installed', surface='settings')

    # 10. Restart WinPoint: every WinPoint program restarts; the new owner runs V and clears the job
    rows = len(h.s.rows())
    h.record('click Restart WinPoint', h.click('settings', 'update-winpoint-restart'))
    boots = {}
    for kind, wait in (('settings', lambda: h.s.native_app_boot('settings', rows, 60)),
                       ('shell', lambda: h.s.native_boot(rows, 60))):
        try:
            boots[kind] = wait()
        except TimeoutError as error:
            boots[kind] = {'error': str(error)}
    h.record('restart boots', boots)
    h.check('Settings and the shell restarted', all('error' not in b for b in boots.values()), boots=boots)
    try:
        h.until(lambda: load_job(h) is None and not (state_dir(h) / 'work').exists(),
                'the new owner cleared the job', 60, 0.5)
    except TimeoutError:
        pass
    h.check('the restarted owner (version V) cleared job.json and work/', load_job(h) is None
            and not (state_dir(h) / 'work').exists(), job=load_job(h))
    wait_card(h, 'the card offers the release again (the hosts still pretend to run the lower version)',
              lambda c: c['title'] == title and 'update-winpoint-install' in c['names'], 60)

    # 11. a damaged download: the digest and the .sha256 are right, what arrives is not
    mode(h, 'damaged')
    got = len(downloads(h))
    mark_click(h, 2)
    h.record('click Download and install (damaged)', h.click('settings', 'update-winpoint-install'))
    c = wait_card(h, 'a damaged download ends the job: not installed, the download deleted',
                  lambda c: c['title'] == tx['failed'].format(v=newer) and tx['damaged'] in c['labels'], 120)
    h.check('nothing changed, and Retry is offered', tx['nothingChanged'] in c['labels']
            and 'update-winpoint-retry' in c['names'], card=c)
    job = load_job(h) or {}
    h.check('job.json: failed with checksum, nothing changed', job.get('state') == 'failed'
            and (job.get('error') or {}).get('code') == 'checksum' and job.get('outcome') == 'nothingChanged', job=job)
    left = sorted(p.name for p in work.iterdir()) if work.is_dir() else []
    h.check('the damaged tarball was deleted at once (no tarball, no .part, no bundle)',
            not any(n.endswith(('.tar.gz', '.part')) or n == 'bundle' for n in left), left=left)
    h.check('the damaged tarball was fetched whole', [d['status'] for d in downloads(h)[got:]] == [200, 302, 200],
            downloads=downloads(h)[got:])
    h.check('the version on disk is still the installed one, in the same payload', version.read_text() == newer + '\n'
            and version.parent.stat().st_ino == installed_inode)
    try:
        h.click('settings', 'update-winpoint-details')
        h.sleep(0.6)
    except LookupError as error:
        h.log(f'details: {error}')
    h.screenshot('winpoint-damaged', surface='settings')

    # 12. no digest from GitHub: Retry refuses it (with the links its text points at); a check then shows phase 1's
    # card for it
    mode(h, 'nodigest')
    mark_click(h, 3)
    h.record('click Retry (no digest)', h.click('settings', 'update-winpoint-retry'))
    c = wait_card(h, 'Retry with a release without a digest: refused, a check offered instead',
                  lambda c: c['title'] == tx['failed'].format(v=newer) and tx['noDigest'] in c['labels']
                  and 'update-winpoint-check' in c['names'], 60)
    h.check('the refusal says to download it from the link, and the link is there: Release notes, Download on '
            'GitHub and the how-to line', {'update-winpoint-notes', 'update-winpoint-download'} <= set(c['names'])
            and 'update-winpoint-retry' not in c['names']
            and any(tx['howto'] in l and f'winpoint-{newer}' in l for l in c['labels']), card=c)
    h.check('job.json: failed with noDigest', ((load_job(h) or {}).get('error') or {}).get('code') == 'noDigest',
            job=load_job(h))
    wait_recent(h)
    h.record('click check (no digest)', click_check(h))
    c = wait_card(h, "a release without a digest: phase 1's card", lambda c: c['title'] == title
                  and tx['noDigest'] in c['labels'], 30)
    h.check('Release notes, Download on GitHub and the how-to line; no install button',
            {'update-winpoint-notes', 'update-winpoint-download', 'update-winpoint-check'} <= set(c['names'])
            and 'update-winpoint-install' not in c['names']
            and any(tx['howto'] in l and f'winpoint-{newer}' in l for l in c['labels']), card=c)
    h.screenshot('winpoint-nodigest', surface='settings')
    mode(h, 'newer')
