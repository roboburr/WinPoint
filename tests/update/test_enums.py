#!/usr/bin/python3 -IB
"""GW1: tests/update/enums.json equals the installed PackageKitGlib-1.0 (and NM-1.0) typelib. No D-Bus.

Run as ``python3 -I tests/update/test_enums.py``. Exit 0 = equal (or skipped: typelib absent), 1 = a difference.
Three checks:
1. every nick -> value in enums.json equals the typelib (``pk_<enum>_enum_from_string`` and ``_to_string``);
2. the bit fields equal ``pk_filter_bitfield_from_string`` / ``pk_transaction_flag_bitfield_from_string``;
3. enums.json still holds every value design section 4.6 lists (the table below is copied from the design),
   so a later edit of enums.json cannot silently drop or move a pinned value.
GW2's model.rs is compared with enums.json by GW2's own unit test.
"""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent

# windows-update-gnome.md section 4.6 "Enums" (and 4.3 for NM), verbatim values.
DESIGN = {
    'Role': {'get-details': 3, 'get-update-detail': 8, 'get-updates': 9, 'install-files': 10, 'install-packages': 11,
             'refresh-cache': 13, 'remove-packages': 14, 'update-packages': 22, 'get-old-transactions': 28,
             'repair-system': 29, 'upgrade-system': 33},
    'Status': {'wait': 1, 'setup': 2, 'running': 3, 'query': 4, 'info': 5, 'remove': 6, 'refresh-cache': 7,
               'download': 8, 'install': 9, 'update': 10, 'cleanup': 11, 'obsolete': 12, 'dep-resolve': 13,
               'sig-check': 14, 'test-commit': 15, 'commit': 16, 'finished': 18, 'cancel': 19,
               'download-repository': 20, 'download-packagelist': 21, 'download-filelist': 22,
               'download-changelog': 23, 'download-group': 24, 'download-updateinfo': 25, 'loading-cache': 27,
               'waiting-for-lock': 30, 'waiting-for-auth': 31, 'run-hook': 36},
    'Info': {'installed': 1, 'available': 2, 'low': 3, 'enhancement': 4, 'normal': 5, 'bugfix': 6, 'important': 7,
             'security': 8, 'blocked': 9, 'downloading': 10, 'updating': 11, 'installing': 12, 'removing': 13,
             'cleanup': 14, 'obsoleting': 15, 'finished': 18, 'downgrading': 20, 'preparing': 21,
             'decompressing': 22, 'critical': 26, 'install': 27, 'remove': 28, 'obsolete': 29, 'downgrade': 30},
    'Exit': {'success': 1, 'failed': 2, 'cancelled': 3, 'killed': 6, 'need-untrusted': 8, 'cancelled-priority': 9,
             'repair-required': 11},
    'Restart': {'none': 1, 'application': 2, 'session': 3, 'system': 4, 'security-session': 5, 'security-system': 6},
    'Network': {'offline': 1, 'online': 2, 'wired': 3, 'wifi': 4, 'mobile': 5},
    'Filter': {'none': 1},
    'TransactionFlag': {'only-trusted': 1, 'simulate': 2, 'only-download': 3},
    # "Errors mapped in 3.7": numbers only in the design; names from the typelib.
    'Error': {2: None, 5: None, 10: None, 13: None, 17: None, 25: None, 26: None, 27: None, 30: None, 31: None,
              35: None, 36: None, 37: None, 39: None, 43: None, 46: None, 48: None, 51: None, 56: None, 58: None,
              60: None, 61: None, 65: None, 66: None, 67: None},
}
DESIGN_BITS = {'filter:none': 2, 'flags:only-trusted': 2, 'flags:simulate|only-trusted': 6}
DESIGN_NM = {'Connectivity': {'none': 1, 'portal': 2, 'limited': 3, 'full': 4},
             'Metered': {'yes': 1, 'no': 2, 'guess-yes': 3, 'guess-no': 4}}
FUNCS = {'Role': 'role', 'Status': 'status', 'Info': 'info', 'Exit': 'exit', 'Restart': 'restart',
         'Network': 'network', 'Filter': 'filter', 'TransactionFlag': 'transaction_flag', 'Error': 'error'}


def main() -> int:
    data = json.loads((HERE / 'enums.json').read_text(encoding='utf-8'))
    try:
        import gi
        gi.require_version('PackageKitGlib', '1.0')
        from gi.repository import PackageKitGlib as P
    except (ImportError, ValueError) as e:
        print(f'SKIP: PackageKitGlib-1.0 typelib not available ({e})')
        return 0
    fails, checked = [], 0
    version = f'{P.MAJOR_VERSION}.{P.MINOR_VERSION}.{P.MICRO_VERSION}'
    if data['source']['PackageKitGlib'] != version:
        fails.append(f'enums.json was read from PackageKitGlib {data["source"]["PackageKitGlib"]}, installed is {version}')
    for kind, table in data['PackageKitGlib'].items():
        f = FUNCS[kind]
        from_string, to_string = getattr(P, f'{f}_enum_from_string'), getattr(P, f'{f}_enum_to_string')
        for nick, value in table.items():
            checked += 1
            if int(from_string(nick)) != value:
                fails.append(f'{kind} {nick}: enums.json {value}, typelib {int(from_string(nick))}')
            if to_string(value) != nick:
                fails.append(f'{kind} {value}: enums.json {nick!r}, typelib {to_string(value)!r}')
    for name, value in data['bitfields'].items():
        checked += 1
        kind, _, text = name.partition(':')
        text = text.replace('|', ';')
        fn = P.filter_bitfield_from_string if kind == 'filter' else P.transaction_flag_bitfield_from_string
        if int(fn(text)) != value:
            fails.append(f'bitfield {name}: enums.json {value}, typelib {int(fn(text))}')
    for kind, table in DESIGN.items():
        have = data['PackageKitGlib'].get(kind, {})
        for nick, value in table.items():
            checked += 1
            if kind == 'Error':
                if nick not in have.values():
                    fails.append(f'design error code {nick} missing from enums.json')
            elif have.get(nick) != value:
                fails.append(f'design {kind} {nick}={value}, enums.json {have.get(nick)}')
    for name, value in DESIGN_BITS.items():
        checked += 1
        if data['bitfields'].get(name) != value:
            fails.append(f'design bitfield {name}={value}, enums.json {data["bitfields"].get(name)}')
    nm_note = 'NM-1.0 typelib not available, NM part skipped'
    try:
        gi.require_version('NM', '1.0')
        from gi.repository import NM
        types = {'Connectivity': NM.ConnectivityState, 'Metered': NM.Metered, 'State': NM.State}
        for kind, table in data['NM'].items():
            real = {v.value_nick: int(v) for v in types[kind].__enum_values__.values()}
            for nick, value in table.items():
                checked += 1
                if real.get(nick) != value:
                    fails.append(f'NM {kind} {nick}: enums.json {value}, typelib {real.get(nick)}')
        for kind, table in DESIGN_NM.items():
            for nick, value in table.items():
                checked += 1
                if data['NM'][kind].get(nick) != value:
                    fails.append(f'design NM {kind} {nick}={value}, enums.json {data["NM"][kind].get(nick)}')
        nm_note = 'NM-1.0 compared'
    except (ImportError, ValueError):
        pass
    for f in fails:
        print('FAIL ' + f)
    print(f'test_enums: {checked} values checked against PackageKitGlib {version} ({nm_note}), {len(fails)} differences')
    return 1 if fails else 0


if __name__ == '__main__':
    sys.exit(main())
