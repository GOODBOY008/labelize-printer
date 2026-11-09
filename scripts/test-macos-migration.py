"""Exercise installer migration against disposable volumes, never /Applications."""
import os
import plistlib
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1] / "platform/macos/installer-scripts"


@unittest.skipUnless(os.uname().sysname == "Darwin", "macOS installer utilities required")
class Migration(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="studio migration ")
        self.addCleanup(self.temporary.cleanup)
        self.volume = Path(self.temporary.name)
        self.data = self.volume / "Users/test/Library/Application Support/LabelizePrinter"
        self.data.mkdir(parents=True)
        (self.data / "workspace.json").write_text('{"draft":"keep"}')

    def bundle(self, name, identifier="com.labelize.printer"):
        app = self.volume / "Applications" / (name + ".app")
        (app / "Contents/MacOS").mkdir(parents=True)
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": identifier}))
        executable = app / "Contents/MacOS/labelize-printer"
        executable.write_text("#!/bin/sh\nexit 0\n")
        executable.chmod(0o755)
        return app

    def run_script(self, name):
        return subprocess.run([str(SCRIPTS / name), "package", "/Applications", str(self.volume)],
                              capture_output=True, text=True).returncode

    def test_upgrade_removes_only_old_bundle_and_preserves_data(self):
        old = self.bundle("Labelize Printer")
        new = self.bundle("Labelize Studio")
        self.assertEqual(self.run_script("preinstall"), 0)
        self.assertEqual(self.run_script("postinstall"), 0)
        self.assertFalse(old.exists())
        self.assertTrue(new.exists())
        self.assertEqual((self.data / "workspace.json").read_text(), '{"draft":"keep"}')
        self.assertEqual(self.run_script("postinstall"), 0)

    def test_failed_payload_keeps_legacy_application(self):
        old = self.bundle("Labelize Printer")
        self.assertNotEqual(self.run_script("postinstall"), 0)
        self.assertTrue(old.exists())

    def test_unrelated_bundle_and_symlinks_are_not_overwritten(self):
        foreign = self.bundle("Labelize Studio", "com.example.unrelated")
        self.assertNotEqual(self.run_script("preinstall"), 0)
        legacy = self.volume / "Applications/Labelize Printer.app"
        legacy.symlink_to(foreign, target_is_directory=True)
        self.assertNotEqual(self.run_script("preinstall"), 0)
        self.assertTrue(foreign.exists())

    def test_unrelated_legacy_bundle_is_retained(self):
        old = self.bundle("Labelize Printer", "com.example.unrelated")
        self.bundle("Labelize Studio")
        self.assertEqual(self.run_script("postinstall"), 0)
        self.assertTrue(old.exists())


if __name__ == "__main__":
    unittest.main()
