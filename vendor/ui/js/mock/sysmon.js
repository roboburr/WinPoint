// Win11 Shell — mock sysmon.* (owned by the taskmgr agent). SPEC §6.11.
//
// A small simulation of a desktop PC: 16-thread Ryzen, 32 GB RAM, NVMe + SATA + HDD + USB
// disks, RTX 3080, ≈180 processes (apps with child processes, user daemons, system
// services incl. a running Windows VM). Values drift over time; every call advances the
// simulation to "now", so processes/performance stay consistent with each other.
//
// Shapes mirror host/services/sysmon.py (incl. its additive fields: system groups, word states,
// totals.diskActive, groups canKill/efficiency/threads/user, processes nice/exe/critical,
// performance extras, services displayName/critical, health one item per check).
// Mock only: the LibreOffice window "hangs" (status 'not-responding', ignores a normal end task).
// sysmon.restartPlan {pid} → {pid, how, name, via?, count?, tty?} (what a restart would do; refusals throw, like the
// host's restart_plan: a process of an app restarts the app, unless it has a terminal of its own — then only itself).
// sysmon.restartProcess {pid} → {ok, how:'app'|'service'|'shell'|'exec', error?}: an app comes back with new pids
// (and responds again), a unit's or a single process gets a new pid, the Win11 Shell row restarts the mock host
// (mock/shell.js hostGate: every sysmon.* call is down for a moment). ?mockHostDown=… simulates a dead host.

import { hostGate } from './shell.js';

const KB = 1024, MB = 1024 ** 2, GB = 1024 ** 3;

