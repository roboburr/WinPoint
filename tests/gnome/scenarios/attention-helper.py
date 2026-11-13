"""Helper for launch-attention.py (not a scenario): an app that runs once, like Claude, WhatsApp or Steam.

The first process shows a window. A second launch only hands over (it touches TRIGGER and exits); the first then
presents its window itself, without the activation token the launch carried, as those apps do. On GNOME that is
the request that ends as "“…” is ready" instead of the window.

    python3 attention-helper.py <state dir>
"""
import os
import sys

import gi

gi.require_version('Gtk', '3.0')
from gi.repository import GLib  # noqa: E402

APP_ID = 'org.example.AttentionTest'
GLib.set_prgname(APP_ID)          # before GTK initializes: the Wayland app id, so GNOME maps the window to the app
from gi.repository import Gtk  # noqa: E402
STATE = sys.argv[1]
PIDFILE = os.path.join(STATE, 'attention-helper.pid')
TRIGGER = os.path.join(STATE, 'attention-helper.present')


def running():
    try:
        with open(PIDFILE) as f:
            pid = int(f.read().strip())
        os.kill(pid, 0)
        return pid != os.getpid()
    except (OSError, ValueError):
        return False


if running():
    open(TRIGGER, 'w').close()
    sys.exit(0)
with open(PIDFILE, 'w') as f:
    f.write(str(os.getpid()))
win = Gtk.Window(title='AttentionTest')
win.set_default_size(420, 260)
win.add(Gtk.Label(label='attention-helper'))
win.connect('destroy', Gtk.main_quit)
win.show_all()


def poll():
    if os.path.exists(TRIGGER):
        os.unlink(TRIGGER)
        win.present()
    return True


GLib.timeout_add(100, poll)
Gtk.main()
