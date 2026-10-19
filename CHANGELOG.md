# Changelog

## Unreleased

- Package release archives in main-branch CI. Tag releases wait for successful
  CI on the same commit and publish its verified artifacts without rebuilding,
  allowing the commit and signed version tag to be pushed together.
- Bundle the MIT-licensed pico-ice Rev3 demo PCB with verified provenance,
  a pinned model downloader, and real KiCad conversion/reopening tests in Linux CI.
- Upgrade the embedded SolidCraft engine, control client and matching kernel
  patches to the `v0.16.2` Git tag. Generated assemblies use file format 27 and
  require SolidCraft 0.16.0 or newer for opening and live insertion.
- Add `scripts/update_solidcraft.py` to update coupled tags, discover kernel
  patches, refresh compatibility documentation and validate upgrades. Derive
  engine metadata automatically and reject mismatched tags/revisions at build time.
- Add `--check-update` to check GitHub immediately, bypassing the daily cache,
  without a PCB. Supports JSON output and reports explicit check failures.
- Isolate KiCad subprocess temporary files so concurrent exports do not race
  on the CLI instance lock or emit invalid-lock warnings.

## 0.2.0

- Upgrade the embedded SolidCraft engine, control client and matching kernel
  patches to the `v0.15.0` release tag.
- Live insertion accepts SolidCraft 0.15.0 and newer using semantic version
  comparison. JSON insertion reports identify the connected desktop version.
- Generated assemblies use SolidCraft file format 26 and require SolidCraft
  0.15.0 or later to open. SolidCraft 0.14.x and earlier cannot open them.
- Run validation for pull requests and branch pushes; build and publish release
  archives only for version tags, after checks pass on all three platforms.

## 0.1.1

- Release builds notify interactive users about newer stable GitHub releases,
  using a daily background check. Offline failures are silent; debug builds,
  JSON output, and redirected stderr skip checking. `--no-update-check` opts out.
- No message is printed when already up to date.

## 0.1.0

First release of solidcraft-kicad: import KiCad PCBs as self-contained, coloured
SolidCraft assemblies with independently movable and hideable components.

- Fast native builds with shared component geometry and a CAD substrate that
  displays outline and hole edges in Shaded with edges mode.
- Both-side silkscreen, stackup-matched soldermask, plated pads and holes, and
  outer copper tracks, vias and filled zones, enabled by default.
- HASL, ENIG and other surface finishes, with optional layer exclusions.
- Automatic filenames in the current directory, permissive missing models,
  atomic overwrite handling, and interactive per-layer progress.
- Optional JSON reports, configurable model paths, and live insertion into an
  open SolidCraft assembly.

Download the archive for **Linux x64**, **macOS arm64 (Apple Silicon)**, or
**Windows x64**. Extract it and put `solidcraft-kicad` (or `solidcraft-kicad.exe`)
on your `PATH`. `SHA256SUMS` contains archive checksums.

Requires **KiCad 10** and its model libraries. Offline conversion uses the
embedded SolidCraft 0.14.0 engine; opening the result or live insertion requires
SolidCraft 0.14.0. The macOS binary targets macOS 14 or later. Linux binaries
require glibc 2.35 or later and are built on Ubuntu 22.04. See the README for
configuration and examples.

On the development machine, a 323-part board with all default details converts
in about **7.5 seconds**, opens in **1.6 seconds**, and renders in **2.4 seconds**
including opening time. Inner copper and editable board sketches are not included.
