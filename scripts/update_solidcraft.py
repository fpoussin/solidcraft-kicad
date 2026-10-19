#!/usr/bin/env python3
"""Update coupled SolidCraft tags, discover required patches, and refresh docs."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent
BEGIN = "<!-- solidcraft-compatibility:begin -->"
END = "<!-- solidcraft-compatibility:end -->"
FILES = ("Cargo.toml", "Cargo.lock", "README.md", "CHANGELOG.md")


def run(command):
    return subprocess.run(command, cwd=ROOT, check=True, capture_output=True,
                          text=True, encoding="utf-8").stdout


def write(name, text):
    (ROOT / name).write_bytes(text.encode("utf-8"))


def version(value):
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value):
        raise ValueError(f"expected a stable semantic version, got {value!r}")
    return tuple(map(int, value.split(".")))


def section(text, name, edit):
    pattern = rf"(?ms)(^\[{re.escape(name)}\][^\n]*\n)(.*?)(?=^\[|\Z)"
    result, count = re.subn(pattern, lambda m: m[1] + edit(m[2]), text, count=1)
    if count != 1:
        raise ValueError(f"missing [{name}] section")
    return result


def set_tags(text, git, tag):
    dependencies = tomllib.loads(text)["dependencies"]

    def edit(body):
        for name, dependency in dependencies.items():
            if not isinstance(dependency, dict) or dependency.get("git") != git:
                continue
            pattern = rf'(?m)(^{re.escape(name)}\s*=\s*\{{[^\n]*?)(?:tag|rev|branch)\s*=\s*"[^"]*"'
            body, count = re.subn(pattern, lambda m: m[1] + f'tag = "{tag}"', body)
            if count != 1:
                raise ValueError(f"{name} needs one inline Git tag declaration")
        return body

    return section(text, "dependencies", edit)


def set_patches(text, git, tag, names):
    existing = tomllib.loads(text)["patch"]["crates-io"]
    managed = {name for name, value in existing.items() if value.get("git") == git}
    conflicts = set(names) & (set(existing) - managed)
    if conflicts:
        raise ValueError(f"upstream patches conflict with existing overrides: {sorted(conflicts)}")

    def edit(body):
        lines = []
        for line in body.splitlines():
            match = re.match(r"([A-Za-z0-9_-]+)\s*=", line)
            if not match or match[1] not in managed:
                lines.append(line)
        retained = "\n".join(lines).strip()
        added = "\n".join(
            f'{name} = {{ git = {json.dumps(git)}, tag = {json.dumps(tag)} }}'
            for name in sorted(names)
        )
        return (retained + "\n" if retained else "") + added + "\n\n"

    return section(text, "patch.crates-io", edit)


def set_minimum(text, minimum):
    def edit(body):
        body, count = re.subn(r'(?m)^minimum-version\s*=\s*"[^"]*"',
                              f'minimum-version = "{minimum}"', body)
        if count != 1:
            raise ValueError("missing minimum-version compatibility policy")
        return body

    return section(text, "package.metadata.solidcraft", edit)


def compatibility_block(info):
    return (
        f"{BEGIN}\n"
        f"Offline builds embed SolidCraft **{info['version']}** (`v{info['version']}`).\n"
        f"Generated designs use file format **{info['file_format']}** and require\n"
        f"**SolidCraft {info['minimum_version']} or later** for opening and live insertion.\n"
        f"{END}"
    )


def update_readme(text, info):
    if text.count(BEGIN) != 1 or text.count(END) != 1:
        raise ValueError("README must have one managed compatibility block")
    before, rest = text.split(BEGIN)
    _, after = rest.split(END)
    return before + compatibility_block(info) + after


def engine_info():
    return json.loads(run(["cargo", "run", "--quiet", "--locked", "--example", "solidcraft_info"]))


def upgrade(tag=None, minimum=None, validate=False):
    originals = {name: (ROOT / name).read_bytes() for name in FILES}
    manifest = tomllib.loads(originals["Cargo.toml"].decode())
    engine = manifest["dependencies"]["solidcraft-engine"]
    git = engine["git"]
    match = re.fullmatch(r"https://github.com/([^/]+/[^/]+?)(?:\.git)?", git)
    if not match:
        raise ValueError("expected a GitHub upstream source")
    repo = match[1]
    if tag is None:
        release = json.loads(run(["gh", "api", f"repos/{repo}/releases/latest"]))
        tag = release["tag_name"]
    target = tag.removeprefix("v")
    version(target)
    upstream = tomllib.loads(run([
        "gh", "api", f"repos/{repo}/contents/Cargo.toml?ref={tag}",
        "-H", "Accept: application/vnd.github.raw+json",
    ]))
    if upstream["workspace"]["package"]["version"] != target:
        raise ValueError("release tag does not match upstream's package version")
    upstream_patches = set(upstream["patch"]["crates-io"])
    old = engine_info()
    supported = minimum or manifest["package"]["metadata"]["solidcraft"]["minimum-version"]
    if version(supported) > version(target):
        raise ValueError("minimum desktop version cannot be newer than the embedded engine")
    try:
        text = set_minimum(set_tags(originals["Cargo.toml"].decode(), git, tag), supported)
        # Resolve with every upstream patch first. Metadata then identifies which
        # are actually needed by our graph, including newly added kernel patches.
        write("Cargo.toml", set_patches(text, git, tag, upstream_patches))
        metadata = json.loads(run(["cargo", "metadata", "--format-version", "1"]))
        needed = upstream_patches & {package["name"] for package in metadata["packages"]}
        write("Cargo.toml", set_patches(text, git, tag, needed))
        run(["cargo", "metadata", "--format-version", "1"])
        info = engine_info()
        if info["version"] != target or info["minimum_version"] != supported:
            raise ValueError("linked engine metadata does not match the requested upgrade")
        if info["file_format"] != old["file_format"]:
            if minimum is None or version(supported) <= version(old["minimum_version"]):
                raise ValueError(
                    f"native format changed {old['file_format']} -> {info['file_format']}; "
                    "check upstream compatibility notes and rerun with --minimum-version "
                    "FIRST_SUPPORTED_RELEASE"
                )
        write("README.md", update_readme(originals["README.md"].decode(), info))
        if tag != engine["tag"]:
            notes = originals["CHANGELOG.md"].decode()
            if "## Unreleased\n" not in notes:
                notes = notes.replace("# Changelog\n", "# Changelog\n\n## Unreleased\n", 1)
            notes = notes.replace("## Unreleased\n", (
                f"## Unreleased\n\n- Upgrade SolidCraft crates and kernel patches to `{tag}` "
                f"(native format {info['file_format']}; desktop {supported} or newer).\n"
            ), 1)
            write("CHANGELOG.md", notes)
        if validate:
            for command in (
                ["cargo", "fmt", "--check"],
                ["cargo", "clippy", "--locked", "--all-targets", "--", "-D", "warnings"],
                ["cargo", "test", "--locked"],
                ["cargo", "build", "--locked", "--release"],
            ):
                run(command)
    except BaseException:
        for name, data in originals.items():
            (ROOT / name).write_bytes(data)
        raise
    print(json.dumps({**info, "tag": tag, "kernel_patches": sorted(needed)}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", help="release tag (default: latest stable GitHub release)")
    parser.add_argument("--minimum-version", help="first desktop release reading a changed native format")
    parser.add_argument("--validate", action="store_true", help="also run formatting, Clippy, tests and release build")
    args = parser.parse_args()
    try:
        upgrade(args.tag, args.minimum_version, args.validate)
    except subprocess.CalledProcessError as error:
        parser.exit(1, f"{error.stderr or error}\n")
    except (ValueError, KeyError, OSError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
