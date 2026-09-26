"""Linux first-run checks: setup runs without Tk and never downloads an extractor."""
import argparse
import contextlib
import io
import os
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.asset_pipeline import install as engine
from tools.asset_pipeline import customiser_setup
import tools.setup as setup


class LinuxSetup(unittest.TestCase):
    @unittest.skipIf(os.name == 'nt', 'Windows keeps the pinned extract-xiso path')
    def test_iso_input_guides_to_a_pre_extracted_copy_without_downloading(self):
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)/'data'; iso = Path(temp)/'game.iso'; iso.write_bytes(b'iso')
            with patch.object(engine, 'dependency') as dependency:
                with self.assertRaisesRegex(RuntimeError, 'default.xex'):
                    engine.install(iso, base, Path('unused.exe'), lambda _: None)
                dependency.assert_not_called()
            self.assertFalse((base/'installations').exists())

    @unittest.skipIf(os.name == 'nt', 'Windows keeps the pinned extract-xiso path')
    def test_iso_through_setup_entry_never_downloads_an_extractor(self):
        # customiser_setup.install is the real entry point used by setup.py; it
        # extracts ISOs itself, so the guard must run before its workspace.
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)/'data'; iso = Path(temp)/'game.iso'; iso.write_bytes(b'iso')
            with patch.object(engine, 'dependency') as dependency:
                with self.assertRaisesRegex(RuntimeError, 'default.xex'):
                    customiser_setup.install(iso, base, Path('unused.exe'), lambda _: None)
                dependency.assert_not_called()
            self.assertFalse(base.exists())

    @unittest.skipIf(os.name == 'nt', 'Windows keeps the pinned extract-xiso path')
    def test_headless_iso_input_reports_guidance_without_downloading(self):
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)/'data'; iso = Path(temp)/'game.iso'; iso.write_bytes(b'iso')
            args = argparse.Namespace(base=base, game_exe=Path('unused.exe'), refresh=False, source=iso)
            buffer = io.StringIO()
            with patch.object(engine, 'dependency') as dependency, contextlib.redirect_stdout(buffer):
                self.assertEqual(setup.headless(args), 2)
            dependency.assert_not_called()
            self.assertIn('default.xex', buffer.getvalue())

    def test_headless_rejects_a_missing_source(self):
        with tempfile.TemporaryDirectory() as temp:
            args = argparse.Namespace(base=Path(temp)/'data', game_exe=Path('unused.exe'),
                                      refresh=False, source=Path(temp)/'missing.xex')
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                self.assertEqual(setup.headless(args), 2)
            self.assertIn('No such file', buffer.getvalue())

    def test_headless_without_a_terminal_reports_how_to_pass_the_source(self):
        args = argparse.Namespace(base=Path('/unused'), game_exe=Path('unused.exe'),
                                  refresh=False, source=None)
        for stdin in (io.StringIO(''), None):
            buffer = io.StringIO()
            with patch.object(sys, 'stdin', stdin), contextlib.redirect_stdout(buffer):
                self.assertEqual(setup.headless(args), 2)
            self.assertIn('--source', buffer.getvalue())

    @unittest.skipIf(os.name == 'nt', 'Windows reports display errors rather than falling back')
    def test_missing_display_falls_back_to_headless(self):
        class NoDisplay(Exception): pass
        tk = types.ModuleType('tkinter')
        tk.TclError = NoDisplay
        tk.Tk = Mock(side_effect=NoDisplay)
        tk.filedialog = tk.messagebox = tk.ttk = Mock()
        with patch.dict(sys.modules, {'tkinter': tk}), \
             patch.object(sys, 'argv', ['setup.py', '--base', '/unused', '--game-exe', '/unused']), \
             patch.object(setup, 'headless', return_value=2) as headless:
            self.assertEqual(setup.main(), 2)
            headless.assert_called_once()


if __name__ == '__main__':
    unittest.main()
