"""Settings parity, batch E (notes/claude/settings-parity.md §2, §5.6, §5.8, §7; plan.json "testing"): the Accounts pages
in the private GNOME. Run twice, nl and en, with --strace (Aanmeldingsopties reaches passwd):

    tools/heavy.sh --min-mb 2560 --tag e -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --scenario tests/gnome/scenarios/settings-accounts.py --strace tests/gnome/scenarios/settings-accounts.out/strace.txt
    (and --lang en --out tests/gnome/scenarios/settings-accounts.en.out --strace .../settings-accounts.en.out/strace.txt)

The daemons are batch E's stand-ins on the private buses (tests/gnome/lib/standins/e.py, ``start_live``): they keep
their state and send AccountsService's Changed, fprintd's EnrollStatus and GOA's InterfacesRemoved, so a page that
reads the daemon again after a write shows what the daemon has now. Per page: the title or breadcrumb, a screenshot
(<page>.<lang>), no placeholder, no Dutch word in en. Then:

  accounts        Windows' header from AccountsService (name, Administrator), the four child links, Nu vergrendelen
  yourinfo        Naam wijzigen sends SetRealName (interactive) and the page follows the daemon's Changed; a default
                  picture from GNOME's gallery sends SetIconFile with the stock file, Foto verwijderen SetIconFile('')
  email-accounts  without goa-daemon nothing can be added; with the GOA stand-in the Google account is listed with
                  Beheren (mail and contacts on, calendar off; the calendar switch sets CalendarDisabled false),
                  Verwijderen (Account.Remove, interactive; the account goes), and Account toevoegen lists GOA's
                  providers, one of which starts `win11-gtk-settings --goa-add <provider>`
  signin          no fingerprint row without fprintd (caps), no USB row without usbguard; the lock question (real
                  clicks) writes GNOME's lock-enabled and lock-delay and follows both written from outside; Ook
                  vergrendelen wanneer de pc in slaapstand gaat writes ubuntu-lock-on-suspend and follows it;
                  Automatisch aanmelden sends SetAutomaticLogin; Wachtwoord wijzigen runs passwd (the run/stubs passwd,
                  logged; under --strace no real passwd runs) and shows its refusal; with the fprintd stand-in
                  (Settings started again: caps.get asks fprintd once per backend) the fingerprint row, an enrolment
                  (Claim, EnrollStart, the stages, EnrollStop, Release, each write interactive) and Vingerafdrukken
                  verwijderen (Claim, DeleteEnrolledFingers2, Release)
  otherusers      the other user and not the own one; Automatisch aanmelden, Accounttype wijzigen (SetAccountType)
                  and Wachtwoord opnieuw instellen (SetPasswordMode 1: at the next sign-in), each on that user's
                  object; Account toevoegen (CreateUser, then SetPasswordMode 1 on the new user, who is listed) and
                  Account en gegevens verwijderen (DeleteUser with the files; the user goes); with a logind
                  stand-in in which sam has a session, Verwijderen shows GNOME's "is nog aangemeld" with only
                  Annuleren and users.delete refuses it (no DeleteUser)
  dialogs         a probe: whether a real pointer click reaches a kit dialog's button and switch (the other dialog
                  clicks go through the dev channel, as in native-gtk-apps.py)

No key of these pages is WinPoint-owned (allow/accounts.rs has no Owned row), so there is no pointer to check; locks
are the host unit tests' (contract §8 note 8).
"""
import importlib.util
import os
import sys
import time
from pathlib import Path


def _load(name, path, package=None):
    """A module by path: the harness runs with python -I and loads tests/gnome/lib by path, never from sys.path."""
    spec = importlib.util.spec_from_file_location(name, path, submodule_search_locations=[str(package)] if package else None)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


_LIB = Path(__file__).resolve().parent.parent / 'lib' / 'standins'
_load('ge1h_standins', _LIB / '__init__.py', _LIB)
e = _load('ge1h_standins.e', _LIB / 'e.py')

