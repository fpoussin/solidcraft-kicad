//! Analytic geometry fixtures exercise orchestration without requiring KiCad or a GUI.
use std::{path::Path, time::Duration};

use serde_json::{Value, json};
use solidcraft_kicad::{
    cad,
    config::Settings,
    error::{Error, Result},
    inventory,
    staging::{Exporter, Staged},
};
use solidcraft_mcp::{Backend, Headless};

#[cfg(unix)]
fn write_mesh_fixture(path: &Path, missing: bool, shared: bool) -> Result<()> {
    let positions: [[f32; 3]; 8] = [
        [0., 0., 0.],
        [0.002, 0., 0.],
        [0.002, 0., -0.003],
        [0., 0., -0.003],
        [0., 0.004, 0.],
        [0.002, 0.004, 0.],
        [0.002, 0.004, -0.003],
        [0., 0.004, -0.003],
    ];
    let indices: [u32; 36] = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6,
        3, 0, 4, 3, 4, 7,
    ];
    let mut bin = Vec::new();
    for value in positions.into_iter().flatten() {
        bin.extend(value.to_le_bytes());
    }
    for value in indices {
        bin.extend(value.to_le_bytes());
    }
    let children = if missing {
        json!([1, 2])
    } else {
        json!([1, 2, 3])
    };
    let mut document = json!({
        "asset":{"version":"2.0"}, "scene":0, "scenes":[{"nodes":[0]}],
        "nodes":[{"children":children},{"name":"PCB","mesh":0},{"name":"R1","mesh":1,"translation":[0.01,0,-0.02]},{"name":"U1","children":[4]},{"name":"model","mesh":2}],
        "meshes": (0..3).map(|i| json!({"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":i}]})).collect::<Vec<_>>(),
        "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.,0.5,0.,1.]}},{"pbrMetallicRoughness":{"baseColorFactor":[0.1,0.2,0.3,1.]}},{"pbrMetallicRoughness":{"baseColorFactor":[0.8,0.4,0.,1.]}}],
        "buffers":[{"byteLength":bin.len()}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":96},{"buffer":0,"byteOffset":96,"byteLength":144}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":8,"type":"VEC3","min":[0,0,-0.003],"max":[0.002,0.004,0]},{"bufferView":1,"componentType":5125,"count":36,"type":"SCALAR"}]
    });
    if shared {
        document["materials"][2] = document["materials"][1].clone();
        document["nodes"][3]["rotation"] = json!([
            0.,
            std::f64::consts::FRAC_1_SQRT_2,
            0.,
            std::f64::consts::FRAC_1_SQRT_2
        ]);
        document["nodes"][3]["translation"] = json!([-0.005, 0.007, 0.011]);
    }
    std::fs::write(
        path,
        solidcraft_engine::io::glb_container(&serde_json::to_vec(&document)?, &bin),
    )?;
    Ok(())
}

#[cfg(unix)]
fn fixture(missing: bool) -> Result<(tempfile::TempDir, Staged)> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("board.txt");
    std::fs::write(
        &input,
        r#"(kicad_pcb (version 20260101) (generator test)
        (general (thickness 1))
        (footprint "test" (property "Reference" "U1") (model "${TEST_MODELS}/one.step"))
        (footprint "test" (property "Reference" "R1") (model "${TEST_MODELS}/two.step")))"#,
    )?;
    let mut backend = Headless::default();
    cad::execute(
        &mut backend,
        "solid.box",
        json!({"length": 2, "width": 3, "height": 4}),
    )?;
    cad::execute(
        &mut backend,
        "file.export",
        json!({"path": dir.path().join("source.step")}),
    )?;
    let script = dir.path().join("fake-kicad");
    write_mesh_fixture(&dir.path().join("batch.glb"), false, false)?;
    write_mesh_fixture(&dir.path().join("batch-missing.glb"), true, false)?;
    std::fs::write(
        &script,
        r##"#!/bin/sh
printf '%s\n' "$*" >> "$(dirname "$0")/calls"
if [ "$1" = version ]; then printf '10.0.7\n'; exit 0; fi
format="$3"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) shift; output="$1";;
    --component-filter) shift; reference="$1";;
    --board-only) board_only=1;;
  esac
  shift
