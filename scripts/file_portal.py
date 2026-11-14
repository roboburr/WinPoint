"""FileChooser-only desktop portal registration. Installer use only; no Python runtime.

No running portal or browser is restarted. Only owned files and the single
FileChooser preference are changed; unrelated portal configuration stays intact.
"""
from __future__ import annotations
import base64
import json
import os
from pathlib import Path
import re
import shlex
import stat
import tempfile

NAME = 'org.freedesktop.impl.portal.desktop.win11shell'
IFACE = 'org.freedesktop.impl.portal.FileChooser'
UNIT = 'app-win11-file-portal.service'
MAGIC = 'win11-file-portal-v1'
MANIFEST = 'file-portal.json'
OWNED = 'win11shell'
LANGUAGE = 'nl'  # set by install.py/uninstall.py (Win11's language, contracts C1)
# i18n: pairs (tools/i18n/check_pairs.py); inline, as these texts appear when something is broken (contracts C3).
T = {
    'symlink': {'nl': 'Portalconfiguratie is een symbolische koppeling en is niet overschreven: {path}', 'en': 'The portal configuration is a symbolic link and was not overwritten: {path}'},
    'foreign': {'nl': 'Portalbestand is niet van deze gebruiker: {path}', 'en': 'The portal file does not belong to this user: {path}'},
    'duplicate': {'nl': 'Dubbele [preferred]-sectie of FileChooser-sleutel; portalconfiguratie is niet gewijzigd.', 'en': 'Duplicate [preferred] section or FileChooser key; the portal configuration was not changed.'},
    'badRecord': {'nl': 'Ongeldig portal-installatiebewijs: {path}: {error}', 'en': 'Invalid portal installation record: {path}: {error}'},
    'changed': {'nl': 'Gewijzigd portalbestand is niet overschreven: {path}', 'en': 'A changed portal file was not overwritten: {path}'},
    'unowned': {'nl': 'Bestaand portalbestand zonder eigendomsbewijs is niet overschreven: {path}', 'en': 'An existing portal file without an ownership record was not overwritten: {path}'},
}


def _t(key, **params):
    pair = T[key]
    return (pair.get(LANGUAGE) or pair['nl']).format_map(params)


def _bytes(path: Path):
    if path.is_symlink():
        raise SystemExit(_t('symlink', path=path))
    if not path.exists():
        return None
    if not path.is_file() or path.stat().st_uid != os.getuid():
        raise SystemExit(_t('foreign', path=path))
    return path.read_bytes()


def _b64(value):
    return None if value is None else base64.b64encode(value).decode('ascii')


def _unb64(value):
    return None if value is None else base64.b64decode(value, validate=True)


