#!/usr/bin/env python3
"""Runs test_installer.py and test_extension_text.py together (tests/installer/run.sh)."""
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import test_installer  # noqa: E402
import test_extension_text  # noqa: E402

if __name__ == "__main__":
    loader = unittest.TestLoader()
    names = sys.argv[1:]
    modules = (test_installer, test_extension_text)
    if names:
        suite = unittest.TestSuite()
        for name in names:
            by_module = {m.__name__: m for m in modules}
            if name in by_module:
                suite.addTests(loader.loadTestsFromModule(by_module[name]))
                continue
            module = next((m for m in modules if hasattr(m, name.split(".")[0])), None)
            if module is None:
                raise SystemExit(f"no test named {name}")
            suite.addTests(loader.loadTestsFromName(name, module))
    else:
        suite = unittest.TestSuite(loader.loadTestsFromModule(m) for m in modules)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    sys.exit(0 if result.wasSuccessful() else 1)
