"""Install bundled Win11 icons privately; expose only collision-checked own aliases.

This module never changes existing Win11 theme folders. It restores the prior
icon choice on uninstall only if the current choice is one of our own aliases.
The main installer owns the payload marker and calls these functions explicitly.
"""
from pathlib import Path
import hashlib,json,os,shutil,subprocess

ALIASES={"Win11Shell":"Win11","Win11Shell-dark":"Win11-dark"}
MARKER='.win11-gnome-owned'
MAGIC='win11-gnome-rust-host-v1\n'
TRACKER='.win11-icon-aliases.json'
STATE='.win11-icon-install-state.json'
LANGUAGE='nl'  # set by install.py/uninstall.py (Win11's language, contracts C1)
# i18n: pairs (tools/i18n/check_pairs.py); inline, as these texts appear when something is broken (contracts C3).
T={
    'unknownBundle':{'nl':'Onbekende pictogrambundel','en':'Unknown icon bundle'},
    'absoluteLink':{'nl':'Absoluut pictogramkoppelpad: {rel}','en':'Absolute icon link path: {rel}'},
    'missingTarget':{'nl':'Ontbrekend pictogramdoel: {rel}','en':'Missing icon link target: {rel}'},
    'linkOutside':{'nl':'Pictogramkoppeling buiten de bundel: {rel}','en':'Icon link outside the bundle: {rel}'},
    'filesDiffer':{'nl':'Pictogrambestanden komen niet overeen met de gecontroleerde bundel','en':'The icon files do not match the verified bundle'},
    'archiveOutside':{'nl':'Bronarchief buiten de bundel','en':'Source archive outside the bundle'},
    'archiveChanged':{'nl':'Het pictogrambronarchief is gewijzigd','en':'The icon source archive has changed'},
    'licenceMissing':{'nl':'Pictogramlicentie of auteurs ontbreken','en':'The icon licence or authors are missing'},
    'aliasForeign':{'nl':'Niet overschreven: bestaande pictogramnaam {alias} is niet van dit pakket.','en':'Not overwritten: the existing icon name {alias} does not belong to this package.'},
    'markerMissing':{'nl':'De pictograminstallatie mist de eigen pakketmarkering.','en':'The icon installation lacks this package’s own marker.'},
}

def _t(key,**params):
    pair=T[key];return (pair.get(LANGUAGE) or pair['nl']).format_map(params)

def _owned(payload):
    try:return (Path(payload)/MARKER).is_file() and (Path(payload)/MARKER).read_text()==MAGIC
    except OSError:return False

def validate(vendor):
    vendor=Path(vendor);base=vendor/'icons';manifest=json.loads((vendor/'icon-bundle.json').read_text())
    if manifest.get('schema')!=1 or manifest.get('aliases')!=ALIASES:raise ValueError(_t('unknownBundle'))
    actual={};stats={'files':0,'symlinks':0,'bytes':0}
    for directory,dirs,files in os.walk(base,followlinks=False):
        for name in dirs+files:
            path=Path(directory)/name;rel=path.relative_to(base).as_posix()
            if path.is_symlink():
                target=os.readlink(path)
                if os.path.isabs(target):raise ValueError(_t('absoluteLink',rel=rel))
                try:resolved=path.resolve(strict=True)
                except (OSError,RuntimeError) as error:raise ValueError(_t('missingTarget',rel=rel)) from error
                if not resolved.is_relative_to(base.resolve()):raise ValueError(_t('linkOutside',rel=rel))
                actual[rel]={'type':'symlink','target':target};stats['symlinks']+=1
            elif path.is_file():
                data=path.read_bytes();actual[rel]={'type':'file','size':len(data),'sha256':hashlib.sha256(data).hexdigest()}
                stats['files']+=1;stats['bytes']+=len(data)
    if actual!=manifest['entries']:raise ValueError(_t('filesDiffer'))
    source=vendor/manifest['upstream']['source_archive']
    if not source.resolve().is_relative_to((vendor/'icon-source').resolve()):raise ValueError(_t('archiveOutside'))
    if hashlib.sha256(source.read_bytes()).hexdigest()!=manifest['upstream']['archive_sha256']:raise ValueError(_t('archiveChanged'))
    for theme in ALIASES.values():
        if not (base/theme/'COPYING').is_file() or not (base/theme/'AUTHORS').is_file():raise ValueError(_t('licenceMissing'))
    return stats

def _same_target(alias,target):
    try:return alias.is_symlink() and alias.resolve(strict=False)==target.resolve(strict=False)
    except (OSError,RuntimeError):return False

def _interface_settings():
    try:
        from gi.repository import Gio
        source=Gio.SettingsSchemaSource.get_default()
        schema=source.lookup('org.gnome.desktop.interface',True) if source else None
        return Gio.Settings.new_full(schema,None,None) if schema else None
    except (ImportError,ValueError,RuntimeError):return None

