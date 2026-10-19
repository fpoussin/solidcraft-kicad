# solidcraft-kicad

Import a KiCad PCB as a native SolidCraft component assembly, or insert it directly
into an open assembly. Rust orchestration reuses KiCad's model resolver/exporter,
KiUtils, and SolidCraft's own engine, document writer, and control client.

![A KiCad PCB imported into SolidCraft, with copper traces, silkscreen, plated pads and coloured component models](docs/images/pcb-assembly.png)

*A native SolidCraft assembly with outer copper, silkscreen, plated pads and matching soldermask colours.*

The result is a PCB parent component with a substrate child and a child per
populated footprint with visible assigned models. Builds use coloured triangle
meshes for component models and board coatings so even large boards open quickly.
The substrate is a native CAD solid, giving the PCB outline and hole edges in
SolidCraft's default **Shaded with edges** view. Each footprint can be moved or hidden
independently; moving the parent moves the board and its parts together. Colours
and KiCad's model placement are retained. Saved designs embed the geometry and
do not depend on temporary assets.

## Requirements and installation

Download a prebuilt archive from [GitHub Releases](https://github.com/fpoussin/solidcraft-kicad/releases)
for Linux x64, macOS arm64 (Apple Silicon), or Windows x64. Extract it and put the
executable on your `PATH`. Releases include `SHA256SUMS` for archive verification.
The macOS binary targets macOS 14 or later; Linux binaries require glibc 2.35 or
later and are built on Ubuntu 22.04.

To build from source:

- Rust 1.95 or later, Cargo, and a C compiler for upstream native dependencies.
- KiCad 10's `kicad-cli` on `PATH`, its model libraries, and your usual KiCad path
  configuration.
- A compatible SolidCraft desktop for opening files or live insertion, as shown
  below. Offline builds need neither the desktop application nor `solidcraft-cli`.

```sh
cargo build --locked --release
./target/release/solidcraft-kicad --help
```

The SolidCraft dependency and its required kernel patches share a Git release
tag in `Cargo.toml`. `Cargo.lock` records the resolved commit for reproducible
builds. The
upstream crates include the CAD kernel; the first compilation is larger than a
typical command-line utility. There is no runtime dependency on FreeCAD.
README images use Git LFS; run `git lfs pull` if they are missing after cloning.

<!-- solidcraft-compatibility:begin -->
Offline builds embed SolidCraft **0.16.2** (`v0.16.2`).
Generated designs use file format **27** and require
**SolidCraft 0.16.0 or later** for opening and live insertion.
<!-- solidcraft-compatibility:end -->

## Build a native part assembly

```sh
solidcraft-kicad build path/to/board.kicad_pcb
```

This creates `board.solidcraft` in the current directory. Use `--output FILE` to
choose another filename or directory. Open the resulting file in SolidCraft.
If the output exists, interactive terminals prompt before replacing it (default:
Yes). Use `--force` (`-f`) to overwrite without prompting, including in scripts:

```sh
solidcraft-kicad build path/to/board.kicad_pcb --force
```

The existing file is replaced atomically after a successful conversion.
Successful builds and insertions show a progress bar only when stderr is an
interactive terminal. Builds report each enabled copper, silkscreen and soldermask
side, pads/plating, the substrate and component models as their exports finish.
The step count adjusts to omitted layers; exports remain concurrent.
Redirected output stays quiet; warnings and errors go to
stderr. Add `--json` to print a report to stdout with source hash, exported
references, prefix overrides and omissions:

```sh
solidcraft-kicad build path/to/board.kicad_pcb --json
```

Release builds check GitHub for newer stable releases in the background, at most
once a day, and show an update notice on interactive stderr after the command.
Automatic checks never delay conversion; offline failures are silent. They are
disabled for debug builds, `--json`, redirected stderr, and `--help`/`--version`.
Use `--no-update-check` to disable checking and notices. The cache is stored in
the platform's user cache directory under `solidcraft-kicad`.

To check immediately, ignoring the daily cache, run:

```sh
solidcraft-kicad --check-update
solidcraft-kicad --check-update --json
```

This checks and exits without a PCB or conversion. It works in debug builds and
redirected output too, waits for the bounded request, and reports either a newer
release or that you are up to date. An unsuccessful explicit check reports an
error and exits unsuccessfully. `--json` prints the current and latest versions,
whether an update is available, and the download URL. It cannot be combined with
`--no-update-check`.

Builds include front and back silkscreen, soldermask with its openings, pads,
plated holes, and outer copper tracks, vias and filled zones by default.
Soldermask and silkscreen colours come from the PCB's
physical stackup, including separate colours for each side. Pad and hole plating
uses the stackup's surface finish: gold for ENIG/ENEPIG and gold finishes, tin for
HASL/HAL and tin/nickel finishes, silver for silver finishes, and copper for OSP.
Unspecified or unrecognised finishes retain KiCad's exported pad colour.
Metallic response and roughness are saved with the geometry. Thin board coatings
are rendered opaque for clear silkscreen and predictable colours; component
materials retain their exported opacity.
A 0.1 mm outward silkscreen display offset keeps lettering visible in
SolidCraft's desktop viewport.

Outer copper is plotted by KiCad and tessellated into flat surfaces, with a
5 µm curve tolerance. Overlapping items are united, drill openings are subtracted,
and copper is clipped to the board outline and cutouts. Covered copper is baked
into the soldermask as a subtle colour and roughness variation; exposed copper
uses the same finish as the pads. Each side adds at most two mesh bodies,
independent of the number of tracks or zones. Inner copper is omitted.

Omit individual details when desired:

```sh
solidcraft-kicad build board.kicad_pcb --no-silkscreen
solidcraft-kicad build board.kicad_pcb --no-pads --no-soldermask
solidcraft-kicad build board.kicad_pcb --no-copper
```

`--no-copper` omits the outer copper surfaces while retaining raised pads and
plated holes. `--no-pads` omits those solids; flat pad shapes remain part of the
copper plots when copper is enabled.

The board layers and component models are exported concurrently as binary glTF
(GLB), alongside copper SVG plots and a small STEP export of the substrate.
Faces sharing a material
are batched, and exact duplicate board vertices are stored once. The writer
preserves placements, converts metres/Y-up to KiCad's
millimetres/Z-up, and groups model meshes under their footprint references.
Identical geometry with matching colour and material response is stored once,
with a named occurrence for each footprint. Each occurrence retains its own
placement and visibility. Component meshes avoid STEP reconstruction and surface
tessellation both during conversion and when SolidCraft opens the file. Only the
substrate uses native CAD geometry; mask, silk, pad and copper triangles are kept as
batched mesh bodies, without triangle edges cluttering the board surface.

`--json --measure` also includes mesh dimensions, volume and area in the report.
These measurements describe the exported triangles; builds are intended for
assembly placement and visual fit checks, rather than editing exact CAD surfaces.

For example, a board's component tree might look like:

```text
board
├── PCB
├── C1
├── J1
├── R1
├── U1
└── …
```

Each part may contain several mesh bodies to retain different material colours.
Imported geometry uses the common KiCad board frame; editable footprint-local
sketch history is not reconstructed.

## Insert into an open assembly

Start SolidCraft with its control channel enabled:

```sh
solidcraft --control 7878
solidcraft-kicad insert path/to/board.kicad_pcb --port 7878
```

Live insertion uses the existing B-rep workflow and can take longer than building
and opening a mesh assembly. Its contents are rigidly grouped; use Unlock
Contents to move those parts separately.

The existing `solidcraft-mcp::Remote` client finds SolidCraft's per-launch token
automatically, including `SOLIDCRAFT_CONTROL_TOKEN` and `SOLIDCRAFT_CONTROL_DIR`.
The companion connects only to loopback. It does not start, restart, switch tabs,
or save the desktop application.

Keep the intended assembly active until insertion completes. Finish active
tools/dialogs, move the timeline to its end, and disable the agent cursor before
inserting. Unsaved designs are supported. The companion snapshots the destination
before staging and checks it again immediately before sending the insertion.
The insertion request has no document-target precondition, so this check cannot lock
the active tab against a concurrent switch.

The new PCB is inserted at the design root and the current authoring component
is preserved. Names receive numeric suffixes when required by existing component
or instance labels. Names are selected before construction so component names
and occurrence labels stay consistent.

All live mutations use one direct `engine.batch` with rollback enabled. Command
failure restores the checkpoint; successful insertion follows SolidCraft's
normal multiple-entry undo history. On a lost or timed-out reply, the operation
may have run: the companion never repeats it and retains staging files at the
reported path. Inspect the application before another invocation. Each normal
invocation adds a new assembly; it does not update an earlier import.

## Finding 3D models

With no companion configuration, KiCad resolves the source's model paths using
its own installation defaults, Configure Paths settings, environment and project
location. This includes `${KICAD10_3DMODEL_DIR}`, `${KIPRJMOD}`, and custom prefixes
such as `${CUSTOM_3DMODELS}`. Configure Paths variables need not exist in your
shell environment. `KICAD_CONFIG_HOME` is inherited. The original board stays in
its original project directory during export.

Optional TOML configuration supplies prefix overrides:

```toml
[model_prefixes]
CUSTOM_3DMODELS = "../kicad-libs/3d"
KICAD10_3DMODEL_DIR = "/usr/share/kicad/3dmodels"
```

Configuration sources, highest priority first:

1. Repeatable `--model-prefix NAME=PATH` arguments.
2. `--config FILE`, or the nearest `solidcraft-kicad.toml` found walking upward
   from the PCB directory.
3. User `config.toml` in the platform configuration folder (Linux:
   `$XDG_CONFIG_HOME/solidcraft-kicad/config.toml`, normally
   `~/.config/solidcraft-kicad/config.toml`).
4. KiCad's normal resolution.

Figment resolves paths from TOML files relative to the file that defined each
value. CLI paths are relative to the invocation directory. Use absolute paths or
ordinary relative paths; shell-style `~` expansion inside TOML is not supported.
Overrides apply only to KiCad child processes, never to your KiCad settings or
the parent environment. `KIPRJMOD` cannot be overridden.

```sh
solidcraft-kicad build board.kicad_pcb --output board.solidcraft \
  --model-prefix CUSTOM_3DMODELS=/data/models \
  --define-var VARIANT=production
```

`--define-var KEY=VALUE` forwards project variables to KiCad. Conflicting repeated
values, or different values for the same model-prefix/project-variable key,
are errors. `--kicad-cli PATH` selects another executable; `--timeout SECONDS`
sets the per-process timeout (default 120).

STEP substitution is enabled for VRML references. KiCad resolves a suitable STEP
or IGES counterpart and exports its geometry; embedded models are delegated to
KiCad too.

## Missing models and geometry limits

Missing or unreadable assigned models are permitted by default. Available
geometry is retained, with omitted components reported as warnings on stderr and
in the optional JSON report's `omissions` and `assigned_models_complete: false`.
The substrate is always required. Use `--strict-models` to reject incomplete
assigned-model exports:

```sh
solidcraft-kicad build board.kicad_pcb --strict-models
```

`--allow-missing-models` remains accepted for compatibility. Footprints without
visible model assignments are reported separately as warnings; DNP footprints
are excluded. Live insertion validates imported solids during staging; offline
builds preserve KiCad's coloured meshes. `--measure` adds mesh measurements.
Use `--json` to review omissions before relying on an assembly for fit checks.

Inner copper is omitted. The substrate solid represents the
dielectric body and can be thinner than the board's nominal total stack-up.
Board surfaces and component placement follow KiCad's stack-up offsets. Native
mesh components support visual fit checks, placement and visibility; editable
board sketches, synchronization and ECAD write-back are deferred. The current
input limits are 64 MiB and 1000 footprints.

The application owns source-path/hash/reference provenance under the native
metadata key `org.solidcraft-kicad.imports`. It leaves other metadata intact.

## Development and validation

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Normal tests need no KiCad installation or GUI: an analytic solid and a fake
exporter exercise concurrent board/model exports, both-side layer colours,
surface finishes, metallic appearances, vertex sharing, persistence, individual
visibility and movement, collisions in names, rollback, destination changes,
missing-model policy and lost replies. The substrate is also checked for display
edges visible in the shaded-with-edges style after saving and reopening.
Board-specific acceptance tests are ignored by default because their source
boards and model libraries are not bundled. See [tests/assembly.rs](tests/assembly.rs)
for their required environment variables and expected geometry.
To exercise the actual desktop control channel on Linux, use a separate app
under Xvfb (requires `xvfb-run`):

```sh
python3 scripts/validate-desktop.py --app /path/to/SolidCraft.AppImage \
  --board /path/to/board.kicad_pcb --cli target/debug/solidcraft-kicad \
  --expected-solids 42
```

This launches a disposable desktop with isolated settings and a private control
token. It inserts into an assembly containing an analytic test solid, verifies
that solid is unchanged, and writes screenshots, the native assembly and JSON
evidence under the ignored `artifacts/desktop` directory. It never connects to
your existing application. Set `--expected-solids` to the number of imported
solids expected for your board, including the substrate (42 is an example).
This can differ from the footprint count because a model may contain several
solids.

Subprocess tests require normal local IPC; restrictive sandboxes may block the
timeout library's signal pipe.

To time native-file loading and the first render in an isolated desktop:

```sh
python3 scripts/validate-desktop.py --app /path/to/SolidCraft.AppImage \
  --native /path/to/board.solidcraft --max-open-seconds 3
```

A development benchmark with 323 placed parts and all default board details,
including outer copper, converted in **7.54 s**. SolidCraft 0.16.2 opened the
result in **1.74 s**, with its first render ready in **2.52 s** in the isolated
desktop. The placed parts,
including the board, share 42 geometry definitions. All board layers together
use nine mesh bodies alongside the CAD substrate; the native file occupies
**27.6 MB**. One unavailable
model was reported as an omission. These are measured wall times on the
development machine, excluding application startup.

No KiCad source files are rewritten. KiCad itself can create local `.kicad_prl`
preferences when loading a project that has none.

To upgrade the embedded SolidCraft release, use Python 3.11+ and an authenticated
GitHub CLI:

```sh
python3 scripts/update_solidcraft.py --validate
```

The command selects the latest stable upstream release, updates every coupled
Git tag, discovers the required kernel patches from the resolved dependency
graph, refreshes the lockfile and compatibility block above, and adds a changelog
entry. `--tag` selects a specific release. `--validate` runs formatting, Clippy,
tests and a release build; real-board and desktop checks still use the workflow
above.

The engine version comes from `Cargo.lock`; the native format comes from the
linked engine. The only manual compatibility policy is `minimum-version` in
`[package.metadata.solidcraft]`. When the native format changes, the command
requires `--minimum-version` with the first compatible desktop release from
upstream's compatibility notes. Failed or interrupted upgrades restore the
manifest, lockfile, README and changelog. Builds reject mixed upstream tags or
revisions, and tests check the documented versions and format against the engine.

## Releases

Pull requests and pushes to `master` run the **CI** workflow: formatting, Clippy,
maintenance-script tests, and release-mode tests on Linux x64, macOS arm64
and Windows x64. These runs do not create release archives or publish releases.

Version-tag pushes (`v*`) run the **Release** workflow. It reuses the same CI
validation, then builds, smoke-tests and packages the three native binaries.
Release archives are available as workflow artifacts before publication.

To publish a release, update `Cargo.toml`, `Cargo.lock` and the corresponding
section in [CHANGELOG.md](CHANGELOG.md), commit with a signature, and push the
branch. After branch CI passes, create and push a signed `vVERSION` tag matching
the package version. The release workflow validates it before publishing.
For example:

```sh
git tag -s v0.2.0 -m 'solidcraft-kicad v0.2.0'
git push origin v0.2.0
```

The tag runs the complete CI workflow. Once every platform and check passes, CI
packages the binaries with documentation, project and dependency license notices,
the README image and build provenance, generates `SHA256SUMS`, and publishes all
three archives together to
GitHub Releases. Publishing uses the workflow's GitHub token; no personal token
or local release builds are required. A failed draft upload can be retried; an
already published release is never overwritten.

## Dependencies and license

This project is independently implemented and does not contain StepUp code.
KiUtils parses metadata; KiCad exports geometry; SolidCraft imports, validates,
builds and serializes designs. Offline conversion uses SolidCraft's document
writer directly to preserve exported meshes. Clap handles arguments, Figment
handles layered configuration and relative paths, and tempfile manages staging.

Licensed under MIT or Apache-2.0, at your option. Upstream dependencies retain
their own licenses and notices. See [ATTRIBUTION.md](ATTRIBUTION.md).
