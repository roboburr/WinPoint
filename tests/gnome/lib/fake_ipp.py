#!/usr/bin/env python3
"""A fake CUPS server and IPP Everywhere printer on loopback for batch B's printer pages (notes/claude/settings-parity.md
§5.3, §7; plan.testing: "libcups talks to tests/gnome/lib/fake_ipp.py on loopback through CUPS_SERVER"). Standard
library only. It runs inside the harness sandbox (its own network namespace) or any other private network namespace;
nothing here reaches a real cupsd, a real printer or the LAN.

    python3 -B -I tests/gnome/lib/fake_ipp.py --port 0 --log RUN/logs/fake-ipp.jsonl

The first line on stdout is ``ready <port>`` once it listens on 127.0.0.1. The host's libcups finds it through
``CUPS_SERVER=127.0.0.1:<port>`` or a private ``$HOME/.cups/client.conf`` (``ServerName 127.0.0.1:<port>``).

As the CUPS scheduler (what host/src/services/printers.rs asks through libcups):

* CUPS-Get-Printers, CUPS-Get-Default, Get-Printer-Attributes on ``/printers/<name>``: two queues, ``HP_LaserJet_M404``
  (the default; idle, one toner level, printer-commands with Clean) and ``Epson_ET_2850`` (stopped with
  media-empty-error, four ink levels, no maintenance commands), with ``<Option>-default`` attributes;
* ``GET /printers/<name>.ppd``: a small PPD per queue (General: PageSize, Duplex; Color: ColorModel; Quality:
  cupsPrintQuality; nl translations);
* Get-Jobs (which-jobs not-completed / completed / all), Print-Job (recorded with the document's first bytes, a new
  job), Set-Job-Attributes (job-priority), CUPS-Get-PPDs (a short driver list with ``everywhere``),
  Create-Printer-Subscriptions / Renew-Subscription / Cancel-Subscription (accepted).

As a network printer (``printers.discover {address: "127.0.0.1:<port>"}`` probes ``/ipp/print``): Get-Printer-Attributes
with the IPP Everywhere attributes (ipp-versions-supported 2.0, image/pwg-raster, image/urf, application/pdf,
media-supported, the margins, sides, print-color-mode, print-quality), enough for cupsd's own ``ppd-name everywhere``
PPD generator (amendment 41, recorded in notes/claude/settings-b.md).

Every IPP request is appended to ``--log`` (JSON lines: at, path, op, attrs, docBytes, docHead); ``GET
/_fake/requests`` answers the same list.
"""
from __future__ import annotations

import argparse
import json
import struct
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# --------------------------------------------------------------------------------------------- IPP wire format

TAG_OPERATION, TAG_JOB, TAG_END, TAG_PRINTER, TAG_UNSUPPORTED = 0x01, 0x02, 0x03, 0x04, 0x05
INTEGER, BOOLEAN, ENUM = 0x21, 0x22, 0x23
DATETIME, RESOLUTION, RANGE = 0x31, 0x32, 0x33
TEXT_LANG, NAME_LANG = 0x35, 0x36
TEXT, NAME, KEYWORD, URI, URISCHEME, CHARSET, LANGUAGE, MIME = 0x41, 0x42, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49
NO_VALUE = 0x13

OPS = {
    0x0002: 'Print-Job', 0x0004: 'Validate-Job', 0x0008: 'Cancel-Job', 0x000A: 'Get-Jobs',
    0x000B: 'Get-Printer-Attributes', 0x0014: 'Set-Job-Attributes', 0x0016: 'Create-Printer-Subscriptions',
    0x0018: 'Renew-Subscription', 0x001B: 'Cancel-Subscription', 0x4001: 'CUPS-Get-Default',
    0x4002: 'CUPS-Get-Printers', 0x400C: 'CUPS-Get-PPDs', 0x400F: 'CUPS-Get-PPD',
}
OK, NOT_FOUND, BAD_REQUEST, NOT_SUPPORTED = 0x0000, 0x0406, 0x0400, 0x0501