SCREENSAVER = 'org.gnome.desktop.screensaver'
FACES = '/usr/share/pixmaps/faces/'
# test values for the harness's own dialog (passwd is the run's stub; no real password anywhere)
CURRENT, NEW = 'Qh4-harness-1x', 'Vz8#qLm2!pRt9kWx'


def radio(h, text):
    """The kit radio button (a CheckButton with the w11-radio class) whose label is ``text``."""
    for n in h.find('settings', cls='w11-radio'):
        if text in e.labels(h, n):
            return n
    return None


def accounts_page(h):
    text = e.page(h, 'accounts')
    h.check('accounts: the header names the own account and its type', 'Alex Morgan' in text['text']
            and 'Administrator' in text['text'], text=text['text'][:600])
    links = [f'link-{p}' for p in ('yourinfo', 'email-accounts', 'signin', 'otherusers')]
    h.check('accounts: Accountinstellingen links to the four pages', all(e.node(h, n) for n in links),
            missing=[n for n in links if not e.node(h, n)])
    h.check('accounts: Nu vergrendelen is there', bool(e.node(h, 'accounts-lock')))


def dialog_probe(h):
    """Does a real pointer click reach a kit dialog's button? (slice 1 saw it not; kit dialog.rs's smoke claims every
    press with a GestureClick, and GTK cancels a child's gesture when an ancestor claims its sequence)"""
    e.press(h, 'st-yourinfo-rename')
    if not e.dialog_open(h):
        h.check('dialogs: Naam wijzigen opens its dialog', False)
        return
    h.sleep(0.8)
    click = h.click('settings', 'w11-dialog-close')
    closed = e.until(h, lambda: not h.find('settings', 'w11-dialog') or None, 'Annuleren closes the dialog', 3)
    h.record('dialog probe: real click on Annuleren', {'input': click.get('input'), 'closed': bool(closed)})
    h.check('dialogs: a real pointer click on a kit dialog’s button (Annuleren) closes it', bool(closed)
            and click.get('input') == 'clutter', click=click)
    if not closed:
        h.key('settings', 'Escape')
        e.until(h, lambda: not h.find('settings', 'w11-dialog') or None, 'Escape closes the dialog', 5)


def yourinfo_page(h, acc, uid):
    e.page(h, 'yourinfo')
    if not acc:
        return
    dialog_probe(h)
    seen = len(acc.calls())
    e.press(h, 'st-yourinfo-rename')
    if e.dialog_open(h):
        h.screenshot(f'yourinfo-rename.{h.lang}', 'settings')
        h.type_text('settings', 'st-yourinfo-name-entry', 'Alex Morgan-Reed')
        e.dialog_button(h)
    c = e.call_of(acc, 'SetRealName', seen)
    h.check('yourinfo: Naam wijzigen sends SetRealName to the own user, interactive',
            bool(c) and c['args'] == "('Alex Morgan-Reed',)" and c['interactive'] and c['path'] == e.user_path(uid), call=c)
    h.check('yourinfo: the dialog closes after the change', e.dialog_closed(h))
    h.check('yourinfo: the page follows AccountsService (Changed): the new name',
            bool(h.wait_text('settings', 'Alex Morgan-Reed', timeout=10)))

    gallery = e.node(h, 'st-yourinfo-gallery')
    h.check('yourinfo: GNOME’s gallery (Een standaardafbeelding kiezen)', bool(gallery))
    if gallery:
        # Only visible children are exposed by the repaired kit: open the gallery before enumerating its faces.
        if not e.expander_open(h, 'st-yourinfo-gallery'):
            e.press(h, 'st-yourinfo-gallery')
        faces = e.until(h, lambda: h.find('settings', where=lambda n: str(n.get('name', '')).startswith(
            'st-yourinfo-face-')), 'the open stock-picture gallery', 8) or []
        h.check('yourinfo: the open gallery offers stock pictures', bool(faces))
        h.record('yourinfo faces', [f['name'] for f in faces])
        first = e.expand(h, 'st-yourinfo-gallery', faces[0]['name']) if faces else None
        if first:
            size = first.get('rect', [0, 0, 0, 0])[2:]
            h.check('yourinfo: the default pictures are drawn at their own size (a 64 px picture in its button)',
                    all(60 <= v <= 80 for v in size), size=size)
            h.sleep(0.5)
            h.screenshot(f'yourinfo-gallery.{h.lang}', 'settings')
            seen = len(acc.calls())
            click = e.press(h, faces[0]['name'])
            c = e.call_of(acc, 'SetIconFile', seen)
            stem = faces[0]['name'].removeprefix('st-yourinfo-face-')
            h.check('yourinfo: a default picture (a real click) sends SetIconFile with GNOME’s stock file, interactive',
                    bool(c) and c['args'].startswith(f"('{FACES}{stem}.") and c['interactive']
                    and click.get('input') == 'clutter', call=c, input=click.get('input'))
    remove = e.until(h, lambda: e.node(h, 'st-yourinfo-remove'), 'Foto verwijderen once there is a picture', 10)
    h.check('yourinfo: Foto verwijderen once the account has a picture', bool(remove))
    if remove:
        h.screenshot(f'yourinfo-picture.{h.lang}', 'settings')
        seen = len(acc.calls())
        e.press(h, 'st-yourinfo-remove')
        c = e.call_of(acc, 'SetIconFile', seen)
        h.check('yourinfo: Foto verwijderen sends SetIconFile(\'\'), interactive',
                bool(c) and c['args'] == "('',)" and c['interactive'], call=c)


