#!/usr/bin/env python3
"""Fetch only the pinned KiCad models needed by the bundled demo PCB."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import tempfile
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "tests/fixtures/pico-ice/models.json"
DEFAULT_OUTPUT = ROOT / "artifacts/demo-models"


def fetch(entry, output):
    target = output / entry["path"]
    if target.is_file() and hashlib.sha256(target.read_bytes()).hexdigest() == entry["sha256"]:
        return
    with urllib.request.urlopen(entry["url"], timeout=60) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != entry["sha256"]:
        raise ValueError(f"Checksum mismatch: {entry['path']}")
    target.parent.mkdir(parents=True, exist_ok=True)
    # Never leave a partial download at a model's final path.
    with tempfile.NamedTemporaryFile(dir=target.parent, delete=False) as stream:
        temporary = Path(stream.name)
        stream.write(data)
    try:
        temporary.replace(target)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    entries = json.loads(MANIFEST.read_text())["files"]
    with ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(lambda entry: fetch(entry, args.output), entries))
    print(f"Verified {len(entries)} files in {args.output.resolve()}")


if __name__ == "__main__":
    main()
