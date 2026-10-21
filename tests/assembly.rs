//! Analytic geometry fixtures exercise orchestration without requiring KiCad or a GUI.
use std::{path::Path, time::Duration};

use serde_json::{Value, json};
use solidcraft_kicad::{
    cad,
    config::Settings,
    error::{Error, Result},
    inventory,
    staging::{BoardLayers, Exporter, Staged},
};
use solidcraft_mcp::{Backend, Headless};

#[cfg(unix)]
fn write_mesh_fixture(path: &Path, missing: bool, shared: bool) -> Result<()> {
    let (document, bin) = mesh_fixture(missing, shared);
    write_gltf(path, &document, &bin)
}

#[cfg(unix)]
fn mesh_fixture(missing: bool, shared: bool) -> (Value, Vec<u8>) {
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
    let children = if missing { json!([2]) } else { json!([2, 3]) };
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
    (document, bin)
}

#[cfg(unix)]
fn write_gltf(path: &Path, document: &Value, bin: &[u8]) -> Result<()> {
    std::fs::write(
        path,
        solidcraft_engine::io::glb_container(&serde_json::to_vec(document)?, bin),
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
    let (mut board, bin) = mesh_fixture(false, false);
    board["nodes"][0]["children"] = json!([1]);
    board["materials"][0]["pbrMetallicRoughness"]["metallicFactor"] = json!(0.);
    board["materials"][0]["pbrMetallicRoughness"]["roughnessFactor"] = json!(0.8);
    write_gltf(&dir.path().join("board.glb"), &board, &bin)?;
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
if [ "$format" = svg ]; then
  cp "$(dirname "$0")/copper.svg" "$output"
  exit 0
fi
if [ "$format" = glb ] && [ -n "$board_only" ]; then
  cp "$(dirname "$0")/board.glb" "$output"
  exit 0
fi
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
    std::fs::write(
        dir.path().join("copper.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="30mm" viewBox="0 0 20 30"/>"#,
    )?;
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

#[cfg(unix)]
fn assert_same_native_archive_payloads(
    left: &[u8],
    right: &[u8],
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::{collections::BTreeMap, io::Read};

    type Entries = BTreeMap<String, (zip::CompressionMethod, Vec<u8>)>;

    fn entries(bytes: &[u8]) -> std::result::Result<Entries, Box<dyn std::error::Error>> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        let mut entries = BTreeMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name()?.into_owned();
            let method = entry.compression();
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            if name == "cache/model.json" {
                let mut model: Value = serde_json::from_slice(&data)?;
                if let Some(features) = model
                    .pointer_mut("/model/features")
                    .and_then(Value::as_array_mut)
                {
                    for feature in features {
                        if let Some(fields) = feature.as_object_mut() {
                            fields.remove("ms");
                        }
                    }
                }
                data = serde_json::to_vec(&model)?;
            }
            entries.insert(name, (method, data));
        }
        Ok(entries)
    }

    let left = entries(left)?;
    let right = entries(right)?;
    assert_eq!(
        left.keys().collect::<Vec<_>>(),
        right.keys().collect::<Vec<_>>(),
        "native archive entry lists differ"
    );
    for (name, (left_method, left_data)) in &left {
        let (right_method, right_data) = right
            .get(name)
            .ok_or_else(|| format!("missing native archive entry {name}"))?;
        assert_eq!(left_method, right_method, "compression changed for {name}");
        assert!(
            left_data == right_data,
            "payload changed for {name} after normalizing SolidCraft feature timings"
        );
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn both_build_paths_preserve_native_documents_and_report_compression()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::{cell::RefCell, fs::File};

    let (dir, legacy) = fixture(false)?;
    let settings = Settings::default();
    let program = dir.path().join("fake-kicad");
    let prepared = Exporter {
        program: &program,
        timeout: Duration::from_secs(5),
        settings: &settings,
    }
    .stage_build(inventory::read(&dir.path().join("board.txt"))?, false)?;
    for (path, stage) in [("legacy", &legacy), ("prepared", &prepared)] {
        let mut documents = Vec::new();
        for compression in [cad::Compression::Compact, cad::Compression::Fast] {
            let output = dir
                .path()
                .join(format!("{path}-{compression:?}"))
                .join("board.solidcraft");
            let stages = RefCell::new(Vec::new());
            cad::build_with_progress(stage, &output, false, false, compression, &|label| {
                assert!(!output.exists(), "published before all stages completed");
                stages.borrow_mut().push(label);
            })?;
            let expected = match compression {
                cad::Compression::Compact => {
                    vec!["Compressing SolidCraft geometry", "Writing SolidCraft file"]
                }
                cad::Compression::Fast => {
                    vec!["Storing SolidCraft geometry", "Writing SolidCraft file"]
                }
            };
            assert_eq!(stages.into_inner(), expected);
            documents.push(solidcraft_engine::io::read_design(&std::fs::read(
                &output,
            )?)?);
            let mut backend = Headless::default();
            cad::execute(&mut backend, "doc.open", json!({"path": output}))?;
            assert_eq!(cad::validate(&mut backend)?["body_count"], 3);
            let mut archive = zip::ZipArchive::new(File::open(&output)?)?;
            for index in 0..archive.len() {
                let entry = archive.by_index(index)?;
                let name = entry.name()?.into_owned();
                if name.starts_with("blobs/") {
                    assert_eq!(
                        entry.compression(),
                        if compression == cad::Compression::Compact {
                            zip::CompressionMethod::Deflated
                        } else {
                            zip::CompressionMethod::Stored
                        },
                        "unexpected compression for {name} in {path} with {compression:?}"
                    );
                }
            }
            let old = std::fs::read(&output)?;
            assert!(
                cad::build_with_compression(stage, &output, false, false, compression).is_err()
            );
            assert_eq!(std::fs::read(&output)?, old);
            cad::build_with_compression(stage, &output, false, true, compression)?;
            let rebuilt = std::fs::read(&output)?;
            assert_same_native_archive_payloads(&rebuilt, &old)?;
        }
        assert_eq!(documents[0], documents[1]);
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn board_layers_retain_both_side_colors_finish_and_batched_geometry() -> Result<()> {
    let (dir, _stage) = fixture(false)?;
    let (mut board, mut bin) = mesh_fixture(false, false);
    board["nodes"] = json!([{"children":[1,2,3,4,5,6]},
        {"name":"PCB", "mesh":0},
        {"name":"=>[Pads]", "mesh":1},
        {"name":"=>[Top Soldermask]", "mesh":2},
        {"name":"=>[Bottom Soldermask]", "mesh":3},
        {"name":"=>[Top Silkscreen]", "mesh":4},
        {"name":"=>[Bottom Silkscreen]", "mesh":5}]);
    board["materials"] = json!([
        {"pbrMetallicRoughness":{"baseColorFactor":[0.,0.5,0.,1.],"metallicFactor":0.,"roughnessFactor":0.8}},
        {"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.5,0.5,1.],"metallicFactor":1.,"roughnessFactor":0.4}},
        {"alphaMode":"BLEND","pbrMetallicRoughness":{"baseColorFactor":[0.,0.,1.,0.83]}},
        {"alphaMode":"BLEND","pbrMetallicRoughness":{"baseColorFactor":[1.,0.,0.,0.83]}},
        {"alphaMode":"BLEND","pbrMetallicRoughness":{"baseColorFactor":[1.,1.,1.,0.9]}},
        {"alphaMode":"BLEND","pbrMetallicRoughness":{"baseColorFactor":[1.,1.,0.,0.9]}}
    ]);
    board["meshes"] = json!((0..6).map(|material| {
        let primitive = json!({"attributes":{"POSITION":0},"indices":1,"material":material});
        // A pad export contains many face primitives sharing one material.
        json!({"primitives": if material == 1 { vec![primitive; 4] } else {vec![primitive]}})
    }).collect::<Vec<_>>());
    for index in 0..6 {
        board["materials"][index]["name"] = json!(format!("mat_{index}"));
        if index >= 2 {
            board["materials"][index]["pbrMetallicRoughness"]["metallicFactor"] = json!(0.);
            board["materials"][index]["pbrMetallicRoughness"]["roughnessFactor"] =
                json!(if index >= 4 { 0.9 } else { 0.6 });
        }
    }
    // Ink planes on both sides test the minimal viewport display separation.
    for (mesh, y) in [(4, 0.006_f32), (5, -0.001_f32)] {
        let offset = bin.len();
        for _ in 0..2 {
            for (x, z) in [(0_f32, 0_f32), (0.002, 0.), (0.002, -0.003), (0., -0.003)] {
                for value in [x, y, z] {
                    bin.extend(value.to_le_bytes());
                }
            }
        }
        let view = board["bufferViews"].as_array().map_or(0, Vec::len);
        let accessor = board["accessors"].as_array().map_or(0, Vec::len);
        board["bufferViews"]
            .as_array_mut()
            .ok_or_else(|| Error::message("missing buffer views"))?
            .push(json!({"buffer":0,"byteOffset":offset,"byteLength":96}));
        board["accessors"].as_array_mut().ok_or_else(|| Error::message("missing accessors"))?.push(json!({"bufferView":view,"componentType":5126,"count":8,"type":"VEC3","min":[0,y,-0.003],"max":[0.002,y,0]}));
        board["meshes"][mesh]["primitives"][0]["attributes"]["POSITION"] = json!(accessor);
    }
    board["buffers"][0]["byteLength"] = json!(bin.len());
    write_gltf(&dir.path().join("board.glb"), &board, &bin)?;
    let input = dir.path().join("board.txt");
    let source = std::fs::read_to_string(&input)?;
    let program = dir.path().join("fake-kicad");
    let settings = Settings::default();
    let exporter = Exporter {
        program: &program,
        settings: &settings,
        timeout: Duration::from_secs(5),
    };
    for (finish, color) in [
        ("ENIG", [178, 156, 0]),
        ("HASL lead-free", [160, 160, 160]),
        ("OSP", [184, 115, 50]),
        ("Immersion silver", [213, 213, 213]),
        ("ENEPIG", [178, 156, 0]),
    ] {
        std::fs::write(
            &input,
            source.replace(
                "(general",
                &format!("(setup (stackup (copper_finish \"{finish}\"))) (general"),
            ),
        )?;
        let inventory = inventory::read(&input)?;
        assert_eq!(inventory.copper_finish.as_deref(), Some(finish));
        let stage = exporter.stage_build(inventory, false)?;
        let native = dir.path().join(format!("{finish}.solidcraft"));
        cad::build_with_measurements(&stage, &native, false)?;
        drop(stage);
        let mut backend = Headless::default();
        cad::execute(&mut backend, "doc.open", json!({"path":native}))?;
        let state = backend.session.world_state();
        assert_eq!(
            state.bodies.len(),
            8,
            "face primitives must be batched by material"
        );
        let looks = &backend.session.doc.appearances.bodies;
        let plated = looks
            .values()
            .find(|l| l.name == finish)
            .ok_or_else(|| Error::message("missing finish appearance"))?;
        assert_eq!(plated.color, color);
        let pbr = plated
            .pbr
            .ok_or_else(|| Error::message("finish lost metallic appearance"))?;
        assert_eq!(pbr.metallic(), 1.);
        assert!((pbr.roughness() - 0.4).abs() < 1e-6);
        for color in [[0, 0, 255], [255, 0, 0], [255, 255, 255], [255, 255, 0]] {
            assert!(looks.values().any(|l| l.color == color && l.opacity == 1.0));
        }
        let pcb = backend
            .session
            .doc
            .components
            .iter()
            .find(|c| c.name == "PCB")
            .ok_or_else(|| Error::message("missing PCB"))?
            .id;
        let meshes: Vec<_> = state.bodies.iter().filter(|b| b.component == pcb).collect();
        assert_eq!(meshes.len(), 6);
        for feature in backend
            .session
            .doc
            .features
            .iter()
            .filter(|f| f.component == pcb)
        {
            let solidcraft_engine::doc::FeatureKind::MeshImport { meshes, .. } = &feature.kind
            else {
                assert!(matches!(
                    feature.kind,
                    solidcraft_engine::doc::FeatureKind::Import { .. }
                ));
                continue;
            };
            assert!(
                meshes.iter().all(|m| m.positions.len() <= 24),
                "shared face vertices should be stored once"
            );
            for (color, z) in [([255, 255, 255], 6.1), ([255, 255, 0], -1.1)] {
                let ink = meshes
                    .iter()
                    .find(|m| looks.get(&m.name).is_some_and(|l| l.color == color))
                    .ok_or_else(|| Error::message("missing ink plane"))?;
                assert!(
                    ink.positions
                        .iter()
                        .skip(2)
                        .step_by(3)
                        .all(|v| (v - z).abs() < 1e-6)
                );
            }
        }
    }
    let calls = std::fs::read_to_string(dir.path().join("calls"))?;
    let board_calls: Vec<_> = calls
        .lines()
        .filter(|l| l.starts_with("pcb export glb") && l.contains("--board-only"))
        .collect();
    assert_eq!(board_calls.len(), 5);
    for call in board_calls {
        for flag in [
            "--include-silkscreen",
            "--include-soldermask",
            "--include-pads",
        ] {
            assert!(call.contains(flag));
        }
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn copper_surfaces_keep_finish_and_drill_openings_after_reopening() -> Result<()> {
    let (dir, _legacy_stage) = fixture(false)?;
    let input = dir.path().join("board.txt");
    let source = std::fs::read_to_string(&input)?.replace(
        "(general",
        "(setup (stackup (copper_finish \"ENIG\"))) (general",
    );
    std::fs::write(&input, &source)?;
    std::fs::write(
        dir.path().join("copper.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="30mm" viewBox="0 0 20 30"><rect x="0" y="-3" width="2" height="3" fill="black"/><rect x="0.5" y="-2" width="1" height="1" fill="white"/></svg>"#,
    )?;
    let settings = Settings::default();
    let program = dir.path().join("fake-kicad");
    let stage = Exporter {
        program: &program,
        settings: &settings,
        timeout: Duration::from_secs(5),
    }
    .stage_build(inventory::read(&input)?, false)?;
    let native = dir.path().join("copper.solidcraft");
    cad::build_with_measurements(&stage, &native, false)?;
    drop(stage);
    std::fs::remove_file(dir.path().join("copper.svg"))?;
    let mut backend = Headless::default();
    cad::execute(&mut backend, "doc.open", json!({"path":native}))?;
    let mut planes = Vec::new();
    for feature in &backend.session.doc.features {
        if let solidcraft_engine::doc::FeatureKind::MeshImport { meshes, .. } = &feature.kind {
            for mesh in meshes {
                let Some(look) = backend
                    .session
                    .doc
                    .appearances
                    .bodies
                    .get(&mesh.name)
                    .filter(|l| l.name == "ENIG")
                else {
                    continue;
                };
                assert_eq!(look.color, [178, 156, 0]);
                assert!(look.pbr.is_some_and(|p| p.metallic() == 1.));
                let z = mesh.positions[2];
                assert!(
                    mesh.positions
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .all(|p| (p[2] - z).abs() < 1e-6)
                );
                let area: f64 = mesh
                    .triangles
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|t| {
                        let p: Vec<_> = t
                            .iter()
                            .map(|&i| &mesh.positions[i as usize * 3..i as usize * 3 + 3])
                            .collect();
                        let cross = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                            - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
                        assert_eq!(cross.is_sign_positive(), z > 0.);
                        cross.abs() * 0.5
                    })
                    .sum();
                assert!((area - 5.).abs() < 1e-4, "drill opening lost: area {area}");
                planes.push(z);
            }
        }
    }
    planes.sort_by(f64::total_cmp);
    assert_eq!(planes.len(), 2);
    assert!((planes[0] + 0.035).abs() < 1e-6 && (planes[1] - 4.035).abs() < 1e-6);
    assert_eq!(std::fs::read_to_string(input)?, source);
    Ok(())
}

#[test]
#[cfg(unix)]
fn board_builds_without_any_usable_component_models() -> Result<()> {
    let (dir, _stage) = fixture(false)?;
    let input = dir.path().join("board.txt");
    let (mut empty, bin) = mesh_fixture(false, false);
    empty["nodes"][0]["children"] = json!([]);
    write_gltf(&dir.path().join("batch.glb"), &empty, &bin)?;
    let program = dir.path().join("fake-kicad");
    let settings = Settings::default();
    let exporter = Exporter {
        program: &program,
        settings: &settings,
        timeout: Duration::from_secs(5),
    };
    let stage = exporter.stage_build(inventory::read(&input)?, true)?;
    assert_eq!(stage.assets.len(), 1);
    assert_eq!(stage.omissions.len(), 2);
    assert!(
        exporter
            .stage_build(inventory::read(&input)?, false)
            .is_err()
    );
    // No model assignments: skip the models-only subprocess altogether.
    std::fs::write(
        &input,
        "(kicad_pcb (version 20260101) (generator test) (general (thickness 1)))",
    )?;
    std::fs::write(dir.path().join("calls"), "")?;
    let stage = exporter.stage_build(inventory::read(&input)?, false)?;
    assert_eq!(stage.assets.len(), 1);
    assert!(stage.omissions.is_empty());
    let calls = std::fs::read_to_string(dir.path().join("calls"))?;
    assert_eq!(
        calls
            .lines()
            .filter(|l| l.starts_with("pcb export glb"))
            .count(),
        1
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn progress_reports_each_enabled_layer_once_across_concurrent_exports() -> Result<()> {
    let (dir, _stage) = fixture(false)?;
    let settings = Settings::default();
    let program = dir.path().join("fake-kicad");
    let exporter = Exporter {
        program: &program,
        settings: &settings,
        timeout: Duration::from_secs(5),
    };
    for layers in [
        BoardLayers::default(),
        BoardLayers {
            silkscreen: false,
            soldermask: false,
            pads: false,
            copper: false,
        },
    ] {
        let events = std::sync::Mutex::new(Vec::new());
        exporter.stage_build_with_progress(
            inventory::read(&dir.path().join("board.txt"))?,
            false,
            layers,
            &|label| {
                if let Ok(mut events) = events.lock() {
                    events.push(label);
                }
            },
        )?;
        let events = events
            .into_inner()
            .map_err(|_| Error::message("progress callback failed"))?;
        assert_eq!(
            events.len() + 2,
            layers.progress_steps(),
            "read/write are reported by the CLI"
        );
        let unique: std::collections::BTreeSet<_> = events.iter().copied().collect();
        assert_eq!(
            unique.len(),
            events.len(),
            "duplicate completion from parallel jobs"
        );
        for label in [
            "Front silkscreen exported",
            "Back silkscreen exported",
            "Front soldermask exported",
            "Back soldermask exported",
            "Pads and plating exported",
            "Front copper plotted",
            "Back copper plotted",
        ] {
            assert_eq!(unique.contains(label), layers.copper);
        }
        assert_eq!(
            events.last(),
            Some(&"Linked assembly and layer meshes prepared")
        );
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn cli_board_layer_switches_are_independent() -> Result<()> {
    let (dir, _stage) = fixture(false)?;
    for flag in [
        "--no-silkscreen",
        "--no-soldermask",
        "--no-pads",
        "--no-copper",
    ] {
        std::fs::write(dir.path().join("calls"), "")?;
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-kicad"))
            .arg("build")
            .arg(dir.path().join("board.txt"))
            .arg("--output")
            .arg(dir.path().join(format!("{flag}.solidcraft")))
            .arg("--kicad-cli")
            .arg(dir.path().join("fake-kicad"))
            .arg(flag)
            .env("XDG_CONFIG_HOME", dir.path())
            .output()?;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let calls = std::fs::read_to_string(dir.path().join("calls"))?;
        let copper: Vec<_> = calls
            .lines()
            .filter(|l| l.starts_with("pcb export svg"))
            .collect();
        assert_eq!(copper.len(), if flag == "--no-copper" { 0 } else { 2 });
        if !copper.is_empty() {
            assert!(copper[0].contains("--layers F.Cu"));
            assert!(copper[1].contains("--layers B.Cu"));
            assert!(copper.iter().all(|l| l.contains("--page-size-mode 1")
                && l.contains("--scale 1")
                && l.contains("--drill-shape-opt 2")));
        }
        let board = calls
            .lines()
            .find(|l| l.starts_with("pcb export glb") && l.contains("--board-only"))
            .ok_or_else(|| Error::message("board export missing"))?;
        for layer in ["silkscreen", "soldermask", "pads"] {
            assert_eq!(
                board.contains(&format!("--include-{layer}")),
                flag != format!("--no-{layer}")
            );
        }
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn models_with_different_material_responses_are_not_linked() -> Result<()> {
    let (dir, _stage) = fixture(false)?;
    let (mut document, bin) = mesh_fixture(false, true);
    document["materials"][1]["pbrMetallicRoughness"]["roughnessFactor"] = json!(0.2);
    document["materials"][2]["pbrMetallicRoughness"]["roughnessFactor"] = json!(0.9);
    write_gltf(&dir.path().join("batch.glb"), &document, &bin)?;
    let program = dir.path().join("fake-kicad");
    let settings = Settings::default();
    let stage = Exporter {
        program: &program,
        settings: &settings,
        timeout: Duration::from_secs(5),
    }
    .stage_build(inventory::read(&dir.path().join("board.txt"))?, false)?;
    let doc = &stage
        .assembly
        .as_ref()
        .ok_or_else(|| Error::message("missing assembly"))?
        .doc;
    assert_eq!(doc.features.len(), 3);
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
        2
    );
    let output = dir.path().join("board.solidcraft");
    let report = cad::build(&stage, &output)?;
    assert_eq!(report["measurement"]["body_count"], 3);
    assert!(cad::build(&stage, &output).is_err());
    drop(stage);
    for file in ["source.step", "batch.glb", "batch-missing.glb", "board.glb"] {
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
        .filter_map(|body| {
            b.session
                .doc
                .body_color(body)
                .map(|color| color.map(f32::to_bits))
        })
        .collect();
    assert_eq!(colors.len(), 3, "component colours were lost");
    let substrate = state
        .bodies
        .iter()
        .find(|body| !body.body.is_mesh())
        .ok_or_else(|| Error::message("missing CAD substrate"))?;
    let mesh = substrate.mesh();
    assert!(!mesh.edges.is_empty(), "native board has no display edges");
    assert!(
        (0..mesh.edges.len()).any(|i| mesh.shaded_edge_visible(i)),
        "board edges are hidden in shaded-with-edges style"
    );
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
fn concurrent_exports_use_private_temporary_directories() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let (dir, _stage) = fixture(false)?;
    let program = dir.path().join("fake-kicad");
    std::fs::rename(&program, dir.path().join("fake-kicad-exporter"))?;
    std::fs::write(
        dir.path().join("shared-temp"),
        std::env::temp_dir().to_string_lossy().as_bytes(),
    )?;
    std::fs::write(
        &program,
        r##"#!/bin/sh
directory="$(dirname "$0")"
if [ "$TMPDIR" = "$(cat "$directory/shared-temp")" ]; then
  printf "%s\n" "11:36:45: Warning: Invalid lock file '/tmp/user/1000/org.kicad.kicad/instances/kicad-cli-10.0'." >&2
fi
[ "$TMPDIR" = "$TMP" ] && [ "$TMPDIR" = "$TEMP" ] || exit 1
printf '%s\n' "$TMPDIR" >> "$directory/runtime-calls"
if [ "$1" != version ]; then
  printf 'Warning: Keep genuine exporter diagnostics\n' >&2
fi
exec "$directory/fake-kicad-exporter" "$@"
"##,
    )?;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))?;
    let settings = Settings::default();
    let stage = Exporter {
        program: &program,
        timeout: Duration::from_secs(5),
        settings: &settings,
    }
    .stage_build(inventory::read(&dir.path().join("board.txt"))?, false)?;
    assert_eq!(stage.kicad_version, "10.0.7");
    assert!(
        stage
            .warnings
            .iter()
            .all(|w| !w.contains("Invalid lock file"))
    );
    assert!(stage.warnings.iter().any(|w| w.contains("Keep genuine")));
    let calls = std::fs::read_to_string(dir.path().join("runtime-calls"))?;
    let paths: std::collections::BTreeSet<_> = calls.lines().collect();
    assert!(paths.len() > 1);
    assert_eq!(paths.len(), calls.lines().count());
    assert!(paths.iter().all(|path| !Path::new(path).exists()));
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
            assert_eq!(value["solidcraft_version"], cad::SOLIDCRAFT_VERSION);
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
    version: String,
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
                Ok(json!({"version":self.version}))
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
fn live_version_check_accepts_newer_releases_and_rejects_older_or_invalid_versions() -> Result<()> {
    let minimum = semver::Version::parse(cad::MIN_SOLIDCRAFT_VERSION)
        .map_err(|e| Error::message(e.to_string()))?;
    let next_minor = semver::Version::new(minimum.major, minimum.minor + 1, 0);
    let mut cases = vec![
        (minimum.to_string(), true),
        (format!("{minimum}+local.1"), true),
        (cad::SOLIDCRAFT_VERSION.into(), true),
        (
            semver::Version::new(minimum.major, minimum.minor, minimum.patch + 1).to_string(),
            true,
        ),
        (next_minor.to_string(), true),
        (format!("{next_minor}-beta.1"), true),
        (
            semver::Version::new(minimum.major, minimum.minor + 100, 0).to_string(),
            true,
        ),
        (
            semver::Version::new(minimum.major + 1, 0, 0).to_string(),
            true,
        ),
        (format!("{minimum}-rc.1"), false),
        (format!("{}.{}", minimum.major, minimum.minor), false),
        ("invalid".into(), false),
        (String::new(), false),
    ];
    if minimum.patch > 0 {
        cases.push((
            semver::Version::new(minimum.major, minimum.minor, minimum.patch - 1).to_string(),
            false,
        ));
    } else if minimum.minor > 0 {
        cases.push((
            semver::Version::new(minimum.major, minimum.minor - 1, 999).to_string(),
            false,
        ));
    } else if minimum.major > 0 {
        cases.push((
            semver::Version::new(minimum.major - 1, 999, 999).to_string(),
            false,
        ));
    }
    for (version, supported) in cases {
        let mut app = App {
            inner: Headless::default(),
            version: version.clone(),
            batches: 0,
            lose_reply: false,
        };
        let result = cad::destination(&mut app);
        assert_eq!(result.is_ok(), supported, "{version:?}: {result:?}");
        assert_eq!(app.batches, 0);
        match result {
            Ok(destination) => assert_eq!(destination.version, version),
            Err(error) => assert!(
                error
                    .to_string()
                    .contains(&format!("{minimum} or later is required"))
            ),
        }
    }
    Ok(())
}

#[test]
fn engine_metadata_and_documented_compatibility_match_the_linked_engine() -> Result<()> {
    let mut backend = Headless::default();
    let script = cad::execute(&mut backend, "timeline.export_script", json!({}))?;
    assert_eq!(
        script["script"]["about"]["version"],
        cad::SOLIDCRAFT_VERSION
    );
    let block = include_str!("../README.md")
        .split_once("<!-- solidcraft-compatibility:begin -->")
        .and_then(|(_, rest)| rest.split_once("<!-- solidcraft-compatibility:end -->"))
        .map(|(block, _)| block)
        .ok_or_else(|| Error::message("missing README compatibility block"))?;
    for expected in [
        format!("**{}**", cad::SOLIDCRAFT_VERSION),
        format!("format **{}**", cad::SOLIDCRAFT_FORMAT),
        format!("SolidCraft {} or later", cad::MIN_SOLIDCRAFT_VERSION),
    ] {
        assert!(
            block.contains(&expected),
            "README is out of date: {expected}"
        );
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn live_recipe_handles_name_collisions_without_changing_author_context() -> Result<()> {
    let (_dir, mut stage) = fixture(false)?;
    let engine = semver::Version::parse(cad::SOLIDCRAFT_VERSION)
        .map_err(|e| Error::message(e.to_string()))?;
    let mut app = App {
        inner: Headless::default(),
        version: semver::Version::new(engine.major, engine.minor + 1, 0).to_string(),
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
        version: cad::SOLIDCRAFT_VERSION.into(),
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
    assert_eq!(
        state
            .bodies
            .iter()
            .filter(|body| !body.body.is_mesh())
            .count(),
        1
    );
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