def decode(body: bytes) -> tuple[int, int, dict, bytes]:
    """(operation, request id, {group: {name: [values]}}, document) of an IPP request."""
    _version, op, rid = struct.unpack('>HHI', body[:8])
    groups: dict[int, dict] = {}
    group, name, i = None, None, 8
    while i < len(body):
        tag = body[i]
        i += 1
        if tag < 0x10:
            if tag == TAG_END:
                break
            group = groups.setdefault(tag, {})
            continue
        n = struct.unpack('>H', body[i:i + 2])[0]
        i += 2
        if n:
            name = body[i:i + n].decode('utf-8', 'replace')
        i += n
        m = struct.unpack('>H', body[i:i + 2])[0]
        i += 2
        raw = body[i:i + m]
        i += m
        if tag in (INTEGER, ENUM):
            value = struct.unpack('>i', raw)[0]
        elif tag == BOOLEAN:
            value = bool(raw[0])
        elif tag >= 0x40 or tag in (TEXT_LANG, NAME_LANG):
            value = raw.decode('utf-8', 'replace')
        else:
            value = raw.hex()
        if group is not None and name is not None:
            group.setdefault(name, []).append(value)
    return op, rid, groups, body[i:]


def _value(tag: int, value) -> bytes:
    if tag in (INTEGER, ENUM):
        return struct.pack('>i', int(value))
    if tag == BOOLEAN:
        return b'\x01' if value else b'\x00'
    if tag == RESOLUTION:
        x, y, units = value
        return struct.pack('>iib', x, y, units)
    if tag == RANGE:
        return struct.pack('>ii', *value)
    if tag == NO_VALUE:
        return b''
    return str(value).encode()


def encode(status: int, rid: int, groups: list[tuple[int, list[tuple[int, str, list]]]]) -> bytes:
    """An IPP response: version 2.0, the status, the request id and the groups [(group tag, [(value tag, name,
    [values])])]; the operation attributes come first."""
    out = bytearray(struct.pack('>HHI', 0x0200, status, rid))
    head = [(CHARSET, 'attributes-charset', ['utf-8']), (LANGUAGE, 'attributes-natural-language', ['en'])]
    for group_tag, attrs in [(TAG_OPERATION, head)] + groups:
        out.append(group_tag)
        for tag, name, values in attrs:
            for k, value in enumerate(values or [None]):
                raw = _value(tag, value) if value is not None else b''
                key = name.encode() if k == 0 else b''
                out += bytes([tag if value is not None else NO_VALUE]) + struct.pack('>H', len(key)) + key
                out += struct.pack('>H', len(raw)) + raw
    out.append(TAG_END)
    return bytes(out)


# ------------------------------------------------------------------------------------------------ the state