def email_page(h):
    e.page(h, 'email-accounts')
    add = e.node(h, 'st-email-accounts-add')
    h.check('email-accounts: without goa-daemon nothing can be added', bool(add) and add.get('sensitive') is False, add=add)
    g = e.start_live(h, e.goa(), 'goa')
    if not g:
        return
    h.open('settings:accounts')
    e.page(h, 'email-accounts', f'email-accounts-goa.{h.lang}')
    card = 'st-email-accounts-account_1_0'
    h.check('email-accounts: the GOA account is listed with its address', bool(e.node(h, card))
            and bool(h.wait_text('settings', 'alex@example.org', timeout=5)))
    if e.node(h, card):
        if e.expand(h, card, 'st-email-accounts-manage-account_1_0'):
            h.screenshot(f'email-accounts-open.{h.lang}', 'settings')
            e.press(h, 'st-email-accounts-manage-account_1_0')
            if e.dialog_open(h):
                h.sleep(0.6)
                h.screenshot(f'email-accounts-manage.{h.lang}', 'settings')
                states = {s: e.active(h, f'st-email-accounts-{s}') for s in ('mail', 'calendar', 'contacts')}
                h.check('email-accounts: Beheren shows GOA’s services (mail and contacts on, calendar off)',
                        states == {'mail': True, 'calendar': False, 'contacts': True}, states=states)
                seen = len(g.calls())
                click = h.click('settings', 'st-email-accounts-calendar')
                c = e.call_of(g, 'Set:CalendarDisabled', seen, timeout=4)
                h.record('dialog probe: real click on a switch in a dialog', {'input': click.get('input'), 'reached': bool(c)})
                if not c:
                    h.click('settings', 'st-email-accounts-calendar', real=False)
                    c = e.call_of(g, 'Set:CalendarDisabled', seen)
                h.check('email-accounts: the calendar switch sets CalendarDisabled false on the account',
                        bool(c) and c['args'] == 'false' and c['path'] == e.GOA_ACCOUNT, call=c)
                e.dialog_button(h, 'w11-dialog-close')
                e.dialog_closed(h)
    if e.node(h, card) and e.expand(h, card, 'st-email-accounts-remove-account_1_0'):
        seen = len(g.calls())
        e.press(h, 'st-email-accounts-remove-account_1_0')
        if e.dialog_open(h):
            h.sleep(0.4)
            h.screenshot(f'email-accounts-remove.{h.lang}', 'settings')
            e.dialog_button(h)
        c = e.call_of(g, 'Remove', seen)
        h.check('email-accounts: Verwijderen calls the account’s Remove, interactive',
                bool(c) and c['interactive'] and c['path'] == e.GOA_ACCOUNT, call=c)
        gone = e.until(h, lambda: not e.node(h, card) or None, 'the account goes', 10)
        h.check('email-accounts: the removed account is no longer listed', bool(gone))
    add = e.node(h, 'st-email-accounts-add')
    h.check('email-accounts: with goa-daemon Account toevoegen can be clicked', bool(add) and add.get('sensitive') is not False, add=add)
    if add and add.get('sensitive') is not False:
        e.press(h, 'st-email-accounts-add')
        providers = e.until(h, lambda: h.find('settings', where=lambda n: str(n.get('name', '')).startswith('st-email-accounts-add-')),
                            'GOA’s providers', 8) or []
        names = [n['name'].removeprefix('st-email-accounts-add-') for n in providers]
        h.record('email-accounts providers', names)
        h.check('email-accounts: Account toevoegen lists GOA’s providers (Google, Microsoft 365, Nextcloud, IMAP and SMTP)',
                all(p in names for p in ('google', 'ms_graph', 'owncloud', 'imap_smtp')), names=names)
        h.sleep(0.4)
        h.screenshot(f'email-accounts-add.{h.lang}', 'settings')
        if 'google' in names:
            h.click('settings', 'st-email-accounts-add-google', real=False)
            helper = e.until(h, lambda: [p for p in e.proc_cmdlines('--goa-add') if 'google' in p[1]] or None,
                             'the GOA helper', 10) or []
            h.check('email-accounts: a provider starts win11-gtk-settings --goa-add google (GOA’s own dialog)',
                    bool(helper) and helper[0][1][-2:] == ['--goa-add', 'google']
                    and helper[0][1][0].endswith('win11-gtk-settings'), helper=helper)
            h.sleep(2.0)
            h.screenshot(f'email-accounts-goa-helper.{h.lang}')
            if helper:
                h.hm.stop_pids([pid for pid, _ in helper], 'the GOA helper')
        e.dialog_closed(h, 2)
    e.record_calls(h, 'goa', g)
    g.stop()


