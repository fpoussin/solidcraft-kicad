# Attribution

solidcraft-kicad is independently implemented. KiCad StepUp was examined to
understand the user workflow; none of its code or assets are included.

The following dependencies provide the implementation's main capabilities:

| Dependency | Role | Upstream |
| --- | --- | --- |
| SolidCraft 0.14.0 | Native engine, B-rep geometry, headless backend, document persistence and authenticated control client | https://github.com/tangency-labs/solidcraft |
| KiUtils | Read-only KiCad typed metadata and S-expression parsing | https://github.com/Milind220/kiutils-rs |
| KiCad CLI (external executable) | Model resolution and PCB/component STEP export | https://www.kicad.org/ |
| clap | CLI | https://github.com/clap-rs/clap |
| Figment | Configuration merging, provenance and relative paths | https://github.com/SergioBenitez/Figment |
| Serde and serde_json | Structured data | https://serde.rs/ |
| directories | Platform configuration location | https://github.com/dirs-dev/directories-rs |
| tempfile | Temporary files/directories | https://github.com/Stebalien/tempfile |
| thiserror | Errors | https://github.com/dtolnay/thiserror |
| wait-timeout | Child-process timeout | https://github.com/alexcrichton/wait-timeout |
| sha2 | Source fingerprint | https://github.com/RustCrypto/hashes |

SolidCraft's required truck/matext4cgmath patches are reused at the same immutable
upstream commit as its engine. Cargo.lock records all direct and transitive
versions. The dependency licenses are distributed with their Cargo sources.