def _previous_state(payload):
    try:
        state=json.loads((Path(payload)/STATE).read_text())
        previous=state['previous']
        if state.get('schema')==1 and isinstance(previous.get('effective'),str) and (previous.get('user') is None or isinstance(previous.get('user'),str)):
            return state
    except (OSError,ValueError,KeyError,TypeError):pass
    settings=_interface_settings()
    if settings is None:return None
    user=settings.get_user_value('icon-theme')
    return {'schema':1,'previous':{'effective':settings.get_string('icon-theme'),'user':user.unpack() if user else None}}

def preflight_aliases(payload,data):
    payload=Path(payload);data=Path(data)
    for alias,theme in ALIASES.items():
        path=data/'icons'/alias;target=payload/'icons'/theme
        if path.exists() or path.is_symlink():
            if not (_owned(payload) and _same_target(path,target)):
                raise ValueError(_t('aliasForeign',alias=alias))
    return _previous_state(payload)

def copy_into_payload(root,stage):
    root=Path(root);stage=Path(stage);vendor=root/'vendor'
    stats=validate(vendor)
    shutil.copytree(vendor/'icons',stage/'icons',symlinks=True)
    shutil.copytree(vendor/'icon-source',stage/'icon-source',symlinks=True)
    shutil.copy2(vendor/'icon-bundle.json',stage/'icon-bundle.json')
    if (vendor/'ICON_PROVENANCE.md').is_file():shutil.copy2(vendor/'ICON_PROVENANCE.md',stage/'ICON_PROVENANCE.md')
    # Caches are generated at installation time, never copied from a user profile.
    tool=shutil.which('gtk-update-icon-cache')
    if tool:
        for theme in ALIASES.values():
            subprocess.run([tool,'-q','-f','-t',str(stage/'icons'/theme)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=False)
    return stats

def register_aliases(payload,data,previous_state=None):
    payload=Path(payload);data=Path(data)
    if not _owned(payload):raise ValueError(_t('markerMissing'))
    fallback_state=preflight_aliases(payload,data)
    previous_state=previous_state or fallback_state
    icons=data/'icons';icons.mkdir(parents=True,exist_ok=True)
    for alias,theme in ALIASES.items():
        destination=icons/alias;target=payload/'icons'/theme
        if not _same_target(destination,target):os.symlink(os.path.relpath(target,icons),destination,target_is_directory=True)
    (payload/TRACKER).write_text(json.dumps({'schema':1,'aliases':ALIASES},indent=2)+'\n')
    if previous_state is not None:(payload/STATE).write_text(json.dumps(previous_state,indent=2)+'\n')

def _theme_installed(name,data):
    if not isinstance(name,str) or name in ALIASES or '/' in name or not name:return False
    bases=[Path(data)/'icons',Path.home()/'.icons']
    bases += [Path(value)/'icons' for value in os.environ.get('XDG_DATA_DIRS','/usr/local/share:/usr/share').split(':') if value and os.path.isabs(value)]
    return any((base/name/'index.theme').is_file() for base in bases)

def _restore_icon_choice(payload,data):
    settings=_interface_settings()
    if settings is None:return 'unavailable'
    current=settings.get_string('icon-theme')
    if current not in ALIASES:return 'preserved-user-choice'
    if not _same_target(Path(data)/'icons'/current,Path(payload)/'icons'/ALIASES[current]):return 'preserved-user-choice'
    state=_previous_state(payload)
    previous=state.get('previous',{}) if state else {}
    wanted=previous.get('effective')
    if _theme_installed(wanted,data):
        if previous.get('user') is None:settings.reset('icon-theme')
        else:settings.set_string('icon-theme',previous['user'])
    else:
        names=['Win11-dark','Yaru-dark','Adwaita'] if current.endswith('-dark') else ['Win11','Yaru','Adwaita']
        wanted=next((name for name in names if _theme_installed(name,data)),None)
        if wanted:settings.set_string('icon-theme',wanted)
        else:settings.reset('icon-theme')
    from gi.repository import Gio
    Gio.Settings.sync()
    return 'restored-previous-or-stock'

def unregister_aliases(payload,data):
    payload=Path(payload);data=Path(data)
    if not _owned(payload):return {'removed':[],'preserved':list(ALIASES)}
    try:tracker=json.loads((payload/TRACKER).read_text())
    except (OSError,ValueError):return {'removed':[],'preserved':list(ALIASES)}
    if tracker!={'schema':1,'aliases':ALIASES}:return {'removed':[],'preserved':list(ALIASES)}
    result={'removed':[],'preserved':[],'icon_choice':_restore_icon_choice(payload,data)}
    for alias,theme in ALIASES.items():
        path=data/'icons'/alias
        if _same_target(path,payload/'icons'/theme):path.unlink();result['removed'].append(alias)
        else:result['preserved'].append(alias)
    return result
