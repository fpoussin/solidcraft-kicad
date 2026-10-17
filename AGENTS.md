# Working on solidcraft-kicad

Rust CLI for converting KiCad PCBs into self-contained SolidCraft assemblies.
Read [README.md](README.md) for CLI usage, configuration, limitations, and the
validation workflow. This file records project requirements and
implementation decisions that future changes must preserve.

## Workflow

- Make one commit per feature. Inspect the working tree and fetch before
  squashing or pushing.
- Sign every commit.
- Keep KiCad inputs unchanged. Export from the original project directory so
  KiCad resolves its normal model paths and project variables. Route requested
  KiCad design edits through supported KiCad tools and APIs, not text patches.

## CLI requirements

- Successful builds are quiet by default. Full reports require `--json`;
  warnings and errors go to stderr.
- Generate the output name from the `.kicad_pcb` basename with a `.solidcraft`
  extension, in the **current working directory**. Respect explicit `--output`.
- Permit missing component models by default, retain available geometry, and
  report omissions. `--strict-models` opts into failure; board geometry is required.
- Allow replacement with `--force`/`-f`, or an interactive overwrite prompt whose
  default is **Yes**. EOF cancels. Publish atomically after successful conversion
  so failures preserve the previous output.
- Show progress only on interactive stderr, with no progress for `TERM=dumb`.
  Report each enabled copper, silkscreen, and soldermask side, plus pads/plating,
  substrate, models, preparation, and writing. Derive the step count from enabled
  layers and advance on actual completion; keep independent exports concurrent.

## Geometry and performance requirements

- Convert an **entire large PCB in under 10 seconds**, with default details
  enabled. SolidCraft must also open and render it within a few seconds; the
  desktop validation target is 3 seconds, excluding application startup.
- Measure release builds on real boards. Conversion speed alone is insufficient:
  verify saved-file opening and the actual desktop viewport as well.
- Simplification, batching, and merging decorative layers are acceptable while
  every component retains its colour and can be moved or hidden independently.
  Moving the assembly parent must move the board and all its components together.
- Share identical model geometry with matching material response through named
  occurrences. Keep per-footprint placements and visibility independent.
- Preserve the native CAD substrate so PCB outlines and holes have edges in the
  default **Shaded with edges** style. Keep component models and decorative layers
  as meshes, batched by appearance rather than one body per pad, track, or face.
- Offline builds include both-side silkscreen, soldermask, pads/plating, and
  external copper by default, with independent opt-out flags. Match each side's
  stackup colours and the surface finish, including HASL and ENIG. Inner copper
  is currently omitted.
- Preserve self-contained native output: reopening must work after temporary
  exports are deleted. Keep model resolution delegated to KiCad.

## Implementation map and pitfalls

- `src/main.rs`: CLI defaults, overwrite handling, and progress orchestration.
- `src/staging.rs`: concurrent model GLB, board-layer GLB, substrate STEP, and
  copper SVG exports; source-hash checks and permissive model diagnostics.
- `src/assembly.rs`: coordinate conversion, geometry sharing, material retention,
  native substrate insertion, and per-footprint occurrences. GLB is metres/Y-up;
  SolidCraft uses millimetres/Z-up.
- `src/copper.rs`: union copper plots, subtract drills, clip to the substrate,
  and partition mask surfaces into plain and copper-covered regions. Flat
  surfaces avoid the cost of reconstructing individual copper solids.
- `src/cad.rs`: native serialization and atomic output; the separate legacy B-rep
  workflow handles live insertion. See README before changing live mutation or
  retry behaviour; a lost reply may mean insertion already happened.
- `src/progress.rs`: terminal-only worker; layer counts live in `BoardLayers`.
- SolidCraft 0.14's viewport hides silkscreen without the current 0.1 mm outward
  display offset. Validate changes in `window.png`, not just offscreen renders.
- Board coatings are opaque for reliable colours. Copper beneath mask uses a
  subtle mask-coloured variation; exposed copper follows the plating finish.
  Preserve PBR properties when sharing or merging geometry.
- Cargo pins the SolidCraft engine and required kernel patches together. Keep
  them aligned when updating dependencies; consult Cargo.toml for the exact rev.

## Validation

Run checks appropriate to code changes:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

Normal tests use analytic geometry and a fake exporter. Real-board acceptance
tests are ignored by default; see `tests/assembly.rs` for environment variables.
Use representative large and small boards for performance and acceptance checks.
These fixtures and their model libraries are external to this repository.

For loading performance, use `examples/benchmark_open.rs` and
`scripts/validate-desktop.py --native FILE --app APP --max-open-seconds 3`.
The script launches an isolated desktop under Xvfb with private settings and
control credentials; inspect both `model.png` and `window.png`. Keep generated
benchmarks and native files under ignored `artifacts/`.

## Documentation and images

- Keep README explanations and examples generic; development boards are
  development fixtures, not prerequisites for users.
- Track raster images through Git LFS using `.gitattributes`; check existing
  images are LFS pointers in Git, with actual image data present locally.
- Use standard Markdown for README images for portable rendering.
- Prefer real CAD renders with a low viewing angle, tight framing, and the board
  centred in the image. The tracked preview is `docs/images/pcb-assembly.png`.