PPD_COMMON = '''*PPD-Adobe: "4.3"
*FormatVersion: "4.3"
*LanguageVersion: English
*LanguageEncoding: ISOLatin1
*cupsLanguages: "nl"
*Manufacturer: "{make}"
*ModelName: "{model}"
*NickName: "{model}"
*ShortNickName: "{model}"
*PCFileName: "FAKE.PPD"
*ColorDevice: {color}
*cupsFilter2: "application/vnd.cups-pdf application/pdf 0 -"
*OpenGroup: General/General
*OpenUI *PageSize/Media Size: PickOne
*OrderDependency: 10 AnySetup *PageSize
*DefaultPageSize: A4
*PageSize A4/A4: "<</PageSize[595 842]>>setpagedevice"
*PageSize Letter/US Letter: "<</PageSize[612 792]>>setpagedevice"
*PageSize A5/A5: "<</PageSize[420 595]>>setpagedevice"
*CloseUI: *PageSize
*OpenUI *Duplex/2-Sided Printing: PickOne
*OrderDependency: 10 AnySetup *Duplex
*DefaultDuplex: None
*Duplex None/Off: "<</Duplex false>>setpagedevice"
*Duplex DuplexNoTumble/Long Edge (Standard): "<</Duplex true/Tumble false>>setpagedevice"
*Duplex DuplexTumble/Short Edge (Flip): "<</Duplex true/Tumble true>>setpagedevice"
*CloseUI: *Duplex
*CloseGroup: General
*OpenGroup: Color/Color
*OpenUI *ColorModel/Color Mode: PickOne
*OrderDependency: 10 AnySetup *ColorModel
*DefaultColorModel: {color_default}
*ColorModel Gray/Grayscale: "<</cupsColorSpace 18>>setpagedevice"
{color_rgb}*CloseUI: *ColorModel
*CloseGroup: Color
*OpenGroup: Quality/Print Quality
*OpenUI *cupsPrintQuality/Print Quality: PickOne
*OrderDependency: 10 AnySetup *cupsPrintQuality
*DefaultcupsPrintQuality: Normal
*cupsPrintQuality Draft/Draft: ""
*cupsPrintQuality Normal/Normal: ""
*cupsPrintQuality High/High: ""
*CloseUI: *cupsPrintQuality
*CloseGroup: Quality
*nl.Translation General/Algemeen: ""
*nl.Translation PageSize/Papierformaat: ""
*nl.PageSize Letter/US Letter: ""
*nl.Translation Duplex/Dubbelzijdig afdrukken: ""
*nl.Duplex None/Uit: ""
*nl.Duplex DuplexNoTumble/Lange zijde: ""
*nl.Duplex DuplexTumble/Korte zijde: ""
*nl.Translation Color/Kleur: ""
*nl.Translation ColorModel/Kleurmodus: ""
*nl.ColorModel Gray/Grijswaarden: ""
*nl.ColorModel RGB/Kleur: ""
*nl.Translation Quality/Afdrukkwaliteit: ""
*nl.Translation cupsPrintQuality/Afdrukkwaliteit: ""
*nl.cupsPrintQuality Draft/Concept: ""
*nl.cupsPrintQuality Normal/Normaal: ""
*nl.cupsPrintQuality High/Hoog: ""
'''


def ppd_for(make: str, model: str, color: bool) -> str:
    return PPD_COMMON.format(
        make=make, model=model, color='True' if color else 'False', color_default='RGB' if color else 'Gray',
        color_rgb='*ColorModel RGB/Color: "<</cupsColorSpace 1>>setpagedevice"\n' if color else '')


def printers(port: int) -> dict:
    return {
        'HP_LaserJet_M404': {
            'printer-info': 'HP LaserJet M404', 'printer-location': 'Kantoor',
            'printer-make-and-model': 'HP LaserJet Pro M404-M405, hpcups 3.23', 'printer-state': 3,
            'printer-state-reasons': ['none'], 'printer-state-message': '', 'printer-is-accepting-jobs': True,
            'printer-is-shared': False, 'printer-type': 0x0089D04C, 'device-uri': 'ipp://192.168.1.20/ipp/print',
            'marker-names': ['Black Toner'], 'marker-levels': [64], 'marker-colors': ['#000000'],
            'marker-types': ['toner-cartridge'], 'printer-commands': ['AutoConfigure', 'Clean', 'PrintSelfTestPage'],
            'defaults': {'PageSize': 'A4', 'Duplex': 'DuplexNoTumble'},
            'ppd': ppd_for('HP', 'HP LaserJet Pro M404-M405, hpcups 3.23', False),
        },
        'Epson_ET_2850': {
            'printer-info': 'Epson ET-2850', 'printer-location': 'Studeerkamer',
            'printer-make-and-model': 'Epson ET-2850 Series, Epson Inkjet Printer Driver (ESC/P-R) for Linux',
            'printer-state': 5, 'printer-state-reasons': ['media-empty-error', 'paused'],
            'printer-state-message': 'Papier op', 'printer-is-accepting-jobs': True, 'printer-is-shared': False,
            'printer-type': 0x0080D04C, 'device-uri': 'usb://EPSON/ET-2850%20Series?serial=X5QK000123',
            'marker-names': ['Cyan', 'Magenta', 'Yellow', 'Black'], 'marker-levels': [80, 45, 12, 90],
            'marker-colors': ['#00FFFF', '#FF00FF', '#FFFF00', '#000000'],
            'marker-types': ['ink-tank', 'ink-tank', 'ink-tank', 'ink-tank'], 'printer-commands': ['none'],
            'defaults': {}, 'ppd': ppd_for('EPSON', 'Epson ET-2850 Series', True),
        },
    }


