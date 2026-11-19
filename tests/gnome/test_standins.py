"""Test embedded reply handling without opening a bus or starting a process."""
import ast
import io
from pathlib import Path
import sys
import unittest
from gi.repository import GLib

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lib.standins import SERVER
from lib.standins.e import LIVE_SERVER
from lib.standins.d import power_profiles


class Invocation:
    def get_message(self):
        return self

    def get_flags(self):
        return 0

    def return_value(self, value):
        self.value = value


class Replies(unittest.TestCase):
    def test_power_profile_properties_use_the_shared_variant_format(self):
        tree = ast.parse(SERVER)
        conv = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'conv')
        ns = {'GLib': GLib}
        exec(compile(ast.Module(body=[conv], type_ignores=[]), '<standin>', 'exec'), ns)
        spec = power_profiles()
        props = next(iter(next(iter(spec.objects.values())).values()))['props']
        sig, value = props['Profiles']
        variant = GLib.Variant(sig, ns['conv'](value))
        self.assertEqual([v['Profile'] for v in variant.unpack()], ['power-saver', 'balanced', 'performance'])
        sig, value = props['ActiveProfileHolds']
        self.assertEqual(GLib.Variant(sig, ns['conv'](value)).unpack(), [])

    def test_variant_replies_can_depend_on_arguments_in_both_servers(self):
        for name, source in (('shared', SERVER), ('live', LIVE_SERVER)):
            with self.subTest(server=name):
                tree = ast.parse(source)
                functions = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in ('conv', 'handler')]
                ns = {'GLib': GLib, 'FLAG': 0, 'log': io.StringIO(), 'json': __import__('json'),
                      'note': lambda *args: None}
                exec(compile(ast.Module(body=functions, type_ignores=[]), '<standin>', 'exec'), ns)
                desc = {'methods': {'Probe': {'out': 'v', 'value': [{'$v': ['s', 'default']}],
                                              'byArgs': {"('chosen',)": [{'$v': ['s', 'selected']}]}}}}
                call, _, _ = ns['handler']('/probe', 'io.test.Probe', desc)
                for arg, expected in (('chosen', 'selected'), ('other', 'default')):
                    reply = Invocation()
                    call(None, None, '/probe', 'io.test.Probe', 'Probe', GLib.Variant('(s)', (arg,)), reply)
                    self.assertEqual(reply.value.get_child_value(0).get_variant().get_string(), expected)


if __name__ == '__main__':
    unittest.main()
