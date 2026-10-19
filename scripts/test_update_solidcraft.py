"""Exercise upgrade transactions without network requests or compiling fixtures."""
import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch

import update_solidcraft as update

GIT = "https://github.com/example/solidcraft.git"


class UpgradeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.addCleanup(patch.stopall)
        patch.object(update, "ROOT", self.root).start()
        (self.root / "Cargo.toml").write_text(f'''[package]
name = "test"
version = "0.1.0"
[package.metadata.solidcraft]
minimum-version = "1.2.0"
[dependencies]
solidcraft-engine = {{ git = "{GIT}", tag = "v1.2.3" }}
solidcraft-mcp = {{ git = "{GIT}", tag = "v1.2.3", default-features = false }}
serde = "1"
[patch.crates-io]
old-kernel = {{ git = "{GIT}", tag = "v1.2.3" }}
local-helper = {{ path = "../helper" }}
[profile.dev]
debug = 0
''')
        (self.root / "Cargo.lock").write_bytes(b"original lock\n")
        self.old = {"version": "1.2.3", "minimum_version": "1.2.0", "file_format": 10}
        self.new = {"version": "1.3.1", "minimum_version": "1.3.0", "file_format": 11}
        (self.root / "README.md").write_text("Before\n" + update.compatibility_block(self.old) + "\nAfter\n")
        (self.root / "CHANGELOG.md").write_text("# Changelog\n\n## Unreleased\n\n- Existing change.\n")
        self.originals = {name: (self.root / name).read_bytes() for name in update.FILES}
        self.commands = []

    def command(self, command):
        self.commands.append(command)
        if command[0] == "gh":
            if command[2].endswith("/releases/latest"):
                return json.dumps({"tag_name": "v1.3.1"})
            return '''[workspace.package]
version = "1.3.1"
[patch.crates-io]
truck-shapeops = { path = "vendor/truck-shapeops" }
new-kernel = { path = "vendor/new-kernel" }
egui-wgpu = { path = "vendor/egui-wgpu" }
'''
        if command[:2] == ["cargo", "metadata"]:
            (self.root / "Cargo.lock").write_text("resolved lock\n")
            return json.dumps({"packages": [{"name": name} for name in
                              ("solidcraft-engine", "solidcraft-mcp", "truck-shapeops", "new-kernel")]})
        return ""

    def upgrade(self, minimum="1.3.0", validate=False, tag=None):
        with patch.object(update, "run", side_effect=self.command), \
             patch.object(update, "engine_info", side_effect=[self.old, self.new]), \
             contextlib.redirect_stdout(io.StringIO()):
            update.upgrade(tag, minimum, validate)

    def assert_restored(self):
        self.assertEqual(self.originals,
                         {name: (self.root / name).read_bytes() for name in update.FILES})

    def test_latest_upgrade_discovers_new_kernel_patches_and_preserves_other_settings(self):
        self.upgrade(validate=True)
        manifest = tomllib.loads((self.root / "Cargo.toml").read_text())
        self.assertEqual(manifest["dependencies"]["solidcraft-engine"]["tag"], "v1.3.1")
        self.assertEqual(manifest["dependencies"]["solidcraft-mcp"]["tag"], "v1.3.1")
        self.assertFalse(manifest["dependencies"]["solidcraft-mcp"]["default-features"])
        self.assertEqual(manifest["dependencies"]["serde"], "1")
        self.assertEqual(set(manifest["patch"]["crates-io"]),
                         {"truck-shapeops", "new-kernel", "local-helper"})
        self.assertEqual(manifest["patch"]["crates-io"]["local-helper"], {"path": "../helper"})
        self.assertIn(update.compatibility_block(self.new), (self.root / "README.md").read_text())
        self.assertIn("`v1.3.1`", (self.root / "CHANGELOG.md").read_text())
        self.assertIn("- Existing change.", (self.root / "CHANGELOG.md").read_text())
        self.assertIn(["cargo", "test", "--locked"], self.commands)

    def test_format_change_requires_an_explicit_minimum_and_restores_files(self):
        self.new["minimum_version"] = self.old["minimum_version"]
        with self.assertRaisesRegex(ValueError, "native format changed"):
            self.upgrade(minimum=None)
        self.assert_restored()

    def test_same_format_preserves_minimum_and_handles_explicit_tags(self):
        self.new.update(file_format=10, minimum_version="1.2.0")
        self.upgrade(minimum=None, tag="v1.3.1")
        manifest = tomllib.loads((self.root / "Cargo.toml").read_text())
        self.assertEqual(manifest["package"]["metadata"]["solidcraft"]["minimum-version"], "1.2.0")
        self.assertFalse(any(command[-1].endswith("/releases/latest") for command in self.commands))

    def test_format_change_cannot_reuse_the_old_minimum(self):
        self.new["minimum_version"] = self.old["minimum_version"]
        with self.assertRaisesRegex(ValueError, "native format changed"):
            self.upgrade(minimum="1.2.0")
        self.assert_restored()

    def test_mislabeled_upstream_tag_is_rejected_without_changes(self):
        original = self.command

        def mislabeled(command):
            result = original(command)
            if command[0] == "gh" and "/contents/" in command[2]:
                return result.replace('version = "1.3.1"', 'version = "1.3.0"')
            return result

        self.command = mislabeled
        with self.assertRaisesRegex(ValueError, "release tag does not match"):
            self.upgrade()
        self.assert_restored()

    def test_failed_validation_restores_all_files(self):
        original = self.command

        def failed(command):
            if command[:2] == ["cargo", "fmt"]:
                raise subprocess.CalledProcessError(1, command, stderr="formatting failed")
            return original(command)

        self.command = failed
        with self.assertRaises(subprocess.CalledProcessError):
            self.upgrade(validate=True)
        self.assert_restored()

    def test_interrupted_upgrade_restores_all_files(self):
        with patch.object(update, "run", side_effect=self.command), \
             patch.object(update, "engine_info", side_effect=[self.old, KeyboardInterrupt()]):
            with self.assertRaises(KeyboardInterrupt):
                update.upgrade("v1.3.1", "1.3.0")
        self.assert_restored()

    def test_invalid_target_is_rejected_without_changes(self):
        with patch.object(update, "run", side_effect=self.command):
            with self.assertRaises(ValueError):
                update.upgrade("v1.3.1-beta.1")
        self.assert_restored()

    def test_newer_minimum_is_rejected_without_changes(self):
        with self.assertRaisesRegex(ValueError, "minimum desktop version cannot be newer"):
            self.upgrade(minimum="2.0.0")
        self.assert_restored()


if __name__ == "__main__":
    unittest.main()
