//! A public board fixture: metadata runs everywhere; real exports require KiCad 10.
use std::{path::PathBuf, time::Duration};

use serde_json::{Value, json};
use solidcraft_kicad::{
    cad,
    config::{Prefix, Settings},
    error::{Error, Result},
    inventory,
    staging::Exporter,
};
use solidcraft_mcp::Headless;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pico-ice")
}

fn verify_sources() -> Result<()> {
    let provenance: Value = serde_json::from_str(include_str!("fixtures/pico-ice/upstream.json"))?;
    let files = provenance["files"]
        .as_object()
        .ok_or_else(|| Error::message("missing fixture provenance"))?;
    for (name, entry) in files {
        assert_eq!(inventory::hash(&fixture().join(name))?, entry["sha256"]);
    }
    Ok(())
}

#[test]
fn demo_inventory() -> Result<()> {
    verify_sources()?;
    let board = inventory::read(&fixture().join("pico-ice.kicad_pcb"))?;
    assert_eq!(board.name, "pico-ice");
    // Captures reference/model parsing on a real routed PCB without external tools.
    assert_eq!(board.footprints.len(), 101);
    assert_eq!(board.warnings.len(), 19);
    for reference in ["U3", "U6", "J1", "SW1", "SW2"] {
        assert!(board.footprints.iter().any(|fp| fp.reference == reference));
    }
    Ok(())
}

#[test]
#[ignore = "requires KiCad 10; first run python3 scripts/fetch_demo_models.py"]
fn demo_mesh_acceptance() -> Result<()> {
    verify_sources()?;
    let models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/demo-models");
    let manifest: Value = serde_json::from_str(include_str!("fixtures/pico-ice/models.json"))?;
    for entry in manifest["files"]
        .as_array()
        .ok_or_else(|| Error::message("missing model manifest"))?
    {
        let name = entry["path"]
            .as_str()
            .ok_or_else(|| Error::message("missing model path"))?;
        let path = models.join(name);
        if !path.is_file() {
            return Err(Error::message(
                "run python3 scripts/fetch_demo_models.py first",
            ));
        }
        assert_eq!(inventory::hash(&path)?, entry["sha256"]);
    }
    let mut settings = Settings::default();
    for variable in ["KICAD6_3DMODEL_DIR", "KISYS3DMOD"] {
        settings.model_prefixes.insert(
            variable.into(),
            Prefix {
                path: models.clone(),
                source: "pinned demo models".into(),
            },
        );
    }
    let exporter = Exporter {
        program: std::path::Path::new("kicad-cli"),
        timeout: Duration::from_secs(120),
        settings: &settings,
    };
    let stage = exporter.stage_build(
        inventory::read(&fixture().join("pico-ice.kicad_pcb"))?,
        true,
    )?;
    let missing: Vec<_> = stage
        .omissions
        .iter()
        .filter_map(|message| {
            message
                .split_once(": omitted;")
                .map(|(reference, _)| reference)
        })
        .collect();
    assert_eq!(missing, ["D2", "J1", "SW1", "SW2", "U2", "U8"]);
    assert_eq!(stage.assets.len(), 77); // PCB plus 76 independent model occurrences.
    let output = tempfile::tempdir()?;
    let native = output.path().join("pico-ice.solidcraft");
    cad::build_with_measurements(&stage, &native, false)?;
    drop(stage); // Reopening must not depend on temporary exports.
    verify_sources()?;
    let mut backend = Headless::default();
    cad::execute(&mut backend, "doc.open", json!({"path": native}))?;
    assert!(
        backend
            .session
            .model
            .results
            .iter()
            .all(|r| r.error.is_none())
    );
    assert_eq!(backend.session.doc.occurrences.len(), 78);
    assert!(backend.session.doc.components.len() < 78); // Repeated parts share geometry.
    let world = backend.session.world_state();
    assert_eq!(world.bodies.iter().filter(|b| !b.body.is_mesh()).count(), 1);
    assert!(world.bodies.iter().filter(|b| b.body.is_mesh()).count() > 76);
    Ok(())
}