PPDS = [
    ('everywhere', 'Generic', 'IPP Everywhere', ''),
    ('drv:///sample.drv/generic.ppd', 'Generic', 'Generic PostScript Printer', ''),
    ('drv:///hpcups.drv/hp-laserjet_pro_m404-m405.ppd', 'HP', 'HP LaserJet Pro M404-M405, hpcups 3.23',
     'MFG:HP;MDL:HP LaserJet Pro M404-M405;'),
    ('drv:///hpcups.drv/hp-deskjet_2700_series.ppd', 'HP', 'HP DeskJet 2700 Series, hpcups 3.23',
     'MFG:HP;MDL:DeskJet 2700 series;'),
    ('lsb/usr/epson-inkjet-printer-escpr/Epson-ET-2850_Series-epson-escpr-en.ppd', 'EPSON',
     'Epson ET-2850 Series, Epson Inkjet Printer Driver (ESC/P-R) for Linux', 'MFG:EPSON;MDL:ET-2850 Series;'),
]

EVERYWHERE = [
    (NAME, 'printer-name', ['Fake LaserJet']), (TEXT, 'printer-info', ['Fake LaserJet']),
    (TEXT, 'printer-location', ['Lab']), (TEXT, 'printer-make-and-model', ['HP Fake LaserJet']),
    (TEXT, 'printer-device-id', ['MFG:HP;MDL:Fake LaserJet;CMD:PDF,PWGRaster,URF;']),
    (KEYWORD, 'ipp-versions-supported', ['1.1', '2.0']),
    (ENUM, 'operations-supported', [0x0002, 0x0004, 0x0008, 0x0009, 0x000A, 0x000B]),
    (MIME, 'document-format-supported', ['application/octet-stream', 'application/pdf', 'image/pwg-raster',
                                         'image/urf', 'image/jpeg']),
    (MIME, 'document-format-default', ['application/octet-stream']),
    (RESOLUTION, 'pwg-raster-document-resolution-supported', [(300, 300, 3), (600, 600, 3)]),
    (KEYWORD, 'pwg-raster-document-type-supported', ['black_1', 'sgray_8', 'srgb_8']),
    (KEYWORD, 'pwg-raster-document-sheet-back', ['normal']),
    (KEYWORD, 'urf-supported', ['V1.4', 'CP1', 'W8', 'SRGB24', 'RS300-600', 'DM1']),
    (KEYWORD, 'media-supported', ['iso_a4_210x297mm', 'na_letter_8.5x11in', 'iso_a5_148x210mm']),
    (KEYWORD, 'media-default', ['iso_a4_210x297mm']), (KEYWORD, 'media-ready', ['iso_a4_210x297mm']),
    (INTEGER, 'media-bottom-margin-supported', [423]), (INTEGER, 'media-left-margin-supported', [423]),
    (INTEGER, 'media-right-margin-supported', [423]), (INTEGER, 'media-top-margin-supported', [423]),
    (KEYWORD, 'media-type-supported', ['stationery', 'photographic']), (KEYWORD, 'media-type-default', ['stationery']),
    (KEYWORD, 'media-source-supported', ['auto', 'main']), (KEYWORD, 'media-source-default', ['auto']),
    (KEYWORD, 'sides-supported', ['one-sided', 'two-sided-long-edge', 'two-sided-short-edge']),
    (KEYWORD, 'sides-default', ['one-sided']),
    (KEYWORD, 'print-color-mode-supported', ['monochrome', 'color', 'auto']),
    (KEYWORD, 'print-color-mode-default', ['auto']), (BOOLEAN, 'color-supported', [True]),
    (ENUM, 'print-quality-supported', [3, 4, 5]), (ENUM, 'print-quality-default', [4]),
    (RESOLUTION, 'printer-resolution-supported', [(600, 600, 3)]),
    (RESOLUTION, 'printer-resolution-default', [(600, 600, 3)]),
    (RANGE, 'copies-supported', [(1, 99)]), (INTEGER, 'copies-default', [1]),
    (ENUM, 'finishings-supported', [3]), (ENUM, 'finishings-default', [3]),
    (ENUM, 'orientation-requested-supported', [3, 4, 5, 6]), (ENUM, 'orientation-requested-default', [3]),
    (KEYWORD, 'output-bin-supported', ['face-down']), (KEYWORD, 'output-bin-default', ['face-down']),
    (KEYWORD, 'compression-supported', ['none']), (KEYWORD, 'pdl-override-supported', ['attempted']),
    (KEYWORD, 'uri-security-supported', ['none']), (KEYWORD, 'uri-authentication-supported', ['none']),
    (CHARSET, 'charset-configured', ['utf-8']), (CHARSET, 'charset-supported', ['utf-8']),
    (LANGUAGE, 'natural-language-configured', ['en']), (LANGUAGE, 'generated-natural-language-supported', ['en']),
    (ENUM, 'printer-state', [3]), (KEYWORD, 'printer-state-reasons', ['none']),
    (BOOLEAN, 'printer-is-accepting-jobs', [True]), (INTEGER, 'printer-up-time', [1000]),
    (INTEGER, 'queued-job-count', [0]),
]


