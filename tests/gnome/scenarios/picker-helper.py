"""Helper for r2d-picker.py (not a scenario): a GTK 4 app of somebody else that asks for a file.

It shows one window titled PickerTest; each time the trigger file <state dir>/picker-helper.open appears it opens
gtk::FileDialog (the desktop's FileChooser portal, so Win11's picker when the portal configuration chooses it) for
that window, modal or not. Each answer is one JSON line in <state dir>/picker-helper.jsonl: {"pid", "modal",
"path"} (path null: cancelled) or {"pid", "modal", "error"}.

    python3 picker-helper.py <state dir> modal|nonmodal
"""
import json
import os
import sys

import gi

gi.require_version('Gtk', '4.0')
from gi.repository import GLib  # noqa: E402

APP_ID = 'org.example.PickerTest'
GLib.set_prgname(APP_ID)          # before GTK initializes: the Wayland app id, so GNOME maps the window to the app
from gi.repository import Gio, Gtk  # noqa: E402
STATE = sys.argv[1]
MODAL = sys.argv[2] == 'modal'
ANSWERS = os.path.join(STATE, 'picker-helper.jsonl')
TRIGGER = os.path.join(STATE, 'picker-helper.open')


def answer(**row):
    with open(ANSWERS, 'a') as f:
        f.write(json.dumps({'pid': os.getpid(), 'modal': MODAL, **row}) + '\n')


def done(dialog, result):
    try:
        file = dialog.open_finish(result)
        answer(path=file.get_path() if file else None)
    except GLib.Error as error:
        if error.matches(Gtk.DialogError.quark(), Gtk.DialogError.DISMISSED) or \
                error.matches(Gtk.DialogError.quark(), Gtk.DialogError.CANCELLED):
            answer(path=None)
        else:
            answer(error=error.message)


def ask(win):
    dialog = Gtk.FileDialog(title='Openen', modal=MODAL)
    dialog.open(win, None, done)


def activate(app):
    win = Gtk.ApplicationWindow(application=app, title='PickerTest')
    win.set_default_size(480, 300)
    win.set_child(Gtk.Label(label='picker-helper'))
    win.present()

    def poll():
        if os.path.exists(TRIGGER):
            os.unlink(TRIGGER)
            ask(win)
        return True
    GLib.timeout_add(100, poll)


app = Gtk.Application(application_id=APP_ID, flags=Gio.ApplicationFlags.NON_UNIQUE)
app.connect('activate', activate)
app.run([sys.argv[0]])
