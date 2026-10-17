"""Check archive contents and release gates without publishing anything."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import release


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        for name, value in (("ROOT", self.root), ("OUTPUT", self.root / "artifacts/release")):
            self.addCleanup(patch.stopall)
            patch.object(release, name, value).start()
        patch.dict(os.environ, {"GITHUB_REF_TYPE": "branch", "GITHUB_SHA": "a" * 40}).start()
        (self.root / "Cargo.toml").write_text('[package]\nname = "solidcraft-kicad"\nversion = "0.1.0"\n')
        (self.root / "CHANGELOG.md").write_text("# Changelog\n\n## 0.1.0\n\nFirst release.\n")
        for name in ("README.md", "LICENSE-MIT", "LICENSE-APACHE", "ATTRIBUTION.md", "Cargo.lock"):
            (self.root / name).write_text(name)
        self.image = self.root / "docs/images/pcb-assembly.png"
        self.image.parent.mkdir(parents=True)
        self.image.write_bytes(b"\x89PNG\r\n\x1a\n")
        self.dependency = self.root / "dependency"
        self.dependency.mkdir()
        (self.dependency / "LICENSE-MIT").write_text("Dependency permission notice")
        for target in release.TARGETS:
            name = "solidcraft-kicad.exe" if target.endswith("windows-msvc") else "solidcraft-kicad"
            binary = self.root / "target" / target / "release" / name
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"native binary fixture")
            binary.chmod(0o755)

    def package(self, target):
        def output(command, **_):
            if command[0] == "cargo":
                return json.dumps({"packages": [{
                    "name": "example", "version": "1.0.0", "source": "registry+example",
                    "manifest_path": str(self.dependency / "Cargo.toml"), "license": "MIT",
                }]})
            return "solidcraft-kicad 0.1.0\n"
        with patch.object(release.subprocess, "check_output", side_effect=output), \
             patch.object(release.subprocess, "run"), contextlib.redirect_stdout(io.StringIO()):
            release.package(target)

    def test_archives_contain_native_binary_docs_image_and_provenance(self):
        for target in release.TARGETS:
            self.package(target)
            base = f"solidcraft-kicad-v0.1.0-{target}"
            if target.endswith("windows-msvc"):
                with zipfile.ZipFile(release.OUTPUT / f"{base}.zip") as archive:
                    names = archive.namelist()
                    info = json.loads(archive.read(f"{base}/BUILD-INFO.json"))
                    self.assertEqual(archive.read(f"{base}/solidcraft-kicad.exe"), b"native binary fixture")
            else:
                with tarfile.open(release.OUTPUT / f"{base}.tar.gz") as archive:
                    names = archive.getnames()
                    info = json.load(archive.extractfile(f"{base}/BUILD-INFO.json"))
                    self.assertEqual(archive.extractfile(f"{base}/solidcraft-kicad").read(), b"native binary fixture")
                    if os.name != "nt":
                        self.assertTrue(archive.getmember(f"{base}/solidcraft-kicad").mode & 0o111)
            self.assertIn(f"{base}/LICENSE-MIT", names)
            self.assertIn(f"{base}/LICENSE-APACHE", names)
            self.assertIn(f"{base}/docs/images/pcb-assembly.png", names)
            self.assertIn(f"{base}/THIRD-PARTY-LICENSES/example-1.0.0/LICENSE-MIT", names)
            self.assertIn(f"{base}/THIRD-PARTY-LICENSES/DEPENDENCIES.json", names)
            self.assertEqual(info, {"version": "0.1.0", "target": target, "commit": "a" * 40})
        release.prepare("v0.1.0")
        lines = (release.OUTPUT / "SHA256SUMS").read_text().splitlines()
        self.assertEqual(len(lines), 3)
        for line in lines:
            digest, name = line.split("  ")
            self.assertEqual(digest, hashlib.sha256((release.OUTPUT / name).read_bytes()).hexdigest())
        self.assertEqual((release.OUTPUT / "notes.md").read_text(), "First release.\n")
        (release.OUTPUT / lines[0].split("  ")[1]).unlink()
        with self.assertRaises(ValueError):
            release.prepare("v0.1.0")

    def test_tag_must_match_package_version(self):
        release.validate_tag("v0.1.0")
        with self.assertRaises(ValueError):
            release.validate_tag("v0.2.0")
        with patch.dict(os.environ, {"GITHUB_REF_TYPE": "tag", "GITHUB_REF_NAME": "v0.2.0"}):
            with self.assertRaises(ValueError):
                self.package(release.TARGETS[0])

    def test_lfs_pointer_cannot_be_packaged_as_a_readme_image(self):
        self.image.write_text("version https://git-lfs.github.com/spec/v1\n")
        with self.assertRaises(ValueError):
            self.package(release.TARGETS[0])


if __name__ == "__main__":
    unittest.main()