class State:
    def __init__(self, port: int, log_path: str | None):
        self.port, self.log_path = port, log_path
        self.lock = threading.Lock()
        self.printers = printers(port)
        self.default = 'HP_LaserJet_M404'
        now = int(time.time())
        self.jobs = [
            {'id': 41, 'printer': 'HP_LaserJet_M404', 'name': 'Rapport.pdf', 'user': 'robin', 'state': 5,
             'kb': 220, 'created': now - 300, 'hold': 'no-hold', 'priority': 50},
            {'id': 42, 'printer': 'HP_LaserJet_M404', 'name': 'Foto.jpg', 'user': 'robin', 'state': 4,
             'kb': 1800, 'created': now - 120, 'hold': 'indefinite', 'priority': 50},
            {'id': 37, 'printer': 'Epson_ET_2850', 'name': 'Brief.odt', 'user': 'robin', 'state': 9,
             'kb': 40, 'created': now - 86400, 'hold': 'no-hold', 'priority': 50},
        ]
        self.requests: list[dict] = []
        self.next_job, self.next_sub = 100, 1

    def record(self, entry: dict) -> None:
        with self.lock:
            self.requests.append(entry)
        if self.log_path:
            with open(self.log_path, 'a') as f:
                f.write(json.dumps(entry) + '\n')

    def uri(self, name: str) -> str:
        return f'ipp://127.0.0.1:{self.port}/printers/{name}'

    def printer_attrs(self, name: str, wanted: set[str] | None) -> list:
        p = self.printers[name]
        queued = sum(1 for j in self.jobs if j['printer'] == name and j['state'] in (3, 4, 5, 6))
        attrs = [
            (NAME, 'printer-name', [name]), (TEXT, 'printer-info', [p['printer-info']]),
            (TEXT, 'printer-location', [p['printer-location']]),
            (TEXT, 'printer-make-and-model', [p['printer-make-and-model']]),
            (ENUM, 'printer-state', [p['printer-state']]),
            (KEYWORD, 'printer-state-reasons', p['printer-state-reasons']),
            (TEXT, 'printer-state-message', [p['printer-state-message']]),
            (BOOLEAN, 'printer-is-accepting-jobs', [p['printer-is-accepting-jobs']]),
            (BOOLEAN, 'printer-is-shared', [p['printer-is-shared']]),
            (BOOLEAN, 'printer-is-temporary', [False]),
            (ENUM, 'printer-type', [p['printer-type']]), (URI, 'device-uri', [p['device-uri']]),
            (URI, 'printer-uri-supported', [self.uri(name)]),
            (NAME, 'marker-names', p['marker-names']), (INTEGER, 'marker-levels', p['marker-levels']),
            (NAME, 'marker-colors', p['marker-colors']), (KEYWORD, 'marker-types', p['marker-types']),
            (KEYWORD, 'printer-commands', p['printer-commands']), (INTEGER, 'queued-job-count', [queued]),
            (KEYWORD, 'sides-supported', ['one-sided', 'two-sided-long-edge', 'two-sided-short-edge']),
            (KEYWORD, 'sides-default', ['one-sided']), (INTEGER, 'number-up-supported', [1, 2, 4, 6, 9, 16]),
            (INTEGER, 'number-up-default', [1]), (ENUM, 'orientation-requested-supported', [3, 4, 5, 6]),
            (ENUM, 'orientation-requested-default', [NO_VALUE]),
        ] + [(NAME, f'{k}-default', [v]) for k, v in p['defaults'].items()]
        attrs = [a for a in attrs if a[2] != [NO_VALUE]]
        if wanted and not ({'all', 'printer-description', 'printer-defaults'} & wanted):
            attrs = [a for a in attrs if a[1] in wanted]
        return attrs

    def job_attrs(self, j: dict) -> list:
        return [
            (INTEGER, 'job-id', [j['id']]), (URI, 'job-uri', [f'ipp://127.0.0.1:{self.port}/jobs/{j["id"]}']),
            (NAME, 'job-name', [j['name']]), (ENUM, 'job-state', [j['state']]),
            (KEYWORD, 'job-state-reasons', ['job-hold-until-specified' if j['hold'] != 'no-hold' else 'none']),
            (NAME, 'job-originating-user-name', [j['user']]), (URI, 'job-printer-uri', [self.uri(j['printer'])]),
            (INTEGER, 'job-priority', [j['priority']]), (INTEGER, 'job-k-octets', [j['kb']]),
            (INTEGER, 'time-at-creation', [j['created']]), (KEYWORD, 'job-hold-until', [j['hold']]),
            (INTEGER, 'job-media-sheets-completed', [0]),
        ]


