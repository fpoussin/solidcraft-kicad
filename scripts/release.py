#!/usr/bin/env python3
"""Package native CI binaries and prepare complete, checksummed GitHub releases."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "artifacts" / "release"
TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
)


def version():
    package = (ROOT / "Cargo.toml").read_text(encoding="utf-8").split("[package]", 1)[1].split("\n[", 1)[0]
    match = re.search(r'^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$', package, re.MULTILINE)
    if not match:
        raise ValueError("Cargo.toml must declare a stable semantic version")
    return match[1]


def validate_tag(tag):
    expected = f"v{version()}"
    if tag != expected:
        raise ValueError(f"tag {tag!r} must match Cargo.toml version {expected!r}")


def dependency_notices(directory):
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, text=True, encoding="utf-8"))
    files = []
    index = []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if not package.get("source"):
            continue
        base = Path(package["manifest_path"]).parent
        roots = [base]
        if package["source"].startswith("git+"):
            for parent in base.parents:
                roots.append(parent)
                if (parent / ".git").exists():
                    break
                if len(roots) == 6:
                    break
        notices = []
        if package.get("license_file"):
            declared = base / package["license_file"]
            if declared.is_file():
                notices.append((declared, declared.name))
        for root in roots:
            found = []
            for path in sorted(root.iterdir()):
                if path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE")):
                    if path.is_file():
                        found.append((path, path.name))
                    elif path.is_dir():
                        found.extend((p, p.relative_to(root).as_posix()) for p in sorted(path.rglob("*")) if p.is_file())
            if found:
                notices.extend(found)
                break
        prefix = f"THIRD-PARTY-LICENSES/{package['name']}-{package['version']}"
        unique = {relative: path for path, relative in notices}
        files.extend((path, f"{prefix}/{relative}") for relative, path in unique.items())
        index.append({
            "name": package["name"], "version": package["version"],
            "license": package.get("license"), "authors": package.get("authors", []),
            "repository": package.get("repository"),
            "notice_files": [f"{prefix}/{relative}" for relative in unique],
        })
    manifest = directory / "DEPENDENCIES.json"
    manifest.write_text(json.dumps(index, indent=2) + "\n", encoding="utf-8")
    files.append((manifest, "THIRD-PARTY-LICENSES/DEPENDENCIES.json"))
    return files


def package(target):
    if os.environ.get("GITHUB_REF_TYPE") == "tag":
        validate_tag(os.environ["GITHUB_REF_NAME"])
    executable = "solidcraft-kicad" + (".exe" if target.endswith("windows-msvc") else "")
    binary = ROOT / "target" / target / "release" / executable
    actual = subprocess.check_output([str(binary), "--version"], text=True, encoding="utf-8").strip()
    if actual != f"solidcraft-kicad {version()}":
        raise ValueError(f"unexpected binary version: {actual}")
    subprocess.run([str(binary), "build", "--help"], check=True, stdout=subprocess.DEVNULL)
    image = ROOT / "docs" / "images" / "pcb-assembly.png"
    if not image.read_bytes().startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("README image must be checked out from Git LFS")
    name = f"solidcraft-kicad-v{version()}-{target}"
    OUTPUT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        info = Path(temporary) / "BUILD-INFO.json"
        info.write_text(json.dumps({
            "version": version(), "target": target,
            "commit": os.environ.get("GITHUB_SHA") or subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, encoding="utf-8").strip(),
        }, indent=2) + "\n", encoding="utf-8")
        files = [(binary, executable), (info, info.name)]
        files += dependency_notices(Path(temporary))
        files += [(ROOT / path, path) for path in (
            "README.md", "CHANGELOG.md", "LICENSE-MIT", "LICENSE-APACHE",
            "ATTRIBUTION.md", "Cargo.lock", "docs/images/pcb-assembly.png",
        )]
        if target.endswith("windows-msvc"):
            archive = OUTPUT / f"{name}.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED,
                                 strict_timestamps=False) as output:
                for source, relative in files:
                    output.write(source, f"{name}/{relative}")
        else:
            archive = OUTPUT / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                for source, relative in files:
                    output.add(source, arcname=f"{name}/{relative}")
    print(archive)


def prepare(tag, commit):
    validate_tag(tag)
    expected = {
        f"solidcraft-kicad-{tag}-{target}.{'zip' if target.endswith('windows-msvc') else 'tar.gz'}": target
        for target in TARGETS
    }
    actual = {p.name for p in OUTPUT.iterdir() if p.name.endswith((".zip", ".tar.gz"))}
    if actual != set(expected):
        raise ValueError(f"release archives do not match: expected {set(expected)}, found {actual}")
    checksums = []
    for name in sorted(expected):
        path = OUTPUT / name
        if path.stat().st_size == 0:
            raise ValueError(f"empty release archive: {name}")
        # Archives came from another run. Verify their embedded identity before
        # publishing any of them, without extracting executable content.
        base = f"solidcraft-kicad-{tag}-{expected[name]}"
        if name.endswith(".zip"):
            with zipfile.ZipFile(path) as archive:
                info = json.loads(archive.read(f"{base}/BUILD-INFO.json"))
        else:
            with tarfile.open(path, "r:gz") as archive:
                info = json.load(archive.extractfile(f"{base}/BUILD-INFO.json"))
        if info != {"version": version(), "target": expected[name], "commit": commit}:
            raise ValueError(f"archive provenance does not match release commit {commit}: {name}")
        checksums.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {name}\n")
    (OUTPUT / "SHA256SUMS").write_text("".join(checksums), encoding="utf-8")
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    marker = f"## {version()}\n"
    if marker not in changelog:
        raise ValueError(f"missing changelog entry for {version()}")
    notes = changelog.split(marker, 1)[1].split("\n## ", 1)[0].strip()
    (OUTPUT / "notes.md").write_text(notes + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("version", help="print the package version for tagging")
    commands.add_parser("validate-tag").add_argument("tag")
    commands.add_parser("package").add_argument("target", choices=TARGETS)
    prepare_command = commands.add_parser("prepare")
    prepare_command.add_argument("tag")
    prepare_command.add_argument("--commit", required=True, help="commit validated by main CI")
    args = parser.parse_args()
    if args.command == "version":
        print(version())
    elif args.command == "validate-tag":
        validate_tag(args.tag)
    elif args.command == "package":
        package(args.target)
    else:
        prepare(args.tag, args.commit)


if __name__ == "__main__":
    main()
