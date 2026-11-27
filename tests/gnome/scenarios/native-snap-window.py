"""A small resizable GTK4 window for native-snap.py; never a scenario itself."""
import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gio, Gtk

app = Gtk.Application(application_id='org.example.WinPointNativeSnap', flags=Gio.ApplicationFlags.NON_UNIQUE)

def activate(application):
    window = Gtk.ApplicationWindow(application=application, title='Native snap regression')
    window.set_default_size(420, 260)
    window.set_child(Gtk.Label(label='Super+Left / Super+Right native Mutter regression'))
    window.present()

app.connect('activate', activate)
app.run([])