def printer_of(path: str, groups: dict) -> str | None:
    uri = (groups.get(TAG_OPERATION, {}).get('printer-uri') or [''])[0]
    for text in (uri, path):
        if '/printers/' in text:
            return text.rsplit('/printers/', 1)[1].split('?')[0]
    return None


def handle(state: State, path: str, body: bytes) -> bytes:
    op, rid, groups, doc = decode(body)
    opg = groups.get(TAG_OPERATION, {})
    wanted = set(opg.get('requested-attributes', [])) or None
    state.record({'at': time.time(), 'path': path, 'op': OPS.get(op, hex(op)),
                  'attrs': {g: v for t, gv in groups.items() for g, v in gv.items()},
                  'docBytes': len(doc), 'docHead': doc[:48].decode('latin-1')})
    if path.startswith('/ipp/print'):
        if op == 0x000B:
            return encode(OK, rid, [(TAG_PRINTER, EVERYWHERE + [
                (URI, 'printer-uri-supported', [f'ipp://127.0.0.1:{state.port}/ipp/print'])])])
        return encode(NOT_SUPPORTED, rid, [])
    with state.lock:
        if op == 0x4002:
            return encode(OK, rid, [(TAG_PRINTER, state.printer_attrs(n, wanted)) for n in state.printers])
        if op == 0x4001:
            return encode(OK, rid, [(TAG_PRINTER, state.printer_attrs(state.default, wanted))])
        if op == 0x000B:
            name = printer_of(path, groups)
            if name not in state.printers:
                return encode(NOT_FOUND, rid, [])
            return encode(OK, rid, [(TAG_PRINTER, state.printer_attrs(name, wanted))])
        if op == 0x000A:
            name = printer_of(path, groups)
            which = (opg.get('which-jobs') or ['not-completed'])[0]
            done = {'completed': True, 'not-completed': False}.get(which)
            jobs = [j for j in state.jobs if (name is None or j['printer'] == name)
                    and (done is None or (j['state'] >= 7) == done)]
            return encode(OK, rid, [(TAG_JOB, state.job_attrs(j)) for j in jobs])
        if op == 0x0002:
            name = printer_of(path, groups)
            if name not in state.printers:
                return encode(NOT_FOUND, rid, [])
            state.next_job += 1
            job = {'id': state.next_job, 'printer': name, 'name': (opg.get('job-name') or ['Untitled'])[0],
                   'user': (opg.get('requesting-user-name') or ['robin'])[0], 'state': 3, 'kb': max(1, len(doc) // 1024),
                   'created': int(time.time()), 'hold': 'no-hold', 'priority': 50}
            state.jobs.append(job)
            return encode(OK, rid, [(TAG_JOB, [(INTEGER, 'job-id', [job['id']]),
                                               (URI, 'job-uri', [f'ipp://127.0.0.1:{state.port}/jobs/{job["id"]}']),
                                               (ENUM, 'job-state', [3])])])
        if op == 0x0014:
            uri = (opg.get('job-uri') or [''])[0]
            jid = int(uri.rsplit('/', 1)[1]) if uri.rsplit('/', 1)[-1].isdigit() else (opg.get('job-id') or [0])[0]
            for j in state.jobs:
                if j['id'] == jid:
                    j['priority'] = (groups.get(TAG_JOB, {}).get('job-priority') or [j['priority']])[0]
                    return encode(OK, rid, [])
            return encode(NOT_FOUND, rid, [])
        if op == 0x400C:
            return encode(OK, rid, [(TAG_PRINTER, [
                (NAME, 'ppd-name', [n]), (TEXT, 'ppd-make', [mk]), (TEXT, 'ppd-make-and-model', [mm]),
                (TEXT, 'ppd-device-id', [di]), (LANGUAGE, 'ppd-natural-language', ['en']),
                (TEXT, 'ppd-product', [f'({mm.split(",")[0]})'])]) for n, mk, mm, di in PPDS])
        if op in (0x0016, 0x0018):
            state.next_sub += 1
            return encode(OK, rid, [(0x06, [(INTEGER, 'notify-subscription-id', [state.next_sub])])])
        if op == 0x001B:
            return encode(OK, rid, [])
    return encode(NOT_SUPPORTED, rid, [])


class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    state: State

    def log_message(self, *args) -> None:  # quiet
        pass

    def body(self) -> bytes:
        if 'chunked' in (self.headers.get('Transfer-Encoding') or '').lower():
            out = bytearray()
            while True:
                size = int(self.rfile.readline().split(b';')[0].strip() or b'0', 16)
                if size == 0:
                    while self.rfile.readline() not in (b'\r\n', b'\n', b''):
                        pass
                    return bytes(out)
                out += self.rfile.read(size)
                self.rfile.readline()
        return self.rfile.read(int(self.headers.get('Content-Length') or 0))

    def send(self, status: int, ctype: str, data: bytes) -> None:
        self.send_response(status)
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self) -> None:
        data = self.body()
        try:
            answer = handle(self.state, self.path, data)
        except Exception as e:  # a request the fake does not understand
            self.state.record({'at': time.time(), 'path': self.path, 'error': repr(e)})
            answer = encode(BAD_REQUEST, 0, [])
        self.send(200, 'application/ipp', answer)

    def do_GET(self) -> None:
        if self.path == '/_fake/requests':
            with self.state.lock:
                data = json.dumps(self.state.requests).encode()
            return self.send(200, 'application/json', data)
        if self.path.startswith('/printers/') and self.path.endswith('.ppd'):
            name = self.path[len('/printers/'):-len('.ppd')]
            with self.state.lock:
                p = self.state.printers.get(name)
            if p:
                return self.send(200, 'application/vnd.cups-ppd', p['ppd'].encode('latin-1'))
        self.send(404, 'text/plain', b'not found')


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=0)
    ap.add_argument('--log')
    args = ap.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', args.port), Handler)
    port = server.server_address[1]
    Handler.state = State(port, args.log)
    print(f'ready {port}', flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == '__main__':
    sys.exit(main())
