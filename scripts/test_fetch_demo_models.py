"""Check model cache integrity without network access."""
import hashlib
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import fetch_demo_models as demo


class ModelCacheTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.output = Path(temporary.name)
        self.data = b"verified model"
        self.entry = {
            "path": "Resistor.3dshapes/model.step",
            "url": "https://example.invalid/model.step",
            "sha256": hashlib.sha256(self.data).hexdigest(),
        }
        self.target = self.output / self.entry["path"]
        self.target.parent.mkdir()

    def test_verified_cache_needs_no_network(self):
        self.target.write_bytes(self.data)
        with patch.object(demo.urllib.request, "urlopen") as request:
            demo.fetch(self.entry, self.output)
        request.assert_not_called()

    def test_corrupt_cache_is_repaired(self):
        self.target.write_bytes(b"truncated download")
        with patch.object(demo.urllib.request, "urlopen", return_value=io.BytesIO(self.data)):
            demo.fetch(self.entry, self.output)
        self.assertEqual(self.target.read_bytes(), self.data)
        self.assertEqual(list(self.target.parent.iterdir()), [self.target])

    def test_wrong_remote_checksum_preserves_existing_file(self):
        self.target.write_bytes(b"existing")
        with patch.object(demo.urllib.request, "urlopen", return_value=io.BytesIO(b"wrong")):
            with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
                demo.fetch(self.entry, self.output)
        self.assertEqual(self.target.read_bytes(), b"existing")


if __name__ == "__main__":
    unittest.main()