def lock_question(h):
    name = 'st-signin-lock-delay'
    h.record('lock-enabled at start', h.gsettings_get(SCREENSAVER, 'lock-enabled'))
    h.record('lock-delay at start', h.gsettings_get(SCREENSAVER, 'lock-delay'))
    if not e.node(h, name):
        h.check('signin: the lock question is on the page', False)
        return
    # GNOME's keyfile backend writes a uint32 without its type ('300'); h.gsettings_set's text has it ('uint32 60')
    both = lambda: (h.gsettings_get(SCREENSAVER, 'lock-enabled'),  # noqa: E731
                    (h.gsettings_get(SCREENSAVER, 'lock-delay') or '').removeprefix('uint32 '))
    click = e.choose(h, name, e.tr(h, 'Na 5 minuten', 'After 5 minutes'), exact=True)
    got = e.until(h, lambda: both() == ('true', '300') or None, 'lock-enabled true, lock-delay 300', 10)
    h.check('signin: Na 5 minuten (real clicks) writes lock-enabled true and lock-delay 300',
            bool(got) and bool(click) and click.get('input') == 'clutter', keys=both(), input=(click or {}).get('input'))
    e.choose(h, name, e.tr(h, 'Nooit', 'Never'), exact=True)
    got = e.until(h, lambda: both() == ('false', '300') or None, 'lock-enabled false, the delay kept', 10)
    h.check('signin: Nooit writes lock-enabled false and keeps the delay, as GNOME', bool(got), keys=both())
    h.gsettings_set(SCREENSAVER, 'lock-delay', 'uint32 60')
    h.gsettings_set(SCREENSAVER, 'lock-enabled', 'true')
    after = e.tr(h, 'Na 1 minuut', 'After 1 minute')
    h.check('signin: the lock question follows lock-enabled and lock-delay written from outside (Na 1 minuut)',
            e.until(h, lambda: after in e.dropdown_label(h, name) or None, after, 10) is not None, label=e.dropdown_label(h, name))
    h.gsettings_set(SCREENSAVER, 'lock-enabled', 'false')
    never = e.tr(h, 'Nooit', 'Never')
    h.check('signin: ... and Nooit when lock-enabled goes false', e.until(
        h, lambda: never in e.dropdown_label(h, name) or None, never, 10) is not None, label=e.dropdown_label(h, name))
    h.screenshot(f'signin-never.{h.lang}', 'settings')


