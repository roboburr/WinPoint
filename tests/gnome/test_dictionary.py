"""Public dictionary exposure stays read-only and excludes links and non-files. No services are started."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('private_gnome_harness', Path(__file__).with_name('harness.py'))
harness = importlib.util.module_from_spec(spec)
import sys
sys.modules[spec.name] = harness
spec.loader.exec_module(harness)


class DictionaryBoundary(unittest.TestCase):
    def test_only_regular_public_data_is_mounted_read_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / 'dictionary.pwd'
            data.write_text('test dictionary')
            link = root / 'dictionary.pwi'
            link.symlink_to(data)
            directory = root / 'dictionary.hwm'
            directory.mkdir()
            self.assertEqual(harness.readable_dictionary_files((data, link, directory, root / 'absent')), [data])
            with patch.object(harness, 'readable_dictionary_files', return_value=[data]):
                args, visible = harness.root_argv()
            triples = [args[i:i+3] for i in range(len(args)-2)]
            self.assertIn(['--ro-bind', str(data), str(data)], triples)
            self.assertNotIn(['--bind', str(data), str(data)], triples)
            self.assertIn(str(data), visible)


if __name__ == '__main__':
    unittest.main()
