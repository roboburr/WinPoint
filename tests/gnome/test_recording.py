"""Pure tests of scenario recording; no display, bus or application is started."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('scenario_recording', ROOT / 'tests/gnome/lib/scenario.py')
scenario = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scenario)


class Recording(unittest.TestCase):
    def test_every_secret_manifest_method_redacts_payloads_without_changing_live_values(self):
        secrets = scenario.secret_methods(ROOT)
        self.assertIn('users.changePassword', secrets)
        for method in secrets:
            with self.subTest(method=method):
                value = {'args': {'surface': 'settings', 'method': method, 'args': {'password': 'private'}},
                         'result': {'password': 'private'}}
                saved = scenario.redact_api(value, secrets)
                self.assertNotIn('private', repr(saved))
                self.assertIn('private', repr(value))
                self.assertEqual(saved['args'], {'surface': 'settings', 'method': method})

    def test_public_calls_and_status_remain_available(self):
        secrets = frozenset({'secret.call'})
        plain = {'method': 'gsettings.get', 'args': {'key': 'x'}, 'result': False}
        self.assertEqual(scenario.redact_api(plain, secrets), plain)
        result = scenario.redact_api({'method': 'secret.call', 'args': 'private', 'result': {'ok': True}}, secrets)
        self.assertEqual(result['args'], '[redacted]')
        self.assertEqual(result['result'], {'ok': True})
        self.assertEqual(scenario.redact_api({'method': []}, secrets), {'method': []})


if __name__ == '__main__':
    unittest.main()