def _write(path: Path, value: bytes, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix='.'+path.name+'.', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def _parse(raw: bytes):
    """Find only this key, preserving every other byte/comment/section."""
    text = raw.decode('utf-8')
    lines = text.splitlines(keepends=True)
    group = None
    rows = []
    sections = []
    for i, line in enumerate(lines):
        header = re.match(r'^\s*\[([^]\r\n]+)\]\s*(?:[#;].*)?(?:\r?\n)?$', line)
        if header:
            group = header[1]
            if group == 'preferred':
                sections.append(i)
            continue
        if group == 'preferred':
            match = re.match(r'^(\s*)'+re.escape(IFACE)+r'\s*=\s*([^\r\n]*)(\r?\n)?$', line)
            if match:
                rows.append((i, match[2].strip()))
    if len(sections) > 1 or len(rows) > 1:
        raise SystemExit(_t('duplicate'))
    return lines, sections, rows


def _value(raw: bytes):
    _, _, rows = _parse(raw)
    return rows[0][1] if rows else None


def _patch(raw: bytes, value):
    lines, sections, rows = _parse(raw)
    newline = '\r\n' if b'\r\n' in raw else '\n'
    if rows:
        i = rows[0][0]
        if value is None:
            del lines[i]
        else:
            ending = '\r\n' if lines[i].endswith('\r\n') else '\n' if lines[i].endswith('\n') else ''
            indent = re.match(r'^\s*', lines[i])[0]
            lines[i] = indent+IFACE+'='+value+ending
    elif value is not None:
        if sections:
            i = sections[0]+1
            if i and not lines[i-1].endswith(('\n','\r')):
                lines[i-1] += newline
            lines.insert(i, IFACE+'='+value+newline)
        else:
            if lines and not lines[-1].endswith(('\n','\r')):
                lines[-1] += newline
            lines += ['[preferred]'+newline, IFACE+'='+value+newline]
    return ''.join(lines).encode('utf-8')


def _desktop_names():
    values = os.environ.get('XDG_CURRENT_DESKTOP', 'GNOME').split(':')
    values = [v.lower() for v in values if re.fullmatch(r'[A-Za-z0-9_-]{1,64}', v)]
    return list(dict.fromkeys(values+['gnome']))


def _effective(config: Path, data: Path):
    names = [n+'-portals.conf' for n in _desktop_names()]+['portals.conf']
    locations = [config]
    locations += [Path(p) for p in os.environ.get('XDG_CONFIG_DIRS','/etc/xdg').split(':') if p]
    locations += [Path('/etc'), data]
    locations += [Path(p) for p in os.environ.get('XDG_DATA_DIRS','/usr/local/share:/usr/share').split(':') if p]
    locations += [Path('/usr/share')]
    for location in dict.fromkeys(locations):
        for name in names:
            path = location/'xdg-desktop-portal'/name
            if path.is_file():
                return path, path.read_bytes()
    return None, b'[preferred]\ndefault=gnome;gtk;\n'


def _selector(config: Path):
    for name in [n+'-portals.conf' for n in _desktop_names()]+['portals.conf']:
        candidate = config/'xdg-desktop-portal'/name
        if candidate.exists() or candidate.is_symlink():
            return candidate
    return config/'xdg-desktop-portal/gnome-portals.conf'


def _paths(prefix: Path, data: Path, config: Path, selector: Path):
    return {
        'definition':data/'xdg-desktop-portal/portals/win11shell.portal',
        'activation':data/'dbus-1/services'/(NAME+'.service'),
        'unit':config/'systemd/user'/UNIT,
        'selector':selector,
    }


def _contents(prefix: Path):
    # D-Bus service Exec uses key-file unescaping followed by shell-like argv
    # tokenization, without executing a shell or expanding shell substitutions.
    dbus = shlex.quote(str(prefix/'bin/win11-file-portal')).replace('\\','\\\\')
    executable = str(prefix/'bin/win11-file-portal')
    executable = '"'+executable.replace('\\','\\\\').replace('"','\\"').replace('$','$$').replace('%','%%')+'"'
    return {
        'definition':('[portal]\nDBusName='+NAME+'\nInterfaces='+IFACE+';\nUseIn=gnome\n').encode(),
        'activation':('[D-BUS Service]\nName='+NAME+'\nExec='+dbus+'\nSystemdService='+UNIT+'\n').encode(),
        'unit':('[Unit]\nDescription=Win11Shell native FileChooser portal\nCollectMode=inactive-or-failed\n\n[Service]\nType=dbus\nBusName='+NAME+'\nExecStart=/usr/bin/env -- '+executable+'\nSlice=app.slice\n').encode(),
    }


def _load(payload: Path, prefix: Path, data: Path, config: Path):
    path = payload/MANIFEST
    if not path.exists():
        return None
    try:
        value = json.loads(path.read_text())
        if value['marker'] != MAGIC:
            raise ValueError('marker')
        selector = Path(value['files']['selector']['path'])
        if selector.parent != config/'xdg-desktop-portal' or not re.fullmatch(r'(?:[A-Za-z0-9_-]+-)?portals\.conf',selector.name):
            raise ValueError('selector path')
        expected = _paths(prefix,data,config,selector)
        if set(value['files']) != set(expected):
            raise ValueError('files')
        for key, entry in value['files'].items():
            if Path(entry['path']) != expected[key] or not 0 <= entry['mode'] <= 0o777:
                raise ValueError('path/mode')
            _unb64(entry['before']);_unb64(entry['owned'])
        return value
    except (KeyError,TypeError,ValueError,OSError) as error:
        raise SystemExit(_t('badRecord', path=path, error=error))


def session_selector(config: Path):
    """The WinPoint login session's own choice (XDG_CURRENT_DESKTOP WinPoint:GNOME, scripts/install-session.py)."""
    return config/'xdg-desktop-portal/winpoint-portals.conf'


def preflight(payload: Path, prefix: Path, data: Path, config: Path, session: bool = False):
    """With the WinPoint session (session=True) the FileChooser choice lives in winpoint-portals.conf, so GNOME and
    Ubuntu keep their own file chooser; a record that names the other file is undone first (an install from before
    the session, or a session removed since)."""
    previous = _load(payload,prefix,data,config)
    wanted = session_selector(config)
    if previous and (Path(previous['files']['selector']['path']) == wanted) != session:
        unregister(payload,prefix,data,config)
        previous = None
    selector = Path(previous['files']['selector']['path']) if previous else (wanted if session else _selector(config))
    paths = _paths(prefix,data,config,selector)
    generated = _contents(prefix)
    _, effective = _effective(config,data)
    planned = {'marker':MAGIC,'files':{}}
    for key,path in paths.items():
        current = _bytes(path)
        old = previous['files'].get(key) if previous else None
        if old:
            before = _unb64(old['before'])
            mode = old['mode']
            if key != 'selector' and current is not None and current != _unb64(old['owned']):
                raise SystemExit(_t('changed', path=path))
        else:
            if key != 'selector' and current is not None:
                raise SystemExit(_t('unowned', path=path))
            before = current
            mode = stat.S_IMODE(path.stat().st_mode) if current is not None else 0o644
        if key == 'selector':
            source = current if current is not None else effective
            # Preserve an explicit FileChooser change made by the user after install.
            owned = current if old and current is not None and _value(current) != OWNED else _patch(source,OWNED)
            original_value = old.get('value') if old else _value(source)
        else:
            owned = generated[key]
            original_value = None
        planned['files'][key] = {'path':str(path),'before':_b64(before),'owned':_b64(owned),'mode':mode,'value':original_value,'managed':not (key=='selector' and old and (not old.get('managed',True) or current is not None and _value(current)!=OWNED))}
    return planned


def register(payload: Path, plan):
    # Persist recovery ownership before any external registration is changed.
    _write(payload/MANIFEST,(json.dumps(plan,indent=2)+'\n').encode(),0o600)
    for key,entry in plan['files'].items():
        _write(Path(entry['path']),_unb64(entry['owned']),entry['mode'])


def unregister(payload: Path, prefix: Path, data: Path, config: Path):
    state = _load(payload,prefix,data,config)
    if state is None:
        return
    for key,entry in state['files'].items():
        path = Path(entry['path'])
        if key == 'selector' and not entry.get('managed', True):
            continue
        try:
            current = _bytes(path)
        except SystemExit:
            continue
        if current is None:
            continue
        owned,before = _unb64(entry['owned']),_unb64(entry['before'])
        if current == owned:
            if before is None:
                path.unlink()
            else:
                _write(path,before,entry['mode'])
        elif key == 'selector' and _value(current) == OWNED:
            # Restore only our key when unrelated settings were edited meanwhile.
            _write(path,_patch(current,entry['value']),entry['mode'])
    (payload/MANIFEST).unlink(missing_ok=True)