/* ------------------------------------------------------------------ PRNG */
function mulberry32(a) {
  return function rnd() {
    a |= 0; a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const rnd = mulberry32(20261025);
const between = (a, b) => a + (b - a) * rnd();
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

const NOW0 = Date.now();
const BOOT = NOW0 - ((5 * 86400) + (3 * 3600) + (22 * 60) + 13) * 1000;
const LOGIN = BOOT + 38 * 1000;

/* ------------------------------------------------------------ definitions */
// [name, displayName, cpu (avg % of machine), memory, threads, cmd, extra]
const APP_DEFS = [
  {
    key: 'firefox', name: 'Firefox', icon: 'firefox', desktopId: 'firefox_firefox.desktop',
    exe: '/snap/firefox/7084/usr/lib/firefox/firefox',
    windows: ['Win11 Shell – projectplan — Mozilla Firefox', 'Weerbericht voor Utrecht — Mozilla Firefox'],
    main: { cpu: 1.6, mem: 540 * MB, threads: 104 },
    kids: [
      ['Web Content', 5, 0.25, 120 * MB, 22, '-contentproc -childID {n} -isForBrowser -prefsLen 31544 -appDir /snap/firefox/7084/usr/lib/firefox/browser 4 tab'],
      ['Isolated Web Content', 6, 0.9, 210 * MB, 28, '-contentproc -childID {n} -isForBrowser -prefsLen 32055 -isolatedOrigin 4 tab'],
      ['WebExtensions', 1, 0.15, 160 * MB, 24, '-contentproc -childID 2 -isForBrowser -extension'],
      ['Socket Process', 1, 0.05, 28 * MB, 10, '-contentproc -parentBuildID 20261014 socket'],
      ['RDD Process', 1, 0.35, 62 * MB, 9, '-contentproc -parentBuildID 20261014 rdd'],
      ['Utility Process', 2, 0.05, 34 * MB, 7, '-contentproc -parentBuildID 20261014 utility'],
    ],
  },
  {
    key: 'brave', name: 'Brave', icon: 'brave-browser', desktopId: 'brave-browser.desktop',
    exe: '/opt/brave.com/brave/brave',
    windows: ['Gmail – Inbox — Brave'],
    main: { cpu: 0.8, mem: 330 * MB, threads: 42 },
    kids: [
      ['brave', 7, 0.3, 140 * MB, 18, '--type=renderer --crashpad-handler-pid=4102 --enable-crash-reporter --lang=nl --renderer-client-id={n}', 'Brave (renderer)'],
      ['brave', 1, 0.6, 190 * MB, 14, '--type=gpu-process --crashpad-handler-pid=4102 --gpu-preferences=WAAAAAAAAAAgAAAEAAAAAAAAAAAAAAAAAABgAAEAAAA', 'Brave (GPU-proces)'],
      ['brave', 3, 0.05, 48 * MB, 9, '--type=utility --utility-sub-type=network.mojom.NetworkService --lang=nl', 'Brave (netwerkservice)'],
      ['brave', 2, 0.01, 30 * MB, 4, '--type=zygote --crashpad-handler-pid=4102', 'Brave (zygote)'],
      ['chrome_crashpad_handler', 1, 0.0, 6 * MB, 3, '--monitor-self-annotation=ptype=crashpad-handler --database=/home/robin/.config/BraveSoftware/Brave-Browser/Crash Reports'],
    ],
  },
  {
    key: 'discord', name: 'Discord', icon: 'discord', desktopId: 'discord_discord.desktop',
    exe: '/snap/discord/212/usr/share/discord/Discord',
    windows: ['#algemeen | Vrienden — Discord'],
    main: { cpu: 0.6, mem: 260 * MB, threads: 48 },
    kids: [
      ['Discord', 2, 0.4, 220 * MB, 26, '--type=renderer --autoplay-policy=no-user-gesture-required --enable-crash-reporter', 'Discord (renderer)'],
      ['Discord', 1, 0.3, 120 * MB, 12, '--type=gpu-process --enable-crash-reporter', 'Discord (GPU-proces)'],
      ['Discord', 1, 0.02, 40 * MB, 8, '--type=utility --utility-sub-type=network.mojom.NetworkService', 'Discord (netwerkservice)'],
    ],
  },
  {
    key: 'telegram', name: 'Telegram', icon: 'telegram', desktopId: 'telegram-desktop_telegram-desktop.desktop',
    exe: '/snap/telegram-desktop/6501/usr/bin/telegram-desktop',
    windows: ['Telegram (3)'],
    main: { cpu: 0.25, mem: 390 * MB, threads: 36 }, kids: [],
  },
  {
    key: 'steam', name: 'Steam', icon: 'steam', desktopId: 'steam.desktop',
    exe: '/home/robin/.local/share/Steam/ubuntu12_32/steam',
    windows: ['Steam'],
    main: { cpu: 0.7, mem: 310 * MB, threads: 71 },
    kids: [
      ['steamwebhelper', 5, 0.25, 150 * MB, 20, '-lang=dutch -cachedir=/home/robin/.local/share/Steam/config/htmlcache', 'Steam webhelper'],
      ['steam-runtime-launcher-service', 1, 0.0, 9 * MB, 3, '--alongside-steam'],
      ['srt-logger', 1, 0.0, 4 * MB, 2, '--sh-syntax --terminal'],
    ],
  },
  {
    key: 'code', name: 'Visual Studio Code', icon: 'code', desktopId: 'code_code.desktop',
    exe: '/snap/code/208/usr/share/code/code',
    windows: ['main.js — win11shell — Visual Studio Code'],
    main: { cpu: 0.9, mem: 280 * MB, threads: 38 },
    kids: [
      ['code', 1, 1.4, 520 * MB, 24, '--type=renderer --crashpad-handler-pid=6611 --enable-crash-reporter', 'Code (venster)'],
      ['code', 1, 0.5, 160 * MB, 12, '--type=gpu-process --crashpad-handler-pid=6611', 'Code (GPU-proces)'],
      ['code', 1, 0.4, 340 * MB, 18, '--type=utility --utility-sub-type=node.mojom.NodeService --dns-result-order=ipv4first --inspect-port=0 extensionHost', 'Code (extensies)'],
      ['code', 1, 0.1, 60 * MB, 9, '--type=utility --utility-sub-type=node.mojom.NodeService fileWatcher', 'Code (bestandsbewaking)'],
      ['code', 1, 0.05, 48 * MB, 8, '--type=utility --utility-sub-type=node.mojom.NodeService ptyHost', 'Code (terminal)'],
      ['node', 2, 0.3, 210 * MB, 11, '/snap/code/208/usr/share/code/resources/app/extensions/node_modules/typescript/lib/tsserver.js --serverMode partialSemantic', 'TypeScript-server'],
      ['bash', 1, 0.0, 5 * MB, 1, '--init-file /snap/code/208/usr/share/code/resources/app/out/vs/workbench/contrib/terminal/common/scripts/shellIntegration-bash.sh'],
    ],
  },
  {
    key: 'nautilus', name: 'Bestanden', icon: 'org.gnome.Nautilus', desktopId: 'org.gnome.Nautilus.desktop',
    exe: '/usr/bin/nautilus', windows: ['Downloads'],
    main: { cpu: 0.05, mem: 140 * MB, threads: 12 }, kids: [],
  },
  {
    key: 'terminal', name: 'Terminal', icon: 'org.gnome.Ptyxis', desktopId: 'org.gnome.Ptyxis.desktop',
    exe: '/usr/bin/ptyxis', windows: ['robin@desktop: ~/projecten'],
    main: { cpu: 0.1, mem: 92 * MB, threads: 9 },
    kids: [
      ['ptyxis-agent', 1, 0.0, 8 * MB, 4, '--socket-fd=3'],
      ['bash', 2, 0.0, 6 * MB, 1, '', null, { tty: true }],     // in the terminal's tabs: a terminal of their own
      ['htop', 1, 0.3, 7 * MB, 1, '', null, { tty: true }],
    ],
  },
  {
    key: 'writer', name: 'LibreOffice Writer', icon: 'libreoffice-writer', desktopId: 'libreoffice-writer.desktop',
    exe: '/usr/lib/libreoffice/program/soffice.bin', windows: ['Begroting 2027.odt - LibreOffice Writer'],
    main: { cpu: 0.0, mem: 410 * MB, threads: 11 }, status: 'not-responding',
    kids: [['oosplash', 1, 0.0, 6 * MB, 2, '--writer']],
  },
  {
    key: 'win11shell-taskmgr', name: 'Taakbeheer', icon: 'win11shell-taskmgr', desktopId: 'win11shell-taskmgr.desktop',
    exe: '/usr/bin/python3', cmdline: 'python3 -m host.appwindow --app taskmgr', windows: ['Taakbeheer'],
    main: { cpu: 0.6, mem: 118 * MB, threads: 14 }, kids: [],
  },
];

// Background (user) processes grouped like the host: by systemd user unit when they have one.
const UNIT_NAMES = {
  'gvfs-daemon.service': 'Bestandssysteemdiensten (GVfs)',
  'evolution-source-registry.service': 'Agenda en contacten (Evolution)',
  'ibus.service': 'Invoermethoden (IBus)',
  'at-spi-dbus-bus.service': 'Toegankelijkheid (AT-SPI)',
  'syncthing.service': 'Syncthing',
  'xdg-desktop-portal.service': 'Portaal voor apps',
};
const unitFor = (name) => (/^gvfs/.test(name) ? 'gvfs-daemon.service' : /^evolution/.test(name) ? 'evolution-source-registry.service'
  : /^ibus/.test(name) ? 'ibus.service' : /^at-spi/.test(name) ? 'at-spi-dbus-bus.service' : name === 'syncthing' ? 'syncthing.service' : null);
const CRITICAL = new Set(['Hyprland', 'Xwayland', 'systemd', '(sd-pam)', 'python3']);

// Background (user) processes: [name, displayName, cpu, mem, threads, exe/cmd, extra]
const BG_DEFS = [
  ['python3', 'Win11 Shell', 0.5, 96 * MB, 12, '/usr/bin/python3 -m host --mode layer', { icon: 'win11shell-settings', canKill: false }],
  ['Hyprland', 'Hyprland (vensterbeheer)', 1.1, 180 * MB, 18, '/usr/bin/Hyprland', { canKill: false }],
  ['Xwayland', 'Xwayland', 0.2, 72 * MB, 9, '/usr/bin/Xwayland :0 -rootless -core -listenfd 63 -listenfd 64 -displayfd 104 -wm 101'],
  ['pipewire', 'PipeWire (geluid)', 0.4, 22 * MB, 4, '/usr/bin/pipewire'],
  ['pipewire-pulse', 'PipeWire PulseAudio', 0.2, 18 * MB, 3, '/usr/bin/pipewire-pulse'],
  ['wireplumber', 'WirePlumber', 0.05, 26 * MB, 6, '/usr/bin/wireplumber'],
  ['xdg-desktop-portal', 'Portaal voor apps', 0.0, 18 * MB, 7, '/usr/libexec/xdg-desktop-portal'],
  ['xdg-desktop-portal-hyprland', 'Portaal voor Hyprland', 0.0, 14 * MB, 5, '/usr/libexec/xdg-desktop-portal-hyprland'],
  ['xdg-desktop-portal-gtk', 'Portaal voor GTK', 0.0, 34 * MB, 6, '/usr/libexec/xdg-desktop-portal-gtk'],
  ['xdg-document-portal', 'Documentportaal', 0.0, 9 * MB, 6, '/usr/libexec/xdg-document-portal'],
  ['xdg-permission-store', 'Machtigingenopslag', 0.0, 6 * MB, 4, '/usr/libexec/xdg-permission-store'],
  ['gvfsd', 'Virtuele bestandssystemen', 0.0, 8 * MB, 4, '/usr/libexec/gvfsd'],
  ['gvfsd-fuse', 'gvfsd-fuse', 0.0, 7 * MB, 6, '/usr/libexec/gvfsd-fuse /run/user/1000/gvfs -f'],
  ['gvfsd-trash', 'Prullenbak-bewaking', 0.0, 8 * MB, 4, '/usr/libexec/gvfsd-trash --spawner :1.12 /org/gtk/gvfs/exec_spaw/0'],
  ['gvfsd-metadata', 'gvfsd-metadata', 0.0, 6 * MB, 3, '/usr/libexec/gvfsd-metadata'],
  ['gvfsd-recent', 'Recente bestanden', 0.0, 7 * MB, 4, '/usr/libexec/gvfsd-recent --spawner :1.12 /org/gtk/gvfs/exec_spaw/1'],
  ['gvfsd-network', 'gvfsd-network', 0.0, 8 * MB, 4, '/usr/libexec/gvfsd-network --spawner :1.12 /org/gtk/gvfs/exec_spaw/2'],
  ['gvfs-udisks2-volume-monitor', 'Stationsbewaking', 0.05, 12 * MB, 5, '/usr/libexec/gvfs-udisks2-volume-monitor'],
  ['gvfs-mtp-volume-monitor', 'Telefoonbewaking (MTP)', 0.0, 7 * MB, 4, '/usr/libexec/gvfs-mtp-volume-monitor'],
  ['gvfs-gphoto2-volume-monitor', 'Camerabewaking', 0.0, 7 * MB, 4, '/usr/libexec/gvfs-gphoto2-volume-monitor'],
  ['gvfs-goa-volume-monitor', 'gvfs-goa-volume-monitor', 0.0, 6 * MB, 4, '/usr/libexec/gvfs-goa-volume-monitor'],
  ['gvfs-afc-volume-monitor', 'gvfs-afc-volume-monitor', 0.0, 7 * MB, 5, '/usr/libexec/gvfs-afc-volume-monitor'],
  ['tracker-miner-fs-3', 'Bestanden indexeren', 0.8, 64 * MB, 8, '/usr/libexec/tracker-miner-fs-3', { disk: 380 * KB }],
  ['tracker-xdg-portal-3', 'tracker-xdg-portal-3', 0.0, 6 * MB, 4, '/usr/libexec/tracker-xdg-portal-3'],
  ['evolution-source-registry', 'Agenda-accounts', 0.0, 26 * MB, 6, '/usr/libexec/evolution-source-registry'],
  ['evolution-calendar-factory', 'Agenda', 0.0, 34 * MB, 9, '/usr/libexec/evolution-calendar-factory'],
  ['evolution-addressbook-factory', 'Adresboek', 0.0, 28 * MB, 7, '/usr/libexec/evolution-addressbook-factory'],
  ['evolution-alarm-notify', 'Agendaherinneringen', 0.0, 52 * MB, 6, '/usr/libexec/evolution-data-server/evolution-alarm-notify'],
  ['gnome-keyring-daemon', 'Sleutelbos', 0.0, 9 * MB, 5, '/usr/bin/gnome-keyring-daemon --foreground --components=pkcs11,secrets --control-directory=/run/user/1000/keyring'],
  ['ssh-agent', 'ssh-agent', 0.0, 3 * MB, 1, '/usr/bin/ssh-agent -D -a /run/user/1000/openssh_agent'],
  ['gpg-agent', 'gpg-agent', 0.0, 4 * MB, 2, '/usr/bin/gpg-agent --supervised'],
  ['ibus-daemon', 'Invoermethoden (IBus)', 0.05, 14 * MB, 4, '/usr/bin/ibus-daemon --panel disable'],
  ['ibus-dconf', 'ibus-dconf', 0.0, 6 * MB, 4, '/usr/libexec/ibus-dconf'],
  ['ibus-extension-gtk3', 'ibus-extension-gtk3', 0.0, 30 * MB, 4, '/usr/libexec/ibus-extension-gtk3'],
  ['ibus-portal', 'ibus-portal', 0.0, 6 * MB, 3, '/usr/libexec/ibus-portal'],
  ['ibus-engine-simple', 'ibus-engine-simple', 0.0, 6 * MB, 3, '/usr/libexec/ibus-engine-simple'],
  ['at-spi-bus-launcher', 'Toegankelijkheid', 0.0, 7 * MB, 5, '/usr/libexec/at-spi-bus-launcher'],
  ['at-spi2-registryd', 'at-spi2-registryd', 0.0, 7 * MB, 4, '/usr/libexec/at-spi2-registryd --use-gnome-session'],
  ['dbus-daemon', 'Berichtenbus (sessie)', 0.05, 6 * MB, 1, '/usr/bin/dbus-daemon --session --address=systemd: --nofork --nopidfile --systemd-activation --syslog-only', { canKill: false }],
  ['dbus-daemon', 'Berichtenbus (toegankelijkheid)', 0.0, 4 * MB, 1, '/usr/bin/dbus-daemon --config-file=/usr/share/defaults/at-spi2/accessibility.conf --nofork --print-address 11'],
  ['systemd', 'Servicebeheer (gebruiker)', 0.0, 11 * MB, 1, '/usr/lib/systemd/systemd --user', { canKill: false }],
  ['(sd-pam)', '(sd-pam)', 0.0, 3 * MB, 1, '(sd-pam)', { canKill: false }],
  ['goa-daemon', 'Online accounts', 0.0, 32 * MB, 6, '/usr/libexec/goa-daemon'],
  ['goa-identity-service', 'goa-identity-service', 0.0, 12 * MB, 4, '/usr/libexec/goa-identity-service'],
  ['dconf-service', 'Instellingenopslag', 0.0, 6 * MB, 4, '/usr/libexec/dconf-service'],
  ['snapd-desktop-integration', 'Snap-integratie', 0.0, 16 * MB, 5, '/snap/snapd-desktop-integration/253/usr/bin/snapd-desktop-integration'],
  ['update-notifier', 'Meldingen voor updates', 0.0, 38 * MB, 5, '/usr/bin/update-notifier'],
  ['syncthing', 'Syncthing', 0.4, 88 * MB, 22, '/usr/bin/syncthing serve --no-browser --logfile=default', { disk: 120 * KB }],
  ['syncthing', 'Syncthing (bewaking)', 0.0, 18 * MB, 9, '/usr/bin/syncthing serve --no-browser --logfile=default --monitor'],
  ['flatpak-session-helper', 'Flatpak-sessiehulp', 0.0, 6 * MB, 4, '/usr/libexec/flatpak-session-helper'],
  ['flatpak-portal', 'Flatpak-portaal', 0.0, 7 * MB, 4, '/usr/libexec/flatpak-portal'],
  ['mpris-proxy', 'Mediabediening (Bluetooth)', 0.0, 4 * MB, 1, '/usr/bin/mpris-proxy'],
  ['obexd', 'Bluetooth-bestandsoverdracht', 0.0, 5 * MB, 1, '/usr/libexec/bluetooth/obexd'],
  ['hypridle', 'Inactiviteitsbewaking', 0.0, 9 * MB, 3, '/usr/bin/hypridle'],
  ['hyprpolkitagent', 'Wachtwoordvenster (polkit)', 0.0, 46 * MB, 7, '/usr/libexec/hyprpolkitagent'],
  ['node', 'Ontwikkelserver (node)', 0.3, 120 * MB, 11, '/usr/bin/node /home/robin/projecten/site/server.js --port 5173', { tty: true }],
  ['nvidia-settings', 'NVIDIA-instellingen (laden)', 0.0, 0, 0, '', { skip: true }],
];

// System processes: [name, displayName, user, cpu, mem, threads, cmd, extra]
const SYS_DEFS = [
  ['systemd', 'Systeembeheer (init)', 'root', 0.05, 14 * MB, 1, '/sbin/init splash'],
  ['systemd-journald', 'Systeemlogboek', 'root', 0.05, 46 * MB, 1, '/usr/lib/systemd/systemd-journald', { disk: 40 * KB }],
  ['systemd-udevd', 'Apparaatbeheer', 'root', 0.0, 9 * MB, 1, '/usr/lib/systemd/systemd-udevd'],
  ['systemd-oomd', 'Geheugenbewaking', 'systemd-oom', 0.05, 7 * MB, 1, '/usr/lib/systemd/systemd-oomd'],
  ['systemd-resolved', 'Naamomzetting (DNS)', 'systemd-resolve', 0.0, 14 * MB, 1, '/usr/lib/systemd/systemd-resolved'],
  ['systemd-timesyncd', 'Tijdsynchronisatie', 'systemd-timesync', 0.0, 7 * MB, 2, '/usr/lib/systemd/systemd-timesyncd'],
  ['systemd-logind', 'Aanmeldbeheer', 'root', 0.0, 8 * MB, 1, '/usr/lib/systemd/systemd-logind'],
  ['systemd-machined', 'Virtuele machines registreren', 'root', 0.0, 6 * MB, 1, '/usr/lib/systemd/systemd-machined'],
  ['systemd-userdbd', 'Gebruikersdatabase', 'root', 0.0, 5 * MB, 1, '/usr/lib/systemd/systemd-userdbd'],
  ['systemd-userwork', 'systemd-userwork', 'root', 0.0, 4 * MB, 1, 'systemd-userwork: waiting...', { count: 3 }],
  ['NetworkManager', 'Netwerkbeheer', 'root', 0.05, 22 * MB, 4, '/usr/sbin/NetworkManager --no-daemon'],
  ['wpa_supplicant', 'Wifi-beveiliging', 'root', 0.0, 9 * MB, 1, '/usr/sbin/wpa_supplicant -u -s -O DIR=/run/wpa_supplicant GROUP=netdev'],
  ['ModemManager', 'Modembeheer', 'root', 0.0, 12 * MB, 4, '/usr/sbin/ModemManager'],
  ['polkitd', 'Rechtenbeheer (polkit)', 'polkitd', 0.0, 10 * MB, 4, '/usr/lib/polkit-1/polkitd --no-debug'],
  ['accounts-daemon', 'Gebruikersaccounts', 'root', 0.0, 8 * MB, 4, '/usr/libexec/accounts-daemon'],
  ['udisksd', 'Schijvenbeheer', 'root', 0.05, 16 * MB, 6, '/usr/libexec/udisks2/udisksd'],
  ['upowerd', 'Energiebeheer', 'root', 0.0, 9 * MB, 4, '/usr/libexec/upowerd'],
  ['rtkit-daemon', 'Realtime-planning', 'rtkit', 0.0, 3 * MB, 3, '/usr/libexec/rtkit-daemon'],
  ['cron', 'Geplande taken', 'root', 0.0, 3 * MB, 1, '/usr/sbin/cron -f -P'],
  ['rsyslogd', 'Logboekservice', 'syslog', 0.0, 6 * MB, 4, '/usr/sbin/rsyslogd -n -iNONE'],
  ['avahi-daemon', 'Apparaten in netwerk zoeken', 'avahi', 0.0, 4 * MB, 1, 'avahi-daemon: running [desktop.local]', { count: 2 }],
  ['cupsd', 'Afdrukservice', 'root', 0.0, 12 * MB, 1, '/usr/sbin/cupsd -l'],
  ['cups-browsed', 'Printers zoeken', 'root', 0.0, 11 * MB, 3, '/usr/sbin/cups-browsed'],
  ['snapd', 'Snap-service', 'root', 0.1, 48 * MB, 18, '/usr/lib/snapd/snapd'],
  ['packagekitd', 'Softwarebeheer', 'root', 0.0, 36 * MB, 3, '/usr/libexec/packagekitd'],
  ['fwupd', 'Firmware-updates', 'root', 0.0, 44 * MB, 5, '/usr/libexec/fwupd/fwupd'],
  ['colord', 'Kleurbeheer', 'colord', 0.0, 12 * MB, 3, '/usr/libexec/colord'],
  ['gdm3', 'Aanmeldscherm', 'root', 0.0, 9 * MB, 3, '/usr/sbin/gdm3'],
  ['gdm-session-worker', 'Aanmeldsessie', 'root', 0.0, 10 * MB, 3, 'gdm-session-worker [pam/gdm-password]'],
  ['bluetoothd', 'Bluetooth', 'root', 0.0, 6 * MB, 1, '/usr/libexec/bluetooth/bluetoothd'],
  ['power-profiles-daemon', 'Energieprofielen', 'root', 0.0, 8 * MB, 4, '/usr/libexec/power-profiles-daemon'],
  ['switcheroo-control', 'GPU-keuze', 'root', 0.0, 5 * MB, 3, '/usr/libexec/switcheroo-control'],
  ['thermald', 'Temperatuurbewaking', 'root', 0.0, 8 * MB, 2, '/usr/sbin/thermald --systemd --dbus-enable --adaptive'],
  ['smartd', 'Schijfgezondheid (SMART)', 'root', 0.0, 6 * MB, 1, '/usr/sbin/smartd -n'],
  ['irqbalance', 'irqbalance', 'root', 0.0, 4 * MB, 2, '/usr/sbin/irqbalance'],
  ['kerneloops', 'Kernelfouten melden', 'kernoops', 0.0, 2 * MB, 1, '/usr/sbin/kerneloops --test', { count: 2 }],
  ['unattended-upgrade-shutdown', 'Automatische updates', 'root', 0.0, 22 * MB, 2, '/usr/bin/python3 /usr/share/unattended-upgrades/unattended-upgrade-shutdown --wait-for-signal'],
  ['containerd', 'Containers (containerd)', 'root', 0.1, 52 * MB, 16, '/usr/bin/containerd'],
  ['dockerd', 'Docker', 'root', 0.05, 88 * MB, 24, '/usr/bin/dockerd -H fd:// --containerd=/run/containerd/containerd.sock'],
  ['containerd-shim-runc-v2', 'Container: postgres', 'root', 0.0, 12 * MB, 10, '/usr/bin/containerd-shim-runc-v2 -namespace moby -id 5c0d1e2f3a4b -address /run/containerd/containerd.sock'],
  ['postgres', 'PostgreSQL', '999', 0.05, 38 * MB, 1, 'postgres'],
  ['postgres', 'PostgreSQL (checkpointer)', '999', 0.0, 12 * MB, 1, 'postgres: checkpointer'],
  ['postgres', 'PostgreSQL (background writer)', '999', 0.0, 9 * MB, 1, 'postgres: background writer'],
  ['postgres', 'PostgreSQL (walwriter)', '999', 0.0, 9 * MB, 1, 'postgres: walwriter'],
  ['postgres', 'PostgreSQL (autovacuum)', '999', 0.0, 10 * MB, 1, 'postgres: autovacuum launcher'],
  ['libvirtd', 'Virtuele machines (libvirt)', 'root', 0.05, 42 * MB, 19, '/usr/sbin/libvirtd --timeout 120'],
  ['virtlogd', 'virtlogd', 'root', 0.0, 6 * MB, 2, '/usr/sbin/virtlogd'],
  ['qemu-system-x86_64', 'Virtuele machine: Windows 11', 'libvirt-qemu', 3.4, 8.3 * GB, 12, '/usr/bin/qemu-system-x86_64 -name guest=win11,debug-threads=on -machine pc-q35-9.2 -m 8192 -smp 4 -drive file=/srv/vm/images/win11.qcow2', { disk: 2.2 * MB }],
  ['dnsmasq', 'DHCP voor virtuele machines', 'libvirt-dnsmasq', 0.0, 3 * MB, 1, '/usr/sbin/dnsmasq --conf-file=/var/lib/libvirt/dnsmasq/default.conf', { count: 2 }],
  ['sshd', 'SSH-server', 'root', 0.0, 9 * MB, 1, 'sshd: /usr/sbin/sshd -D [listener] 0 of 10-100 startups'],
  ['nvidia-persistenced', 'NVIDIA-stuurprogramma', 'nvidia-persistenced', 0.0, 3 * MB, 1, '/usr/bin/nvidia-persistenced --user nvidia-persistenced --no-persistence-mode --verbose'],
  ['nvidia-powerd', 'NVIDIA-energiebeheer', 'root', 0.0, 4 * MB, 2, '/usr/bin/nvidia-powerd'],
  ['mount.ntfs', 'NTFS-schijf (Windows)', 'root', 0.1, 24 * MB, 4, '/sbin/mount.ntfs /dev/nvme0n1p2 /run/media/robin/E2CEC932CEC8FFB1 -o rw,nosuid,nodev,uhelper=udisks2', { disk: 60 * KB }],
  ['boltd', 'Thunderbolt', 'root', 0.0, 5 * MB, 3, '/usr/libexec/boltd'],
  ['uuidd', 'uuidd', 'uuidd', 0.0, 2 * MB, 1, '/usr/sbin/uuidd --socket-activation --cont-clock'],
  ['acpid', 'ACPI-gebeurtenissen', 'root', 0.0, 2 * MB, 1, '/usr/sbin/acpid'],
  ['gnome-remote-desktop-daemon', 'Extern bureaublad', 'gnome-remote-desktop', 0.0, 18 * MB, 5, '/usr/libexec/gnome-remote-desktop-daemon --system'],
  ['saned', 'Scannerservice', 'saned', 0.0, 3 * MB, 1, '/usr/sbin/saned'],
  ['agetty', 'agetty', 'root', 0.0, 2 * MB, 1, '/sbin/agetty -o -p -- \\u --noclear - linux'],
];

const KTHREADS = ['kthreadd', 'pool_workqueue_release', 'rcu_preempt', 'rcu_exp_gp_kthread_worker', 'kcompactd0', 'khugepaged', 'kswapd0',
  'irq/127-nvidia', 'nvidia-modeset/kthread_q', 'nv_queue', 'jbd2/nvme0n1p2-8', 'jbd2/nvme1n1p1-8', 'ext4-rsv-conver', 'kworker/u64:2-events_unbound',
  'kworker/7:1-events', 'kworker/3:2-mm_percpu_wq', 'card1-crtc0', 'watchdogd', 'oom_reaper', 'kauditd', 'khungtaskd', 'kintegrityd', 'kblockd', 'ata_sff',
  ...Array.from({ length: 16 }, (_, i) => `ksoftirqd/${i}`), ...Array.from({ length: 16 }, (_, i) => `migration/${i}`)];

/* ------------------------------------------------------------- the model */

let nextPid = 900;
const pidGen = () => (nextPid += 1 + Math.floor(rnd() * 37));

const procs = new Map(); // pid → proc
const apps = [];         // {def, pids:Set, status}

function mkProc({ name, displayName, user = 'robin', cpu, mem, threads, cmd, group, groupKey, canKill = true, disk = 0, ppid = 1, started }) {
  const pid = pidGen();
  const p = {
    pid, ppid, name, displayName: displayName || name, cmd: cmd || name, user,
    state: 'S', baseCpu: cpu, cpu: cpu, baseMem: mem, memory: mem, threads,
    baseDisk: disk, diskRead: 0, diskWrite: 0,
    started: started ?? (LOGIN + Math.floor(between(0, 4 * 3600 * 1000))),
    group, groupKey, canKill, efficiency: false, phase: rnd() * Math.PI * 2,
    exe: /^\//.test(cmd || '') ? String(cmd).split(' ')[0] : '', critical: false,
  };
  procs.set(pid, p);
  return p;
}

function spawnApp(def) {
  const app = { def, pids: new Set(), status: def.status || 'running' };
  const started = LOGIN + Math.floor(between(30, 3 * 3600) * 1000);
  const main = mkProc({
    name: def.exe.split('/').pop(), displayName: def.name, cpu: def.main.cpu, mem: def.main.mem,
    threads: def.main.threads, cmd: def.cmdline || def.exe, group: 'app', groupKey: def.desktopId, started,
  });
  if (app.status === 'not-responding') main.state = 'D';
  app.pids.add(main.pid);
  for (const [name, count, cpu, mem, threads, args, disp, extra] of def.kids) {
    for (let i = 0; i < count; i++) {
      const k = mkProc({
        name, displayName: disp || name, cpu: cpu * between(0.4, 1.6), mem: mem * between(0.6, 1.4), threads,
        cmd: `${name.startsWith('/') ? name : (def.exe.replace(/[^/]+$/, '') + name)} ${String(args).replace('{n}', String(i + 3))}`.trim(),
        group: 'app', groupKey: def.desktopId, ppid: main.pid, started: started + Math.floor(between(1, 900) * 1000),
      });
      if (extra?.tty) k.tty = true;
      app.pids.add(k.pid);
    }
  }
  apps.push(app);
  return app;
}

function init() {
  // kernel threads, system services (low pids), then the session, then apps
  for (const k of KTHREADS) {
    const p = mkProc({ name: k, displayName: k, user: 'root', cpu: k.startsWith('kworker') || k.startsWith('irq') ? 0.02 : 0, mem: 0, threads: 1, cmd: `[${k}]`,
      group: 'system', groupKey: 'kernel', canKill: false, started: BOOT + 1000 });
    p.state = k.startsWith('kworker') ? 'I' : 'S';
    p.kernel = true;
  }
  nextPid = 600;
  for (const d of SYS_DEFS) {
    const [name, disp, user, cpu, mem, threads, cmd, extra = {}] = d;
    const n = extra.count || 1;
    for (let i = 0; i < n; i++) {
      mkProc({
        name, displayName: disp, user, cpu, mem, threads, cmd, group: 'system', groupKey: `exe:${name}`,
        canKill: false, disk: extra.disk || 0, started: BOOT + Math.floor(between(1, 30) * 1000),
      });
    }
  }
  nextPid = 2400;
  for (const d of BG_DEFS) {
    const [name, disp, cpu, mem, threads, cmd, extra = {}] = d;
    if (extra.skip) continue;
    const unit = unitFor(name);
    const key = name === 'python3' ? 'win11shell' : unit ? `unit:${unit}` : `exe:${name}`;
    const p = mkProc({
      name, displayName: disp, cpu, mem, threads, cmd, group: 'background', groupKey: key,
      canKill: (extra.canKill ?? true) && !CRITICAL.has(name), disk: extra.disk || 0, started: LOGIN + Math.floor(between(1, 60) * 1000),
    });
    p.icon = extra.icon || null;
    p.critical = CRITICAL.has(name);
    p.tty = !!extra.tty;
  }
  nextPid = 4000;
  for (const def of APP_DEFS) spawnApp(def);
}
init();

/* ------------------------------------------------------------ simulation */

const CPU = { model: 'AMD Ryzen 7 5800X 8-Core Processor', cores: 8, threads: 16, baseMHz: 3800, maxMHz: 4850 };
const MEM_TOTAL = Math.round(31.26 * GB);
const SWAP_TOTAL = 8 * GB;
// same disks and mount points as mock/storage.js VOLUMES (like the real PC: one NVMe carries the NTFS
// partition and the VM store, /boot/efi is not a volume the UI lists)
const DISKS = [
  { name: 'nvme0n1', model: 'Lexar SSD NM620 1TB', kind: 'nvme', capacity: 1024209543168, mounts: ['/srv/vm', '/run/media/robin/E2CEC932CEC8FFB1'], base: 2.4 },
  { name: 'nvme1n1', model: 'GIGABYTE GP-GSM2NE8256GNTD', kind: 'nvme', capacity: 256060514304, mounts: ['/', '/boot/efi'], base: 0.9 },
  { name: 'sda', model: 'HGST HTS545050A7E680', kind: 'usb', capacity: 500107862016, mounts: ['/run/media/robin/extra'], base: 0 },
  { name: 'sdb', model: 'KINGSTON SA400S37240G', kind: 'usb', capacity: 240057409536, mounts: ['/run/media/robin/BACKUP'], base: 0 },
  { name: 'sdc', model: 'ST24000DM001-3Y7103', kind: 'hdd', capacity: 24000277250048, mounts: ['/run/media/robin/22TB'], base: 0.3 },
  { name: 'sdd', model: 'ST3000VX009-2AY10G', kind: 'hdd', capacity: 3000592982016, mounts: ['/run/media/robin/3TB'], base: 0.1 },
];
const NETS = [
  { iface: 'enp6s0', kind: 'ethernet', name: 'Ethernet', ipv4: '192.168.1.42', ipv6: 'fe80::3a2c:4aff:fe91:7d10', rx: 180 * KB, tx: 24 * KB },
  { iface: 'virbr0', kind: 'other', name: 'Virtueel netwerk (virbr0)', ipv4: '192.168.122.1', ipv6: '', rx: 6 * KB, tx: 3 * KB },
];

const sim = {
  t: 0, last: performance.now(), wallLast: Date.now(),
  cpu: 9, perCore: new Array(CPU.threads).fill(5), freq: 3900, temp: 51,
  memUsed: 0, cached: 9.1 * GB, buffers: 0.42 * GB, swapUsed: 1.08 * GB,
  disks: DISKS.map((d) => ({ ...d, read: 0, write: 0, active: 0, burst: 0 })),
  nets: NETS.map((n) => ({ ...n, rxBps: 0, txBps: 0, burst: 0 })),
  gpu: { usage: 6, memUsed: 2.3 * GB, temp: 47 },
  burst: 0, // occasional CPU burst (e.g. a page loading)
  burstPid: null,
};

function noise(p, t, speed = 0.7) {
  // smooth-ish noise in 0..2 around 1
  return 1 + 0.45 * Math.sin(t * speed + p.phase) + 0.35 * Math.sin(t * speed * 2.7 + p.phase * 1.7) + (Math.random() - 0.5) * 0.5;
}

function step() {
  const now = performance.now();
  const dt = Math.min(5, Math.max(0, (now - sim.last) / 1000));
  if (dt < 0.25 && sim.t > 0) return; // at most ~4 updates per second
  sim.last = now;
  sim.t += dt || 1;
  const t = sim.t;

  // CPU bursts: every ~20–40 s something busy happens for a few seconds
  if (sim.burst <= 0 && Math.random() < dt / 25) {
    const candidates = [...procs.values()].filter((p) => p.group === 'app' && p.baseCpu > 0.2);
    const pick = candidates[Math.floor(Math.random() * candidates.length)];
    sim.burst = between(3, 7);
    sim.burstPid = pick?.pid ?? null;
  }
  sim.burst = Math.max(0, sim.burst - dt);

  let total = 0, mem = 0;
  for (const p of procs.values()) {
    let c = p.baseCpu * 0.55 * Math.max(0, noise(p, t));
    if (sim.burst > 0 && p.pid === sim.burstPid) c += between(6, 14);
    if (p.efficiency) c *= 0.55;
    p.cpu = clamp(c, 0, 100);
    total += p.cpu;
    p.memory = p.kernel ? 0 : Math.max(1 * MB, p.baseMem * (1 + 0.03 * Math.sin(t / 20 + p.phase)) + (p.baseMem > 300 * MB ? Math.sin(t / 7 + p.phase) * 6 * MB : 0));
    mem += p.memory;
    if (p.baseDisk) {
      const d = p.baseDisk * Math.max(0, noise(p, t, 1.3) - 0.4) * 1.6;
      p.diskRead = d * 0.7; p.diskWrite = d * 0.3;
    } else if (p.group === 'app' && Math.random() < 0.04) {
      p.diskRead = Math.random() * 600 * KB; p.diskWrite = Math.random() * 300 * KB;
    } else { p.diskRead *= 0.3; p.diskWrite *= 0.3; if (p.diskRead < 1024) p.diskRead = 0; if (p.diskWrite < 1024) p.diskWrite = 0; }
  }
  sim.cpu = clamp(total + 1.2, 0.5, 100);
  // spread over cores: a few busier ones
  sim.perCore = sim.perCore.map((v, i) => {
    const target = sim.cpu * (0.55 + ((i * 7) % 5) * 0.22) * (0.6 + Math.random() * 0.8);
    return clamp(v * 0.35 + target * 0.65, 0, 100);
  });
  sim.freq = clamp(3550 + sim.cpu * 28 + (Math.random() - 0.5) * 300, 2200, CPU.maxMHz);
  sim.temp = clamp(sim.temp * 0.9 + (44 + sim.cpu * 0.55) * 0.1 + (Math.random() - 0.5), 38, 88);
  sim.memUsed = mem + 1.25 * GB; // kernel, shared, slab
  sim.cached = clamp(sim.cached + (Math.random() - 0.5) * 30 * MB, 8 * GB, 10 * GB);

  let diskSum = 0;
  for (const p of procs.values()) diskSum += p.diskRead + p.diskWrite;
  sim.disks.forEach((d) => {
    if (d.burst <= 0 && Math.random() < dt / 30 * d.base) d.burst = between(2, 5);
    d.burst = Math.max(0, d.burst - dt);
    const shareOfProcs = d.name === 'nvme1n1' ? diskSum * 0.7 : d.name === 'nvme0n1' ? diskSum * 0.3 : 0;
    const extra = d.burst > 0 ? between(40, 180) * MB : 0;
    d.read = shareOfProcs * 0.65 + extra * 0.8 + (d.base ? Math.random() * 120 * KB * d.base : 0);
    d.write = shareOfProcs * 0.35 + extra * 0.2 + (d.base ? Math.random() * 90 * KB * d.base : 0);
    const speed = d.kind === 'hdd' ? 180 * MB : d.kind === 'nvme' ? 1800 * MB : 450 * MB;
    d.active = clamp(((d.read + d.write) / speed) * 100 * (d.kind === 'hdd' ? 3 : 1.4) + (d.base ? Math.random() * d.base : 0), 0, 100);
  });

  sim.nets.forEach((n) => {
    if (n.burst <= 0 && Math.random() < dt / 18) n.burst = between(2, 6);
    n.burst = Math.max(0, n.burst - dt);
    const f = Math.max(0, 1 + 0.6 * Math.sin(t / 3 + n.rx) + (Math.random() - 0.5) * 0.8);
    n.rxBps = n.rx * f + (n.burst > 0 && n.kind === 'ethernet' ? between(2, 9) * MB : 0);
    n.txBps = n.tx * f + (n.burst > 0 && n.kind === 'ethernet' ? between(50, 400) * KB : 0);
  });

  sim.gpu.usage = clamp(sim.gpu.usage * 0.6 + (4 + sim.cpu * 0.35 + Math.random() * 6) * 0.4, 0, 100);
  sim.gpu.memUsed = clamp(sim.gpu.memUsed + (Math.random() - 0.5) * 40 * MB, 2.0 * GB, 2.8 * GB);
  sim.gpu.temp = clamp(sim.gpu.temp * 0.92 + (42 + sim.gpu.usage * 0.4) * 0.08, 35, 85);
}

/* ------------------------------------------------------------- snapshots */

function procOut(p) {
  return {
    pid: p.pid, ppid: p.ppid, name: p.name, displayName: p.displayName, cmd: p.cmd, user: p.user,
    state: STATE_WORDS[p.state] || 'sleeping', cpu: +p.cpu.toFixed(2), memory: Math.round(p.memory),
    diskRead: Math.round(p.diskRead), diskWrite: Math.round(p.diskWrite), threads: p.threads,
    started: p.started, group: p.group, groupKey: p.groupKey, canKill: p.canKill, efficiency: p.efficiency,
    nice: p.efficiency ? 10 : 0, exe: p.exe || null, critical: !!p.critical,
  };
}
const STATE_WORDS = { R: 'running', S: 'sleeping', D: 'waiting', T: 'stopped', t: 'stopped', Z: 'zombie', I: 'idle' };

function groupOut(key, name, icon, desktopId, list, extra = {}) {
  return {
    key, name, icon, desktopId, pids: list.map((p) => p.pid),
    cpu: +list.reduce((a, p) => a + p.cpu, 0).toFixed(2),
    memory: Math.round(list.reduce((a, p) => a + p.memory, 0)),
    diskRate: Math.round(list.reduce((a, p) => a + p.diskRead + p.diskWrite, 0)),
    windows: extra.windows || [],
    status: extra.status || (list.length && list.every((p) => p.state === 'T') ? 'suspended' : list.length && list.every((p) => p.efficiency) ? 'efficiency' : 'running'),
    canKill: list.some((p) => p.canKill), efficiency: list.length > 0 && list.every((p) => p.efficiency),
    threads: list.reduce((a, p) => a + p.threads, 0), user: list[0]?.user || '',
  };
}

function processesSnapshot() {
  step();
  const appsOut = [];
  for (const app of apps) {
    const list = [...app.pids].map((pid) => procs.get(pid)).filter(Boolean);
    if (!list.length) continue;
    const st = app.status === 'not-responding' ? 'not-responding' : undefined;
    appsOut.push(groupOut(app.def.desktopId, app.def.name, app.def.icon, app.def.desktopId, list, { windows: app.def.windows, status: st }));
  }
  const bgMap = new Map();
  for (const p of procs.values()) {
    if (p.group !== 'background') continue;
    const k = p.groupKey;
    if (!bgMap.has(k)) bgMap.set(k, []);
    bgMap.get(k).push(p);
  }
  const groupName = (k, list) => (k === 'win11shell' ? 'Win11 Shell' : k.startsWith('unit:') ? (UNIT_NAMES[k.slice(5)] || list[0].displayName) : list[0].displayName);
  const background = [...bgMap.entries()].map(([k, list]) => groupOut(k, groupName(k, list), list[0].icon || null, null, list))
    .sort((a, b) => a.name.localeCompare(b.name, 'nl'));
  const sysMap = new Map();
  for (const p of procs.values()) {
    if (p.group !== 'system') continue;
    if (!sysMap.has(p.groupKey)) sysMap.set(p.groupKey, []);
    sysMap.get(p.groupKey).push(p);
  }
  const system = [...sysMap.entries()].map(([k, list]) => groupOut(k, k === 'kernel' ? 'Linux-kernel' : list[0].displayName, null, null, list))
    .sort((a, b) => a.name.localeCompare(b.name, 'nl'));
  const all = [...procs.values()];
  const diskActive = Math.max(...sim.disks.map((d) => d.active));
  return {
    timestamp: Date.now(),
    totals: {
      cpu: +sim.cpu.toFixed(1), memoryUsed: Math.round(sim.memUsed), memoryTotal: MEM_TOTAL,
      processes: all.length, threads: all.reduce((a, p) => a + p.threads, 0),
      diskRead: Math.round(all.reduce((a, p) => a + p.diskRead, 0)),
      diskWrite: Math.round(all.reduce((a, p) => a + p.diskWrite, 0)),
      diskActive: +diskActive.toFixed(1), networkRx: Math.round(sim.nets[0].rxBps), networkTx: Math.round(sim.nets[0].txBps), cpuCount: CPU.threads,
    },
    apps: appsOut,
    background,
    system,
    processes: all.map(procOut),
  };
}

function performanceSnapshot() {
  step();
  const all = [...procs.values()];
  return {
    timestamp: Date.now(),
    cpu: {
      model: CPU.model, usage: +sim.cpu.toFixed(1), perCore: sim.perCore.map((v) => +v.toFixed(1)),
      freqMHz: Math.round(sim.freq), cores: CPU.cores, threads: CPU.threads,
      uptime: Math.round((Date.now() - BOOT) / 1000), processes: all.length,
      threadsTotal: all.reduce((a, p) => a + p.threads, 0) + 412, temperature: Math.round(sim.temp),
      baseMHz: CPU.baseMHz, maxMHz: CPU.maxMHz, load: [+(sim.cpu / 6.2).toFixed(2), 1.21, 1.08],
    },
    memory: {
      total: MEM_TOTAL, used: Math.round(sim.memUsed), available: Math.round(MEM_TOTAL - sim.memUsed),
      cached: Math.round(sim.cached), buffers: Math.round(sim.buffers), swapTotal: SWAP_TOTAL, swapUsed: Math.round(sim.swapUsed),
      committed: Math.round(sim.memUsed + 6.4 * GB), commitLimit: Math.round(MEM_TOTAL / 2 + SWAP_TOTAL), shared: Math.round(0.9 * GB), kernel: Math.round(0.6 * GB),
    },
    disks: sim.disks.map((d, i) => ({
      name: `Schijf ${i}`, device: d.name, index: i, model: d.model, kind: d.kind, readBps: Math.round(d.read), writeBps: Math.round(d.write),
      activePct: +d.active.toFixed(1), capacity: d.capacity, mounts: d.mounts, removable: d.kind === 'usb',
      responseMs: +(d.kind === 'hdd' ? 4 + d.active / 8 : 0.1 + d.active / 40).toFixed(1),
    })),
    network: sim.nets.map((n) => ({
      iface: n.iface, kind: n.kind, name: n.name, rxBps: Math.round(n.rxBps), txBps: Math.round(n.txBps), ipv4: n.ipv4, ipv6: n.ipv6,
      connected: true, linkSpeedMbps: n.kind === 'ethernet' ? 1000 : null,
      adapter: n.kind === 'ethernet' ? 'Realtek RTL8125 2.5GbE Controller' : null, connection: n.kind === 'ethernet' ? 'Bekabelde verbinding 1' : 'virbr0',
    })),
    gpus: [{
      name: 'NVIDIA GeForce RTX 3080', usage: +sim.gpu.usage.toFixed(1), memUsed: Math.round(sim.gpu.memUsed),
      memTotal: 10 * GB, temperature: Math.round(sim.gpu.temp), vendor: 'nvidia',
      engines: { '3d': +(sim.gpu.usage * 0.9).toFixed(1), encode: 0, decode: +(sim.gpu.usage * 0.15).toFixed(1) },
      power: Math.round(28 + sim.gpu.usage * 1.9), clockMHz: Math.round(420 + sim.gpu.usage * 14), driver: '580.95.05', slot: '0000:0b:00.0',
    }],
  };
}

/* --------------------------------------------------------------- startup */

const STARTUP = [
  { id: 'discord_discord.desktop', name: 'Discord', icon: 'discord', enabled: true, command: '/snap/bin/discord --start-minimized', source: 'user' },
  { id: 'telegram-desktop_telegram-desktop.desktop', name: 'Telegram', icon: 'telegram', enabled: true, command: '/snap/bin/telegram-desktop -startintray', source: 'user' },
  { id: 'steam.desktop', name: 'Steam', icon: 'steam', enabled: false, command: '/usr/bin/steam -silent %U', source: 'user' },
  { id: 'syncthing-start.desktop', name: 'Syncthing', icon: 'syncthing', enabled: true, command: '/usr/bin/syncthing serve --no-browser --logfile=default', source: 'user' },
  { id: 'brave-browser.desktop', name: 'Brave', icon: 'brave-browser', enabled: false, command: '/usr/bin/brave-browser-stable --no-startup-window', source: 'user' },
  { id: 'im-launch.desktop', name: 'Invoermethoden (IBus)', icon: 'ibus-setup', enabled: true, command: 'sh -c \'if [ "x$XDG_SESSION_TYPE" = "xwayland" ] ; then exec env IM_CONFIG_CHECK_ENV=1 im-launch true; fi\'', source: 'system' },
  { id: 'gnome-keyring-pkcs11.desktop', name: 'Sleutelbos: certificaten', icon: 'seahorse', enabled: true, command: '/usr/bin/gnome-keyring-daemon --start --components=pkcs11', source: 'system' },
  { id: 'gnome-keyring-secrets.desktop', name: 'Sleutelbos: wachtwoorden', icon: 'seahorse', enabled: true, command: '/usr/bin/gnome-keyring-daemon --start --components=secrets', source: 'system' },
  { id: 'gnome-keyring-ssh.desktop', name: 'Sleutelbos: SSH-sleutels', icon: 'seahorse', enabled: true, command: '/usr/bin/gnome-keyring-daemon --start --components=ssh', source: 'system' },
  { id: 'org.gnome.Evolution-alarm-notify.desktop', name: 'Agendaherinneringen', icon: 'appointment-soon', enabled: true, command: '/usr/libexec/evolution-data-server/evolution-alarm-notify', source: 'system' },
  { id: 'update-notifier.desktop', name: 'Meldingen voor updates', icon: 'update-notifier', enabled: true, command: 'update-notifier', source: 'system' },
  { id: 'nvidia-settings-autostart.desktop', name: 'NVIDIA-instellingen laden', icon: 'nvidia-settings', enabled: true, command: 'sh -c \'/usr/bin/nvidia-settings --load-config-only\'', source: 'system' },
  { id: 'snap-userd-autostart.desktop', name: 'Snap-gebruikersdienst', icon: 'snap', enabled: true, command: '/usr/bin/snap userd --autostart', source: 'system' },
  { id: 'tracker-miner-fs-3.desktop', name: 'Bestanden indexeren', icon: 'system-search', enabled: true, command: '/usr/libexec/tracker-miner-fs-3', source: 'system' },
  { id: 'xdg-user-dirs.desktop', name: 'Gebruikersmappen bijwerken', icon: 'folder', enabled: true, command: 'xdg-user-dirs-update', source: 'system' },
];

/* -------------------------------------------------------------- services */

const SERVICES = [
  ['pipewire.service', 'PipeWire: geluid en video', 'active', 'running'],
  ['pipewire-pulse.service', 'PipeWire: geluid voor oudere apps (PulseAudio)', 'active', 'running'],
  ['wireplumber.service', 'Geluidsapparaten koppelen (WirePlumber)', 'active', 'running'],
  ['xdg-desktop-portal.service', 'Portaal: bestanden openen, schermdelen', 'active', 'running'],
  ['xdg-desktop-portal-hyprland.service', 'Portaal voor Hyprland', 'active', 'running'],
  ['xdg-document-portal.service', 'Documentportaal', 'active', 'running'],
  ['xdg-permission-store.service', 'Machtigingen van apps', 'active', 'running'],
  ['gvfs-daemon.service', 'Virtuele bestandssystemen (netwerk, telefoons)', 'active', 'running'],
  ['gvfs-udisks2-volume-monitor.service', 'Stations en USB-sticks herkennen', 'active', 'running'],
  ['tracker-miner-fs-3.service', 'Bestanden indexeren voor zoeken', 'active', 'running'],
  ['evolution-source-registry.service', 'Agenda- en contactaccounts', 'active', 'running'],
  ['dbus.service', 'Berichtenbus van je sessie', 'active', 'running'],
  ['at-spi-dbus-bus.service', 'Toegankelijkheid (schermlezer)', 'active', 'running'],
  ['syncthing.service', 'Syncthing: bestanden synchroniseren', 'active', 'running'],
  ['ssh-agent.service', 'SSH-sleutels onthouden', 'active', 'running'],
  ['win11shell.service', 'Win11 Shell', 'active', 'running'],
  ['gpg-agent.service', 'GnuPG: sleutels onthouden', 'inactive', 'dead'],
  ['filter-chain.service', 'PipeWire: geluidsfilters', 'inactive', 'dead'],
  ['obex.service', 'Bluetooth-bestandsoverdracht', 'inactive', 'dead'],
  ['speech-dispatcher.service', 'Spraakuitvoer', 'inactive', 'dead'],
  ['snap.snapd-desktop-integration.snapd-desktop-integration.service', 'Snap-integratie met het bureaublad', 'active', 'running'],
].map(([name, description, state, sub]) => ({
  name, description, state, sub, scope: 'user', load: 'loaded', enabled: state === 'active' ? 'enabled' : 'disabled',
  pid: null, critical: /^(dbus|pipewire|pipewire-pulse|wireplumber|xdg-desktop-portal|xdg-desktop-portal-hyprland|xdg-document-portal|xdg-permission-store|win11shell)\.service$/.test(name),
  displayName: name.replace(/\.service$/, ''),
}));

/* ---------------------------------------------------------------- methods */

const err = (code, message) => Object.assign(new Error(message), { code });

function findApp(pid) { return apps.find((a) => a.pids.has(pid)); }

/** A fresh copy of a process with a new pid (what a restart looks like from outside). */
function respawn(p, ppid = p.ppid) {
  const q = mkProc({
    name: p.name, displayName: p.displayName, user: p.user, cpu: p.baseCpu, mem: p.baseMem, threads: p.threads, cmd: p.cmd,
    group: p.group, groupKey: p.groupKey, canKill: p.canKill, disk: p.baseDisk, ppid, started: Date.now(),
  });
  q.icon = p.icon; q.critical = p.critical; q.exe = p.exe; q.tty = p.tty;
  procs.delete(p.pid);
  return q;
}

async function restartPlan({ pid } = {}) {
  const p = procs.get(Number(pid));
  if (!p) throw err('not_found', 'Dit proces bestaat niet meer.');
  if (p.groupKey === 'win11shell') return { pid: p.pid, how: 'shell', name: 'Win11 Shell' };
  if (p.critical || p.kernel) throw err('protected', 'Dit proces is nodig voor je bureaublad en kan niet opnieuw worden gestart.');
  if (!p.canKill || p.user !== 'robin') throw err('protected', 'Dit proces is van een andere gebruiker of van het systeem.');
  const app = findApp(p.pid);
  if (app) {
    const root = [...app.pids].map((x) => procs.get(x)).find((x) => x && !app.pids.has(x.ppid));
    if (!(p.tty && root && root.pid !== p.pid)) {
      return { pid: p.pid, how: 'app', name: app.def.name, desktopId: app.def.desktopId, count: app.pids.size,
        ...(root && root.pid !== p.pid ? { via: p.displayName || p.name } : {}) };
    }
  }
  if (String(p.groupKey).startsWith('unit:')) return { pid: p.pid, how: 'service', name: p.displayName || p.name, unit: p.groupKey.slice(5) };
  return { pid: p.pid, how: 'exec', name: p.displayName || p.name, tty: !!p.tty };
}

async function restartProcess({ pid, confirm } = {}) {
  const p = procs.get(Number(pid));
  if (!p) throw err('not_found', 'Dit proces bestaat niet meer.');
  if (p.groupKey === 'win11shell') {
    setTimeout(() => hostGate.restart(), 80);
    return { ok: true, how: 'shell', dryRun: true };
  }
  if (p.critical || p.kernel) return { ok: false, how: 'exec', error: 'Dit proces is nodig voor je bureaublad en kan niet opnieuw worden gestart.' };
  if (!p.canKill || p.user !== 'robin') return { ok: false, how: 'exec', error: 'Je hebt geen toestemming om dit proces opnieuw te starten.' };
  await new Promise((r) => setTimeout(r, 700));
  const plan = await restartPlan({ pid });
  const app = plan.how === 'app' ? findApp(p.pid) : null;
  if (app) {
    // the whole app restarts: its processes come back with new pids, and it responds again
    const old = [...app.pids].map((x) => procs.get(x)).filter(Boolean);
    const root = old.find((x) => !app.pids.has(x.ppid)) || old[0];
    app.pids.clear();
    const main = respawn(root, root.ppid);
    if (root.state !== 'D') main.state = root.state;
    app.pids.add(main.pid);
    for (const k of old) if (k !== root) app.pids.add(respawn(k, main.pid).pid);
    app.status = 'running';
    return { ok: true, how: 'app', dryRun: true };
  }
  if (String(p.groupKey).startsWith('unit:')) {
    for (const k of [...procs.values()]) if (k.groupKey === p.groupKey) respawn(k);
    return { ok: true, how: 'service', dryRun: true };
  }
  if (p.tty && confirm !== true) {
    throw err('confirm_required', 'Dit proces draait in een terminal. Het wordt opnieuw gestart zonder terminalvenster: '
      + 'je ziet zijn uitvoer dan niet meer en kunt niets meer typen. Toch opnieuw starten?');
  }
  const owner = findApp(p.pid);         // a program in a terminal app's tab: only it restarts, it stays in the group
  const q = respawn(p);
  if (owner) { owner.pids.delete(p.pid); owner.pids.add(q.pid); }
  return { ok: true, how: 'exec', dryRun: true };
}

export default {
  methods: hostGate.wrap({
    'sysmon.restartProcess': restartProcess,
    'sysmon.restartPlan': restartPlan,

    'sysmon.processes': async () => processesSnapshot(),

    'sysmon.performance': async () => performanceSnapshot(),

    'sysmon.endTask': async ({ pids = [], force = false } = {}) => {
      const results = [];
      for (const raw of pids) {
        const pid = Number(raw);
        const p = procs.get(pid);
        if (!p) { results.push({ pid, ok: false, error: 'Dit proces bestaat niet meer.' }); continue; }
        if (!p.canKill) { results.push({ pid, ok: false, error: 'Je hebt geen toestemming om dit proces te beëindigen.' }); continue; }
        const app = findApp(pid);
        // A hanging app ignores the friendly request (SIGTERM); only force works.
        if (app && app.status === 'not-responding' && !force) { results.push({ pid, ok: true }); continue; }
        procs.delete(pid);
        if (app) app.pids.delete(pid);
        results.push({ pid, ok: true });
      }
      // children of a killed main process go away too
      for (const p of [...procs.values()]) {
        if (p.ppid > 1 && !procs.has(p.ppid) && p.group === 'app') { procs.delete(p.pid); findApp(p.pid)?.pids.delete(p.pid); }
      }
      return { results, dryRun: true };
    },

    'sysmon.setEfficiency': async ({ pids = [], enabled = true } = {}) => {
      const results = pids.map((raw) => {
        const pid = Number(raw);
        const p = procs.get(pid);
        if (!p) return { pid, ok: false, error: 'Dit proces bestaat niet meer.' };
        if (p.user !== 'robin') return { pid, ok: false, error: 'Je hebt geen toestemming om dit proces aan te passen.' };
        p.efficiency = !!enabled;
        return { pid, ok: true };
      });
      return { results, dryRun: true };
    },

    'sysmon.startup': async () => STARTUP.map((s) => ({
      ...s, description: s.description || '', systemDefault: s.source === 'system',
      path: `${s.source === 'system' ? '/etc/xdg/autostart' : '/home/robin/.config/autostart'}/${s.id}`, hidden: false,
    })),

    'sysmon.setStartup': async ({ id, enabled } = {}) => {
      const s = STARTUP.find((x) => x.id === id);
      if (!s) throw err('not_found', 'Deze opstart-app bestaat niet meer.');
      s.enabled = !!enabled;
      return { ok: true, dryRun: true };
    },

    'sysmon.services': async () => SERVICES.map((s) => ({ ...s })),

    'sysmon.serviceAction': async ({ name, action } = {}) => {
      const s = SERVICES.find((x) => x.name === name);
      if (!s) throw err('not_found', 'Deze service bestaat niet.');
      if (!['start', 'stop', 'restart'].includes(action)) throw err('bad_request', 'Onbekende actie.');
      if (s.critical && action !== 'start') throw err('protected', 'Deze service is nodig voor je bureaublad en kan niet worden gestopt.');
      await new Promise((r) => setTimeout(r, 400));
      if (action === 'stop') { s.state = 'inactive'; s.sub = 'dead'; } else { s.state = 'active'; s.sub = 'running'; }
      return { ok: true, dryRun: true };
    },

    'sysmon.health': async (args, m) => {
      // mirrors host/services/sysmon.py health(): one entry per check, wording + action params identical,
      // severity 'ok' rows included, sorted critical → warning → info → ok
      step();
      const en = (m.settings?.()?.language || 'nl') === 'en';
      const T = en ? HEALTH_EN : HEALTH_NL;
      const fmt = (k, o = {}) => T[k].replace(/\{(\w+)\}/g, (_, n) => (o[n] ?? ''));
      const items = [];
      let vols = [];
      try { vols = await m.call('storage.volumes', {}); } catch { vols = []; }
      const diskItems = [];
      for (const v of vols) {
        if (!v.total || v.readOnly || v.hidden) continue;
        const pct = (v.used * 100) / Math.max(1, v.used + v.free);
        if (pct < 90) continue;
        const crit = pct >= 97 || (v.isSystem && v.free < 2 * GB);
        diskItems.push({
          id: `disk:${v.id}`, severity: crit ? 'critical' : 'warning',
          title: fmt(crit ? 'diskCritT' : 'diskWarnT', { name: v.name || v.mount }),
          message: fmt(crit ? 'diskCritM' : 'diskWarnM', { free: fmtGB(v.free), total: fmtGB(v.total), pct: Math.round(pct) }),
          action: { label: T.aFree, app: 'taskmgr', params: { page: 'storage', volume: v.id } },
        });
      }
      items.push(...(diskItems.length ? diskItems : [{ id: 'disks', severity: 'ok', title: T.diskOkT, message: T.diskOkM, action: null }]));
      const memAction = { label: T.aTaskmgr, app: 'taskmgr', params: { page: 'processes', sort: 'memory' } };
      const avail = MEM_TOTAL - sim.memUsed;
      const frac = avail / MEM_TOTAL;
      if (frac < 0.05) items.push({ id: 'memory', severity: 'critical', title: T.memCritT, message: fmt('memCritM', { avail: fmtGB(avail) }), action: memAction });
      else if (frac < 0.12) items.push({ id: 'memory', severity: 'warning', title: T.memWarnT, message: fmt('memWarnM', { avail: fmtGB(avail), total: fmtGB(MEM_TOTAL) }), action: memAction });
      else items.push({ id: 'memory', severity: 'ok', title: T.memOkT, message: fmt('memOkM', { avail: fmtGB(avail), total: fmtGB(MEM_TOTAL) }), action: null });
      const su = sim.swapUsed;
      if (su / SWAP_TOTAL >= 0.5 && su > 256 * MB) {
        items.push({ id: 'swap', severity: su / SWAP_TOTAL >= 0.8 ? 'warning' : 'info', title: T.swapWarnT, message: fmt('swapWarnM', { used: fmtGB(su), total: fmtGB(SWAP_TOTAL) }), action: memAction });
      } else items.push({ id: 'swap', severity: 'ok', title: T.swapOkT, message: T.swapOkM, action: null });
      items.push({ id: 'cpu', severity: 'ok', title: T.cpuOkT, message: T.cpuOkM, action: null });
      const temp = Math.round(sim.temp * 10) / 10;
      if (temp >= 85) {
        items.push({ id: 'temp:cpu', severity: temp >= 95 ? 'critical' : 'warning', title: fmt('tempWarnT', { what: T.cpuWord }), message: fmt('tempWarnM', { temp: Math.round(temp) }), action: { label: T.aTaskmgr, app: 'taskmgr', params: { page: 'performance' } } });
      } else items.push({ id: 'temperature', severity: 'ok', title: T.tempOkT, message: fmt('tempOkM', { temp: Math.round(temp) }), action: null, temperature: temp });
      const failed = SERVICES.filter((sv) => sv.state === 'failed');
      if (failed.length) {
        items.push({ id: 'services', severity: 'warning', title: fmt(failed.length === 1 ? 'svcWarnT' : 'svcWarnTpl', { n: failed.length }),
          message: fmt('svcWarnM', { names: failed.slice(0, 4).map((sv) => sv.description || sv.name).join(', ') }),
          action: { label: T.aServices, app: 'taskmgr', params: { page: 'services' } }, units: failed.map((sv) => sv.name) });
      } else items.push({ id: 'services', severity: 'ok', title: T.svcOkT, message: T.svcOkM, action: null });
      items.push({ id: 'updates', severity: 'info', title: T.updInfoT, message: fmt('updInfoM', { n: 7 }) + fmt('updSecM', { s: 2 }),
        action: { label: T.aUpdates, app: 'settings', params: { page: 'update' }, method: 'system.openUpdater' } });
      const trash = 3.1 * GB;
      items.push({ id: 'trash', severity: 'info', title: T.trashInfoT, message: fmt('trashInfoM', { size: fmtGB(trash) }),
        action: { label: T.aFree, app: 'taskmgr', params: { page: 'storage' } }, size: Math.round(trash) });
      const SEV = { critical: 0, warning: 1, info: 2, ok: 3 };
      return items.sort((a, b) => SEV[a.severity] - SEV[b.severity]);
    },
  }),
  start() {},
};

const HEALTH_NL = {
  diskOkT: 'Opslag', diskOkM: 'Er is op al je schijven genoeg ruimte vrij.',
  diskWarnT: '{name} is bijna vol', diskWarnM: 'Er is nog {free} vrij van {total} ({pct}% gebruikt). Ruim bestanden op die je niet meer nodig hebt.',
  diskCritT: '{name} is vol', diskCritM: 'Er is nog maar {free} vrij. Apps en updates kunnen hierdoor vastlopen. Maak zo snel mogelijk ruimte vrij.',
  aFree: 'Ruimte vrijmaken', aTaskmgr: 'Taakbeheer openen', aServices: 'Services bekijken', aUpdates: 'Updates installeren',
  memOkT: 'Geheugen', memOkM: 'Er is genoeg geheugen vrij ({avail} van {total}).',
  memWarnT: 'Weinig geheugen vrij', memWarnM: 'Er is nog {avail} van {total} vrij. Sluit apps die je niet gebruikt, dan blijft alles vlot.',
  memCritT: 'Het geheugen is bijna op', memCritM: 'Er is nog maar {avail} vrij. De computer kan traag worden of apps sluiten. Sluit een paar apps.',
  swapOkT: 'Wisselgeheugen', swapOkM: 'Het wisselgeheugen wordt nauwelijks gebruikt.',
  swapWarnT: 'Veel wisselgeheugen in gebruik', swapWarnM: '{used} van {total} wisselgeheugen is in gebruik. Daardoor kan de computer trager aanvoelen.',
  cpuOkT: 'Processor', cpuOkM: 'Geen enkele app gebruikt langdurig veel rekenkracht.',
  tempOkT: 'Temperatuur', tempOkM: 'De processor is lekker koel ({temp} °C).', cpuWord: 'De processor',
  tempWarnT: '{what} wordt warm', tempWarnM: 'De temperatuur is {temp} °C. Zorg dat de ventilatieopeningen vrij zijn.',
  svcOkT: 'Achtergronddiensten', svcOkM: 'Alle achtergronddiensten werken.',
  svcWarnT: '{n} achtergronddienst werkt niet', svcWarnTpl: '{n} achtergronddiensten werken niet', svcWarnM: 'Niet gestart: {names}.',
  updInfoT: 'Er zijn updates beschikbaar', updInfoM: 'Er zijn {n} updates klaar om te installeren.', updSecM: ' Daarvan {s} voor de beveiliging.',
  trashInfoT: 'De prullenbak is groot', trashInfoM: 'Er staat {size} in de prullenbak. Leeg hem om ruimte vrij te maken.',
};
const HEALTH_EN = {
  diskOkT: 'Storage', diskOkM: 'There is enough free space on all your drives.',
  diskWarnT: '{name} is almost full', diskWarnM: '{free} free of {total} ({pct}% used). Clean up files you no longer need.',
  diskCritT: '{name} is full', diskCritM: 'Only {free} left. Apps and updates may get stuck. Free up space as soon as you can.',
  aFree: 'Free up space', aTaskmgr: 'Open Task Manager', aServices: 'View services', aUpdates: 'Install updates',
  memOkT: 'Memory', memOkM: 'There is enough free memory ({avail} of {total}).',
  memWarnT: 'Low on memory', memWarnM: '{avail} of {total} free. Close apps you are not using to keep things smooth.',
  memCritT: 'Memory is almost full', memCritM: 'Only {avail} left. The computer may slow down or apps may close. Close a few apps.',
  swapOkT: 'Swap', swapOkM: 'Swap is hardly used.',
  swapWarnT: 'A lot of swap in use', swapWarnM: '{used} of {total} swap is in use. The computer may feel slower.',
  cpuOkT: 'Processor', cpuOkM: 'No app has been using a lot of processing power for long.',
  tempOkT: 'Temperature', tempOkM: 'The processor is nice and cool ({temp} °C).', cpuWord: 'The processor',
  tempWarnT: '{what} is getting warm', tempWarnM: 'The temperature is {temp} °C. Make sure the vents are not blocked.',
  svcOkT: 'Background services', svcOkM: 'All background services are working.',
  svcWarnT: '{n} background service is not working', svcWarnTpl: '{n} background services are not working', svcWarnM: 'Not started: {names}.',
  updInfoT: 'Updates are available', updInfoM: '{n} updates are ready to install.', updSecM: ' {s} of them are security updates.',
  trashInfoT: 'The Recycle Bin is large', trashInfoM: 'There is {size} in the Recycle Bin. Empty it to free up space.',
};

function fmtGB(n) {
  const v = n / GB;
  return `${v >= 100 ? Math.round(v) : v.toFixed(1).replace('.', ',')} GB`;
}