def stub_dir(h):
    """The common harness establishes private permissions; this scenario only checks them."""
    mode = (h.hm.RUN / 'stubs').stat().st_mode & 0o777
    h.check('the tool seam run/stubs is private (0700)', mode == 0o700, mode=oct(mode))


def change_password(h):
    stub_dir(h)
    before = len(e.guard_calls(h, 'passwd'))
    if not e.expand(h, 'st-signin-password', 'st-signin-password-change'):
        h.check('signin: Wachtwoord has Wijzigen', False)
        return
    e.press(h, 'st-signin-password-change')
    if not e.dialog_open(h):
        h.check('signin: Je wachtwoord wijzigen opens', False)
        return
    h.type_text('settings', 'st-signin-password-current', CURRENT)
    h.type_text('settings', 'st-signin-password-new', NEW)
    h.type_text('settings', 'st-signin-password-confirm', NEW)
    ready = e.until(h, lambda: (e.node(h, 'w11-dialog-primary') or {}).get('sensitive') or None, 'Wijzigen enabled', 10)
    hint = (e.node(h, 'st-signin-password-hint') or {}).get('label')
    # libpwquality's dictionary is /var/cache/cracklib, and the sandbox's /var is its own: there every password fails
    # the dictionary check (PWQ_ERROR_CRACKLIB_CHECK, GNOME's "Vermijd veelgebruikte woorden"), as in GNOME Settings
    no_dict = not Path('/var/cache/cracklib/cracklib_dict.pwd').exists()
    h.check('signin: Wijzigen is enabled once the new password is typed twice and strong enough (libpwquality)',
            bool(ready), known='sandbox-cracklib' if not ready and no_dict else None, hint=hint, cracklibInSandbox=not no_dict)
    h.screenshot(f'signin-password.{h.lang}', 'settings')
    if ready:
        e.dialog_button(h)
        answer = None
    else:
        # the same call the dialog makes, through the app's own backend: passwd still has to run
        e.dialog_closed(h, 0.5)
        answer = h.backend('settings', 'users.changePassword', {'current': CURRENT, 'new': NEW}, timeout=40)
        h.record('users.changePassword through the backend', answer)
    calls = e.until(h, lambda: e.guard_calls(h, 'passwd')[before:] or None, 'passwd runs', 15) or []
    h.check('signin: Wachtwoord wijzigen runs passwd for the own account (the run/stubs passwd, no arguments)',
            len(calls) == 1 and calls[0].split('\t')[2:] == [''], calls=calls)
    if ready:
        error = e.until(h, lambda: (lambda n: n if n and n.get('label') else None)(e.node(h, 'st-signin-password-error')),
                        'passwd’s refusal in the dialog', 10)
        h.check('signin: the dialog stays open with the refusal (the stub exits 97)',
                bool(error) and bool(h.find('settings', 'w11-dialog')), error=(error or {}).get('label'))
    else:
        message = str((answer or {}).get('error') or '')
        h.check('signin: passwd’s refusal comes back as an error, not as a lost connection (the stub exits 97)',
                bool(message) and 'Win11 Shell' not in message, answer=answer)
    h.screenshot(f'signin-password-refused.{h.lang}', 'settings')
    e.dialog_closed(h, 1)