done
if [ -z "$reference" ] && [ -z "$board_only" ]; then
  if [ -e "$(dirname "$0")/missing" ]; then
    printf 'Could not add 3D model for U1.\nFile not found: ${TEST_MODELS}/one.step\n'
    cp "$(dirname "$0")/batch-missing.$format" "$output"
  else
    cp "$(dirname "$0")/batch.$format" "$output"
  fi
  exit 0
fi
if [ -e "$(dirname "$0")/missing" ] && [ "$reference" = U1 ]; then
  printf 'Could not add 3D model for U1.\nFile not found: ${TEST_MODELS}/one.step\n'
  printf 'invalid empty geometry\n' > "$output"
  exit 0
fi
cp "$(dirname "$0")/source.step" "$output"
"##,
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    if missing {
        std::fs::write(dir.path().join("missing"), "")?;
    }
    let settings = Settings::default();
    let exporter = Exporter {
        program: &script,
        timeout: Duration::from_secs(5),
        settings: &settings,
    };
    let stage = exporter.stage(inventory::read(&input)?, missing)?;
    Ok((dir, stage))
}

#[test]
#[cfg(unix)]
fn identical_models_share_geometry_but_keep_independent_placements() -> Result<()> {
    let (dir, _legacy_stage) = fixture(false)?;
    write_mesh_fixture(&dir.path().join("batch.glb"), false, true)?;
    let settings = Settings::default();
    let program = dir.path().join("fake-kicad");
    let stage = Exporter {
        program: &program,
        timeout: Duration::from_secs(5),
        settings: &settings,
    }
    .stage_build(inventory::read(&dir.path().join("board.txt"))?, false)?;
    let native = dir.path().join("instances.solidcraft");
    cad::build_with_measurements(&stage, &native, false)?;
    drop(stage);
    let mut b = Headless::default();
    cad::execute(&mut b, "doc.open", json!({"path": native}))?;
    assert_eq!(
        b.session.doc.features.len(),
        2,
        "duplicate model geometry was stored twice"
    );
    let parent = b
        .session
        .doc
        .occurrences
        .iter()
        .find(|o| o.name == "board:1")
        .ok_or_else(|| Error::message("missing parent"))?
        .id;
    let r = b
        .session
        .doc
        .occurrences
        .iter()
        .find(|o| o.name == "R1:1")
        .ok_or_else(|| Error::message("missing R1"))?
        .clone();
    let u = b
        .session
        .doc
        .occurrences
        .iter()
        .find(|o| o.name == "U1:1")
        .ok_or_else(|| Error::message("missing U1"))?
        .clone();
    assert_eq!(r.component, u.component);
    let world = b.session.world_state();
    assert_eq!(world.bodies.len(), 3);
    let bounds: Vec<_> = world
        .bodies
        .iter()
        .filter(|body| body.component == r.component)
        .map(|body| body.mesh().bounds())
        .collect();
    assert_eq!(bounds.len(), 2);
    assert!(
        bounds
            .iter()
            .any(|bounds| (bounds.min.x - 10.).abs() < 1e-6 && bounds.min.z.abs() < 1e-6)
    );
    assert!(
        bounds
            .iter()
            .any(|bounds| (bounds.min.x + 8.).abs() < 1e-6 && (bounds.min.z - 7.).abs() < 1e-6)
    );
    cad::execute(
        &mut b,
        "browser.visibility",
        json!({"paths": [[parent, u.id]], "visible": false}),
    )?;
    assert_eq!(
        b.session.visibility.hidden_occurrences,
        vec![vec![parent, u.id]]
    );
    cad::execute(
        &mut b,
        "occurrence.move",
        json!({"occurrence": "U1:1", "translate": [0,0,5]}),
    )?;
    assert_eq!(
        b.session
            .doc
            .occurrences
            .iter()
            .find(|o| o.id == r.id)
            .map(|o| o.transform),
        Some(r.transform)
    );
    let moved = b.session.world_state();
    let bounds: Vec<_> = moved
        .bodies
        .iter()
        .filter(|body| body.component == r.component)
        .map(|body| body.mesh().bounds())
        .collect();
    assert!(bounds.iter().any(|bounds| bounds.min.z.abs() < 1e-6));
    assert!(
        bounds
            .iter()
            .any(|bounds| (bounds.min.z - 12.).abs() < 1e-6)
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn native_parts_reopen_without_assets_and_move_together() -> Result<()> {
    let (dir, _legacy_stage) = fixture(false)?;
    std::fs::write(dir.path().join("calls"), "")?;
    let settings = Settings::default();
    let program = dir.path().join("fake-kicad");
    let stage = Exporter {
        program: &program,
        timeout: Duration::from_secs(5),
        settings: &settings,
    }
    .stage_build(inventory::read(&dir.path().join("board.txt"))?, false)?;
    let calls = std::fs::read_to_string(dir.path().join("calls"))?;
    assert_eq!(
        calls
            .lines()
            .filter(|l| l.starts_with("pcb export glb "))
            .count(),
        1
    );
    let output = dir.path().join("board.solidcraft");
    let report = cad::build(&stage, &output)?;
    assert_eq!(report["measurement"]["body_count"], 3);
    assert!(cad::build(&stage, &output).is_err());
    drop(stage);
    for file in ["source.step", "batch.glb", "batch-missing.glb"] {
        std::fs::remove_file(dir.path().join(file))?;
    }
    let mut b = Headless::default();
    cad::execute(&mut b, "doc.open", json!({"path": output}))?;
    let components = cad::execute(&mut b, "component.list", json!({}))?;
    let names: Vec<_> = components["components"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["name"].as_str())
        .collect();
    for name in ["board", "PCB", "U1", "R1"] {
        assert!(names.contains(&name));
    }
    assert!(cad::inspect(&mut b)?["metadata"][cad::METADATA_KEY].is_object());
    let state = b.session.world_state();
    let colors: std::collections::BTreeSet<_> = state
        .bodies
        .iter()
        .filter_map(|body| body.body.color().map(|color| color.map(f32::to_bits)))
        .collect();
    assert_eq!(colors.len(), 3, "component colours were lost");
    for name in ["PCB", "R1", "U1"] {
        let component = b
            .session
            .doc
            .components
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| Error::message("missing component"))?;
        assert_eq!(
            state
                .bodies
                .iter()
                .filter(|body| b.session.doc.model_body_component(body) == component.id)
                .count(),
            1
        );
    }
    let board_occurrence = b
        .session
        .doc
        .occurrences
        .iter()
        .find(|o| o.name == "board:1")
        .ok_or_else(|| Error::message("missing board occurrence"))?
        .id;
    let part_occurrence = b
        .session
        .doc
        .occurrences
        .iter()
        .find(|o| o.name == "R1:1")
        .ok_or_else(|| Error::message("missing R1 occurrence"))?
        .id;
    let path = vec![board_occurrence, part_occurrence];
    cad::execute(
        &mut b,
        "browser.visibility",
        json!({"paths": [&path], "visible": false}),
    )?;
    assert_eq!(b.session.visibility.hidden_occurrences.len(), 1);
    cad::execute(
        &mut b,
        "browser.visibility",
        json!({"paths": [&path], "visible": true}),
    )?;
    assert!(b.session.visibility.hidden_occurrences.is_empty());
    assert_eq!(
        cad::execute(&mut b, "component.links", json!({}))?["links"],
        json!([])
    );
    let before = cad::execute(&mut b, "inspect.measure", json!({}))?;
    cad::execute(
        &mut b,
        "occurrence.move",
        json!({"occurrence": "board:1", "translate": [10,20,30]}),
    )?;
    let after = cad::execute(&mut b, "inspect.measure", json!({}))?;
    let a = before["bodies"]
        .as_array()
        .ok_or_else(|| Error::message("no measured bodies"))?;
    let z = after["bodies"]
        .as_array()
        .ok_or_else(|| Error::message("no measured bodies"))?;
    for (one, two) in a.iter().zip(z) {
        for side in ["min", "max"] {
            for (index, delta) in [10., 20., 30.].into_iter().enumerate() {
                let old = one["bbox"][side][index]
                    .as_f64()
                    .ok_or_else(|| Error::message("invalid bound"))?;
                let new = two["bbox"][side][index]
                    .as_f64()
                    .ok_or_else(|| Error::message("invalid bound"))?;
                assert!((new - old - delta).abs() < 1e-6);
            }
        }
    }
    let part = b
        .session
        .doc
        .components
        .iter()
        .find(|c| c.name == "R1")
        .ok_or_else(|| Error::message("missing R1 component"))?
        .id;
    let part_body = b
        .session
        .world_state()
        .bodies
        .iter()
        .find(|body| b.session.doc.model_body_component(body) == part)
        .ok_or_else(|| Error::message("missing R1 body"))?
        .name
        .clone();
    assert!(b.session.doc.assembly.joints.is_empty());
    cad::execute(
        &mut b,
        "occurrence.move",
        json!({"occurrence": "R1:1", "translate": [5,0,0]}),
    )?;
    let moved = cad::execute(&mut b, "inspect.measure", json!({}))?;
    for body in z {
        let name = body["name"]
            .as_str()
            .ok_or_else(|| Error::message("missing body name"))?;
        let current = moved["bodies"]
            .as_array()
            .and_then(|bodies| bodies.iter().find(|b| b["name"] == name))
            .ok_or_else(|| Error::message("body disappeared"))?;
        let old = body["bbox"]["min"][0]
            .as_f64()
            .ok_or_else(|| Error::message("missing bound"))?;
        let new = current["bbox"]["min"][0]
            .as_f64()
            .ok_or_else(|| Error::message("missing bound"))?;
        assert!((new - old - if name == part_body { 5.0 } else { 0.0 }).abs() < 1e-6);
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn cli_build_is_quiet_by_default_and_json_is_opt_in() -> Result<()> {
    let (dir, _stage) = fixture(false)?;
    let working_dir = tempfile::tempdir()?;
    let board = dir.path().join("board.txt");
    for report in [false, true] {
        let output = if report {
            dir.path().join("custom.solidcraft")
        } else {
            working_dir.path().join("board.solidcraft")
        };
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-kicad"));
        command
            .current_dir(working_dir.path())
            .arg("build")
            .arg(&board)
            .arg("--kicad-cli")
            .arg(dir.path().join("fake-kicad"))
            .arg("--model-prefix")
            .arg(format!("TEST_MODELS={}", dir.path().display()))
            .env("XDG_CONFIG_HOME", dir.path());
        if report {
            command
                .arg("--output")
                .arg(&output)
                .arg("--json")
                .arg("--measure");
        }
        let result = command.output()?;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(output.is_file());
        assert!(!board.with_extension("solidcraft").exists());
        if report {
            let value: Value = serde_json::from_slice(&result.stdout)?;
            assert_eq!(value["result"]["output"], json!(output));
            assert_eq!(value["result"]["measurement"]["body_count"], 3);
            assert_eq!(value["parts"], json!(["PCB", "R1", "U1"]));
        } else {
            assert!(result.stdout.is_empty(), "unexpected JSON report on stdout");
        }
        assert!(
            result.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let original = std::fs::read(&output)?;
        let repeated = command.output()?;
        assert!(!repeated.status.success());
        assert!(repeated.stdout.is_empty());
        assert!(String::from_utf8_lossy(&repeated.stderr).contains("output already exists"));
        assert_eq!(std::fs::read(&output)?, original);

        // Replacing the file must not truncate readers of its previous inode.
        std::fs::write(&output, b"previous output")?;
        let snapshot = output.with_extension("snapshot");
        std::fs::hard_link(&output, &snapshot)?;
        let forced = command
            .arg(if report { "--force" } else { "-f" })
            .output()?;
        assert!(
            forced.status.success(),
            "{}",
            String::from_utf8_lossy(&forced.stderr)
        );
        assert_eq!(std::fs::read(&snapshot)?, b"previous output");
        assert!(forced.stderr.is_empty());
        if report {
            let value: Value = serde_json::from_slice(&forced.stdout)?;
            assert_eq!(value["result"]["output"], json!(output));
        } else {
            assert!(forced.stdout.is_empty());
        }
        let mut b = Headless::default();
        cad::execute(&mut b, "doc.open", json!({"path": output}))?;
        assert_eq!(b.session.world_state().bodies.len(), 3);
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn failed_forced_build_preserves_existing_output() -> Result<()> {
    let (dir, _stage) = fixture(true)?;
    let board = dir.path().join("board.txt");
    let output = board.with_extension("solidcraft");
    std::fs::write(&output, b"previous output")?;
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-kicad"))
        .current_dir(dir.path())
        .arg("build")
        .arg(&board)
        .arg("--force")
        .arg("--strict-models")
        .arg("--kicad-cli")
        .arg(dir.path().join("fake-kicad"))
        .arg("--model-prefix")
        .arg(format!("TEST_MODELS={}", dir.path().display()))
        .env("XDG_CONFIG_HOME", dir.path())
        .output()?;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("model"));
    assert!(result.stdout.is_empty());
    assert_eq!(std::fs::read(output)?, b"previous output");
    Ok(())
}

#[test]
#[cfg(unix)]
fn quiet_cli_still_reports_errors_and_missing_model_warnings() -> Result<()> {
    let (dir, _stage) = fixture(true)?;
    for allow_missing in [false, true] {
        let output = dir
            .path()
            .join(format!("partial-{allow_missing}.solidcraft"));
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-kicad"));
        command
            .arg("build")
            .arg(dir.path().join("board.txt"))
            .arg("--output")
            .arg(&output)
            .arg("--kicad-cli")
            .arg(dir.path().join("fake-kicad"))
            .env("XDG_CONFIG_HOME", dir.path());
        if !allow_missing {
            command.arg("--strict-models");
        }
        let result = command.output()?;
        assert_eq!(result.status.success(), allow_missing);
        assert_eq!(output.is_file(), allow_missing);
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.contains("U1"), "{stderr}");
        assert!(
            stderr.starts_with(if allow_missing { "Warning:" } else { "error:" }),
            "{stderr}"
        );
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn zero_exit_missing_models_fail_unless_explicitly_bypassed() -> Result<()> {
    let (dir, stage) = fixture(true)?;
    assert_eq!(stage.assets.len(), 2);
    assert!(!stage.omissions.is_empty());
    let settings = Settings::default();
    let program = dir.path().join("fake-kicad");
    let exporter = Exporter {
        program: &program,
        timeout: Duration::from_secs(5),
        settings: &settings,
    };
    let error = exporter
        .stage(inventory::read(&dir.path().join("board.txt"))?, false)
        .err()
        .ok_or_else(|| Error::message("missing model incorrectly succeeded"))?;
    let message = error.to_string();
    assert!(message.contains("U1"));
    assert!(message.contains("${TEST_MODELS}/one.step"));
    Ok(())
}

#[test]
fn failed_import_rolls_back_existing_geometry_and_history() -> Result<()> {
    let mut b = Headless::default();
    cad::execute(
        &mut b,
        "solid.box",
        json!({"length":2,"width":3,"height":4}),
    )?;
    let before = cad::execute(&mut b, "inspect.measure", json!({}))?;
    let undo = cad::inspect(&mut b)?["undo"].clone();
    let steps = vec![
        json!({"command":"component.create","params":{"name":"failed","activate":false}}),
        json!({"command":"solid.box","params":{"length":10,"width":10,"height":10}}),
        json!({"command":"component.insert_design","params":{"path":"/nonexistent-solidcraft-kicad-test.solidcraft"}}),
    ];
    assert!(cad::offline_batch(&mut b, &steps).is_err());
    assert_eq!(cad::execute(&mut b, "inspect.measure", json!({}))?, before);
    assert_eq!(cad::inspect(&mut b)?["undo"], undo);
    assert_eq!(
        cad::execute(&mut b, "component.list", json!({}))?["components"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    Ok(())
}

struct App {
    inner: Headless,
    batches: usize,
    lose_reply: bool,
}

impl Backend for App {
    fn call(&mut self, method: &str, params: Value) -> std::result::Result<Value, String> {
        match method {
            "ui.help" => {
                assert_eq!(
                    params,
                    json!({}),
                    "version checks must not open the About window"
                );
                Ok(json!({"version":"0.14.0"}))
            }
            "ui.inspect" => Ok(json!({"tool":null,"dialog":null})),
            "ui.documents" => {
                Ok(json!({"active":0,"documents":[{"name":"assembly","dirty":true}]}))
            }
            "engine.batch" => {
                self.batches += 1;
                let commands = params["commands"].as_array().ok_or("missing commands")?;
                let result =
                    cad::offline_batch(&mut self.inner, commands).map_err(|e| e.to_string())?;
                if self.lose_reply {
                    Err("connection lost after dispatch".into())
                } else {
                    Ok(result)
                }
            }
            _ => self.inner.call(method, params),
        }
    }
    fn has_ui(&self) -> bool {
        true
    }
    fn describe(&self) -> String {
        "isolated test app".into()
    }
}

#[test]
#[cfg(unix)]
fn live_recipe_handles_name_collisions_without_changing_author_context() -> Result<()> {
    let (_dir, mut stage) = fixture(false)?;
    let mut app = App {
        inner: Headless::default(),
        batches: 0,
        lose_reply: false,
    };
    cad::execute(
        &mut app,
        "solid.box",
        json!({"length":1,"width":1,"height":1}),
    )?;
    cad::execute(&mut app, "component.create", json!({"name":"board"}))?;
    let before = cad::destination(&mut app)?;
    let active = before.components["active"].clone();
    let result = cad::insert(&mut app, &before, &mut stage)?;
    assert_eq!(result["component_name"], "board (2)");
    assert_eq!(app.batches, 1);
    let components = cad::execute(&mut app, "component.list", json!({}))?;
    assert_eq!(components["active"], active);
    assert_eq!(
        cad::execute(&mut app, "inspect.measure", json!({}))?["body_count"],
        4
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn destination_changes_abort_and_lost_replies_are_never_replayed() -> Result<()> {
    let (_dir, mut stage) = fixture(false)?;
    let mut app = App {
        inner: Headless::default(),
        batches: 0,
        lose_reply: false,
    };
    let before = cad::destination(&mut app)?;
    cad::execute(
        &mut app,
        "solid.box",
        json!({"length":1,"width":1,"height":1}),
    )?;
    assert!(cad::insert(&mut app, &before, &mut stage).is_err());
    assert_eq!(app.batches, 0);
    let before = cad::destination(&mut app)?;
    app.lose_reply = true;
    assert!(cad::insert(&mut app, &before, &mut stage).is_err());
    assert_eq!(app.batches, 1);
    let folder = stage
        .assets
        .first()
        .and_then(|a| a.path.parent())
        .ok_or_else(|| Error::message("no staging folder"))?
        .to_path_buf();
    assert!(folder.exists());
    // The isolated app finished its simulated operation; its retained fixtures can now go.
    std::fs::remove_dir_all(folder)?;
    Ok(())
}

#[test]
#[ignore = "requires the user's GPS_BOARD and installed KiCad 10/model libraries"]
fn gps_mesh_acceptance() -> Result<()> {
    let board = std::env::var_os("GPS_BOARD").ok_or_else(|| Error::message("set GPS_BOARD"))?;
    let settings = Settings::default();
    let exporter = Exporter {
        program: Path::new("kicad-cli"),
        timeout: Duration::from_secs(120),
        settings: &settings,
    };
    let stage = exporter.stage_build(inventory::read(Path::new(&board))?, false)?;
    assert_eq!(stage.assets.len(), 7);
    let output = tempfile::tempdir()?;
    let native = output.path().join("gps.solidcraft");
    cad::build_with_measurements(&stage, &native, false)?;
    drop(stage);
    let mut b = Headless::default();
    cad::execute(&mut b, "doc.open", json!({"path": native}))?;
    let state = b.session.world_state();
    assert!(state.bodies.iter().all(|body| body.body.is_mesh()));
    assert_eq!(b.session.doc.occurrences.len(), 8);
    assert!(b.session.doc.components.len() <= 8);
    let measured = cad::execute(&mut b, "inspect.measure", json!({}))?;
    let bodies = measured["bodies"]
        .as_array()
        .ok_or_else(|| Error::message("missing mesh measurements"))?;
    for (axis, (low, high)) in [(146.270375, 164.270375), (-95., -79.), (-4.685, 7.795)]
        .into_iter()
        .enumerate()
    {
        let min = bodies
            .iter()
            .filter_map(|b| b["bbox"]["min"][axis].as_f64())
            .fold(f64::INFINITY, f64::min);
        let max = bodies
            .iter()
            .filter_map(|b| b["bbox"]["max"][axis].as_f64())
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (min - low).abs() < 0.001 && (max - high).abs() < 0.001,
            "mesh placement changed on axis {axis}: {min}..{max}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires the user's GPS_BOARD and installed KiCad 10/model libraries"]
fn gps_acceptance() -> Result<()> {
    let board = std::env::var_os("GPS_BOARD").ok_or_else(|| Error::message("set GPS_BOARD"))?;
    let settings = Settings::default();
    let exporter = Exporter {
        program: Path::new("kicad-cli"),
        timeout: Duration::from_secs(120),
        settings: &settings,
    };
    let stage = exporter.stage(inventory::read(Path::new(&board))?, false)?;
    let output = tempfile::tempdir()?;
    let native = output.path().join("gps.solidcraft");
    let result = cad::build(&stage, &native)?;
    assert_eq!(stage.assets.len(), 7);
    assert_eq!(result["measurement"]["body_count"], 35);
    let measured = result["measurement"]["bodies"]
        .as_array()
        .ok_or_else(|| Error::message("missing GPS measurements"))?;
    for (index, (low, high)) in [(146.270375, 164.270375), (-95., -79.), (-4.685, 7.795)]
        .into_iter()
        .enumerate()
    {
        let minimum = measured
            .iter()
            .filter_map(|b| b["bbox"]["min"][index].as_f64())
            .fold(f64::INFINITY, f64::min);
        let maximum = measured
            .iter()
            .filter_map(|b| b["bbox"]["max"][index].as_f64())
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (minimum - low).abs() < 1e-6 && (maximum - high).abs() < 1e-6,
            "GPS placement changed on axis {index}"
        );
    }
    let volume = result["measurement"]["total"]["volume_mm3"]
        .as_f64()
        .ok_or_else(|| Error::message("missing volume"))?;
    assert!((volume - 1740.0829945).abs() < 1e-5);
    drop(stage);
    let mut b = Headless::default();
    cad::execute(&mut b, "doc.open", json!({"path":native}))?;
    assert_eq!(
        cad::execute(&mut b, "inspect.measure", json!({}))?["body_count"],
        35
    );
    assert_eq!(
        cad::execute(&mut b, "component.links", json!({}))?["links"],
        json!([])
    );
    Ok(())
}
