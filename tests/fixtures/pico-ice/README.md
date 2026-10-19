# pico-ice Rev3 demo PCB

An MIT-licensed, routed four-layer RP2040 + iCE40UP5K FPGA board by tinyVision.ai:
101 footprints, USB-C, flash, PSRAM, regulators, headers, mounting holes, vias,
filled zones, and graphics on both sides. It exercises repeated component
geometry, component placement, native substrate, and the default surface layers.
It is a conversion fixture, not a manufacturing reference or a large-board
performance benchmark.

## Source and license

The PCB, project configuration, and [LICENSE](LICENSE) are **unmodified copies**
from [tinyvision-ai-inc/pico-ice](https://github.com/tinyvision-ai-inc/pico-ice/tree/00e13360969d01fbb9663e4ea17f723c1a8e8400/Board/Rev3),
commit `00e13360969d01fbb9663e4ea17f723c1a8e8400`. [upstream.json](upstream.json)
records original paths and SHA-256 checksums; the normal test suite verifies all
three files. Schematics, fabrication outputs, and upstream renders are omitted.
Embedded footprints need no separately installed footprint library for export.

The optional external 3D models have **their own licenses**, not MIT. They are
downloaded from immutable official KiCad library revisions, with URLs and hashes
in [models.json](models.json). The cache includes KiCad's library license and
the original model headers/attributions. See each model's header for its exact
terms; KiCad's library license includes an exception for electronic designs and
generated files. Models are not vendored or included in release archives.

The original board has six unresolved model assignments: `J1` (USB-C), `D2`
(RGB LED), `U2`/`U8` (memory), and `SW1`/`SW2` (buttons pointing to the author's
Windows drive). Another 19 footprints have no assigned model, including logos,
fiducials, mounting holes and solder jumpers. These are expected omissions;
tests require all 76 available component models to import and remain independent.
The board is intentionally preserved rather than substituting approximate parts.

## Run

From the repository root, install KiCad 10 (`kicad-cli` on `PATH`), then:

```sh
python3 scripts/fetch_demo_models.py
cargo test --locked --release --test demo -- --include-ignored
```

The first command downloads about 4.8 MB into ignored `artifacts/demo-models`.
Subsequent runs verify and reuse the cache; no full model library is needed.
Legacy `.wrl` files are included because KiCad requires them to exist before
substituting the matching STEP files with `--subst-models`.
The acceptance test overrides the two old model-directory variables without
changing the PCB, project, or user's settings. It checks expected omissions,
geometry sharing, independent occurrences, a native substrate, and reopening
after the temporary exports have been deleted. It also verifies input files
remain unchanged. This test runs in the Linux CI job.

For a demo assembly:

```sh
cargo run --locked --release -- build tests/fixtures/pico-ice/pico-ice.kicad_pcb \
  --model-prefix KICAD6_3DMODEL_DIR="$PWD/artifacts/demo-models" \
  --model-prefix KISYS3DMOD="$PWD/artifacts/demo-models" \
  --output artifacts/pico-ice.solidcraft --force
```

Outputs and screenshots belong under ignored `artifacts/`.