def fingerprint(h, fp):
    exp = e.until(h, lambda: e.node(h, 'st-signin-fingerprint'), 'Vingerafdrukherkenning', 10)
    h.check('signin: with fprintd and a reader, Vingerafdrukherkenning is a way to sign in', bool(exp))
    if not exp:
        return
    setup = e.expand(h, 'st-signin-fingerprint', 'st-signin-fingerprint-setup')
    h.screenshot(f'signin-fingerprint.{h.lang}', 'settings')
    one = e.tr(h, 'Er is 1 vingerafdruk ingesteld.', '1 fingerprint has been set up.')
    h.check('signin: one finger set up: Nog een vinger toevoegen and Verwijderen', bool(setup)
            and bool(e.node(h, 'st-signin-fingerprint-remove')) and bool(h.wait_text('settings', one, timeout=5)))
    if setup:
        seen = len(fp.calls())
        e.press(h, 'st-signin-fingerprint-setup')
        if e.dialog_open(h):
            h.sleep(0.4)
            h.screenshot(f'signin-enroll.{h.lang}', 'settings')
            e.dialog_button(h)
            done = e.until(h, lambda: not h.find('settings', 'w11-dialog') or None, 'the enrolment completes', 15)
            h.check('signin: the enrolment follows EnrollStatus to the end and the dialog closes', bool(done))
            e.dialog_closed(h, 1)
        e.until(h, lambda: any(m == 'Release' for m, _, _ in e.writes(fp, seen)) or None, 'the reader released', 10)
        got = e.writes(fp, seen)
        h.check('signin: an enrolment is Claim, EnrollStart(right index finger), EnrollStop, Release; the writes interactive',
                [m for m, _, _ in got] == ['Claim', 'EnrollStart', 'EnrollStop', 'Release']
                and got[0][2] and got[1][2] and got[1][1] == "('right-index-finger',)", calls=got)
        two = e.tr(h, 'Er zijn 2 vingerafdrukken ingesteld.', '2 fingerprints have been set up.')
        e.expand(h, 'st-signin-fingerprint', 'st-signin-fingerprint-setup')
        h.check('signin: then two fingers are set up', bool(h.wait_text('settings', two, timeout=10)))
    if e.expand(h, 'st-signin-fingerprint', 'st-signin-fingerprint-remove'):
        seen = len(fp.calls())
        e.press(h, 'st-signin-fingerprint-remove')
        if e.dialog_open(h):
            e.dialog_button(h)
        e.until(h, lambda: any(m == 'Release' for m, _, _ in e.writes(fp, seen)) or None, 'the reader released', 10)
        got = e.writes(fp, seen)
        h.check('signin: Vingerafdrukken verwijderen is Claim, DeleteEnrolledFingers2, Release; interactive',
                [m for m, _, _ in got] == ['Claim', 'DeleteEnrolledFingers2', 'Release'] and got[0][2] and got[1][2], calls=got)


def signin_page(h, acc, uid):
    e.page(h, 'signin')
    h.check('signin: no fingerprint row without fprintd', not e.node(h, 'st-signin-fingerprint'))
    h.check('signin: no USB row without usbguard (caps)', not e.node(h, 'st-signin-usb-protection'))
    lock_question(h)
    e.flip(h, 'signin', 'st-signin-ubuntu-lock-on-suspend', SCREENSAVER, 'ubuntu-lock-on-suspend', 'true')
    if acc:
        seen = len(acc.calls())
        click = e.press(h, 'st-signin-automatic-login')
        c = e.call_of(acc, 'SetAutomaticLogin', seen)
        h.check('signin: Automatisch aanmelden (a real click) sends SetAutomaticLogin(true) for the own account, interactive',
                bool(c) and c['args'] == '(true,)' and c['interactive'] and c['path'] == e.user_path(uid)
                and click.get('input') == 'clutter', call=c)
        on = e.until(h, lambda: e.active(h, 'st-signin-automatic-login') is True or None, 'the switch stays on', 8)
        h.check('signin: the switch is on after the page reads AccountsService again', bool(on))
    change_password(h)

    # fprintd: caps.get asks it once per backend (cached 30 s), so Settings is started again after the stand-in
    fp = e.start_live(h, e.fprintd(), 'fprintd')
    if fp:
        h.close('settings')
        e.page(h, 'signin', f'signin-fprintd.{h.lang}')
        fingerprint(h, fp)
        e.record_calls(h, 'fprintd', fp)
        fp.stop()


def other_user(h, uid, part):
    """The other user's expander opened until ``part`` of it can be clicked."""
    return e.expand(h, f'st-otherusers-user-{uid}', part)


def otherusers_page(h, acc, uid):
    e.page(h, 'otherusers')
    sam, kim = uid + 1, uid + 2
    h.check('otherusers: the other user is listed, not the own one', bool(e.node(h, f'st-otherusers-user-{sam}'))
            and not e.node(h, f'st-otherusers-user-{uid}'))
    if not acc or not e.node(h, f'st-otherusers-user-{sam}'):
        return
    if not other_user(h, sam, f'st-otherusers-autologin-{sam}'):
        h.check('otherusers: the user expander opens', False)
        return
    h.screenshot(f'otherusers-open.{h.lang}', 'settings')
    seen = len(acc.calls())
    click = e.press(h, f'st-otherusers-autologin-{sam}')
    c = e.call_of(acc, 'SetAutomaticLogin', seen)
    h.check('otherusers: Automatisch aanmelden of another user (a real click) sends SetAutomaticLogin(true) to that user, interactive',
            bool(c) and c['args'] == '(true,)' and c['interactive'] and c['path'] == e.user_path(sam)
            and click.get('input') == 'clutter', call=c)

    seen = len(acc.calls())
    if other_user(h, sam, f'st-otherusers-type-{sam}'):
        e.press(h, f'st-otherusers-type-{sam}')
        if e.dialog_open(h):
            h.sleep(0.4)
            h.screenshot(f'otherusers-type.{h.lang}', 'settings')
            # the choices are named st-otherusers-type-choice-<type>; a dev click (a real one does not reach a dialog)
            if e.node(h, 'st-otherusers-type-choice-administrator'):
                h.click('settings', 'st-otherusers-type-choice-administrator', real=False)
            elif radio(h, 'Administrator'):
                h.click('settings', node=radio(h, 'Administrator'))
            h.sleep(0.4)
            picked = (radio(h, 'Administrator') or {}).get('active')
            h.record('otherusers: Administrator picked in the dialog', picked)
            e.dialog_button(h)
            e.dialog_closed(h, 3)
    c = e.call_of(acc, 'SetAccountType', seen)
    h.check('otherusers: Accounttype wijzigen > Administrator sends SetAccountType(1) to that user, interactive',
            bool(c) and c['args'] == '(1,)' and c['interactive'] and c['path'] == e.user_path(sam), call=c)

    seen = len(acc.calls())
    if other_user(h, sam, f'st-otherusers-password-{sam}'):
        e.press(h, f'st-otherusers-password-{sam}')
        if e.dialog_open(h):
            h.sleep(0.4)
            h.screenshot(f'otherusers-password.{h.lang}', 'settings')
            e.dialog_button(h)
            e.dialog_closed(h, 5)
    c = e.call_of(acc, 'SetPasswordMode', seen)
    h.check('otherusers: Wachtwoord opnieuw instellen (at the next sign-in) sends SetPasswordMode(1) to that user, interactive',
            bool(c) and c['args'] == '(1,)' and c['interactive'] and c['path'] == e.user_path(sam), call=c)

    # Account toevoegen, then Account en gegevens verwijderen
    seen = len(acc.calls())
    e.press(h, 'st-otherusers-add')
    if e.dialog_open(h):
        h.type_text('settings', 'st-otherusers-add-name', 'Kim Lee')
        h.sleep(0.5)
        h.screenshot(f'otherusers-add.{h.lang}', 'settings')
        h.record('otherusers: the suggested user name', (e.node(h, 'st-otherusers-add-username') or {}).get('label'))
        e.dialog_button(h)
        e.dialog_closed(h, 8)
    got = [w for w in e.writes(acc, seen) if w[0] in ('CreateUser', 'SetPasswordMode', 'SetPassword')]
    h.check('otherusers: Account toevoegen sends CreateUser(kim, Kim Lee, standard), then SetPasswordMode(1) '
            '(a password at the first sign-in); interactive',
            got == [('CreateUser', "('kim', 'Kim Lee', 0)", True), ('SetPasswordMode', '(1,)', True)], calls=got)
    listed = e.until(h, lambda: e.node(h, f'st-otherusers-user-{kim}'), 'the new user is listed', 10)
    h.check('otherusers: the new user is listed', bool(listed))
    if listed and other_user(h, kim, f'st-otherusers-remove-{kim}'):
        seen = len(acc.calls())
        e.press(h, f'st-otherusers-remove-{kim}')
        if e.dialog_open(h):
            h.sleep(0.4)
            h.screenshot(f'otherusers-remove.{h.lang}', 'settings')
            e.dialog_button(h)
            e.dialog_closed(h, 3)
        c = e.call_of(acc, 'DeleteUser', seen)
        h.check('otherusers: Bestanden verwijderen sends DeleteUser(uid, true), interactive',
                bool(c) and c['args'] == f'(int64 {kim}, true)' and c['interactive'], call=c)
        gone = e.until(h, lambda: not e.node(h, f'st-otherusers-user-{kim}') or None, 'the user goes', 10)
        h.check('otherusers: the deleted user is no longer listed', bool(gone))

    # GNOME's "%s is still logged in" (review of batch E, finding 1): sam has a session (the logind stand-in), so
    # Verwijderen shows only Annuleren, and users.delete refuses it too: no DeleteUser at all
    lg = e.start_live(h, e.logind([(sam, 'sam')]), 'logind')
    if lg:
        e.page(h, 'accounts', f'accounts-again.{h.lang}')
        e.page(h, 'otherusers', f'otherusers-signed-in-page.{h.lang}')
        seen = len(acc.calls())
        if other_user(h, sam, f'st-otherusers-remove-{sam}'):
            e.press(h, f'st-otherusers-remove-{sam}')
            opened = e.dialog_open(h)
            title = e.tr(h, 'Sam Taylor is nog aangemeld', 'Sam Taylor is still signed in')
            shown = bool(h.wait_text('settings', title, timeout=5)) if opened else False
            h.sleep(0.4)
            h.screenshot(f'otherusers-signed-in.{h.lang}', 'settings')
            only_cancel = not e.node(h, 'w11-dialog-primary') and bool(e.node(h, 'w11-dialog-close'))
            h.check('otherusers: a user who is still signed in gets GNOME’s “is nog aangemeld” with only Annuleren',
                    opened and shown and only_cancel, opened=opened, shown=shown, only_cancel=only_cancel)
            if opened:
                e.dialog_button(h, 'w11-dialog-close')
                e.dialog_closed(h, 3)
        else:
            h.check('otherusers: the signed-in user’s Verwijderen can be reached', False)
        refused = h.backend('settings', 'users.delete', {'uid': sam, 'removeFiles': False})
        h.record('otherusers: users.delete of the signed-in user', refused)
        h.check('otherusers: users.delete refuses the signed-in user (denied users.loggedIn)',
                (refused or {}).get('error', {}).get('code') == 'denied', answer=refused)
        h.check('otherusers: and no DeleteUser reached AccountsService',
                not any(c['method'] == 'DeleteUser' for c in acc.calls()[seen:]))
        h.check('otherusers: logind was asked (ListSessions)', any(c['method'] == 'ListSessions' for c in lg.calls()))
        e.record_calls(h, 'logind', lg)
        lg.stop()


def run(h):
    uid = os.getuid()
    started = time.monotonic()
    acc = e.start_live(h, e.accounts(uid), 'accounts')
    accounts_page(h)
    yourinfo_page(h, acc, uid)
    email_page(h)
    signin_page(h, acc, uid)
    otherusers_page(h, acc, uid)
    h.record('scenario seconds', round(time.monotonic() - started, 1))
    e.record_calls(h, 'accounts', acc)
    if acc:
        acc.stop()
