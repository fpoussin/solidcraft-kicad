//! Share coloured model definitions while retaining named footprint placements.
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

use sha2::{Digest, Sha256};
use solidcraft_engine::{
    doc::{
        Document, FeatureKind, IDENTITY, Mat, MeshData, appearance::Look, apply_point, mat_inverse,
        mat_mul, pbr_appearance::PbrParameters,
    },
    geom::Vec3,
    io::GltfImport,
};

use crate::{
    cad::unique_name,
    error::{Error, Result},
    inventory::Inventory,
};

pub struct Prepared {
    pub doc: Document,
    pub parent: u64,
}

pub struct Part {
    pub reference: String,
    pub bodies: usize,
}

const TO_Z_UP: Mat = [
    [1., 0., 0., 0.],
    [0., 0., 1., 0.],
    [0., -1., 0., 0.],
    [0., 0., 0., 1.],
];
const TO_Y_UP: Mat = [
    [1., 0., 0., 0.],
    [0., 0., -1., 0.],
    [0., 1., 0., 0.],
    [0., 0., 0., 1.],
];

struct PlacedModel {
    meshes: Vec<ModelMesh>,
    origin: Mat,
}

struct ModelMesh {
    mesh: MeshData,
    look: Option<Look>,
}

pub fn prepare(
    path: Option<&Path>,
    board: &Path,
    substrate_path: &Path,
    inventory: &Inventory,
    allow_missing: bool,
) -> Result<(Prepared, Vec<Part>, Vec<String>)> {
    let expected: BTreeSet<&str> = inventory
        .footprints
        .iter()
        .filter(|f| !f.dnp && !f.models.is_empty())
        .map(|f| f.reference.as_str())
        .collect();
    let mut groups = match path {
        Some(path) => read_models(path, &expected, allow_missing, false)?,
        None => BTreeMap::new(),
    };
    let mut board_groups = read_models(board, &BTreeSet::new(), false, true)?;
    let mut substrate = board_groups
        .remove("PCB")
        .ok_or_else(|| Error::message("KiCad exported no board geometry"))?;
    let low = substrate
        .meshes
        .iter()
        .flat_map(|m| m.mesh.positions.iter().skip(2).step_by(3))
        .copied()
        .fold(f64::INFINITY, f64::min);
    let high = substrate
        .meshes
        .iter()
        .flat_map(|m| m.mesh.positions.iter().skip(2).step_by(3))
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    for item in &mut substrate.meshes {
        // KiCad marks its silk material as a rough dielectric. Keep a small visual
        // separation: SolidCraft 0.14's viewport otherwise hides ink behind the mask.
        if item
            .look
            .as_ref()
            .and_then(|l| l.pbr)
            .is_some_and(|p| p.metallic() == 0.0 && (p.roughness() - 0.9).abs() < 1e-5)
            && let Some(&z) = item.mesh.positions.get(2)
            && item
                .mesh
                .positions
                .iter()
                .skip(2)
                .step_by(3)
                .all(|v| (v - z).abs() < 1e-6)
        {
            let offset = if z > (low + high) * 0.5 { 0.1 } else { -0.1 };
            for z in item.mesh.positions.iter_mut().skip(2).step_by(3) {
                *z += offset;
            }
        }
        compact_vertices(&mut item.mesh)?;
    }
    if !board_groups.is_empty() || groups.contains_key("PCB") {
        return Err(Error::message(
            "unexpected geometry in split board/model export",
        ));
    }
    if let Some(finish) = inventory.copper_finish.as_deref()
        && let Some(color) = finish_color(finish)
    {
        for ModelMesh { mesh, look } in &mut substrate.meshes {
            // The board-only GLB contains no component metals: its metallic material
            // represents pads and plated holes. KiCad's exporter ignores copper_finish.
            if let Some(look) = look
                && look.pbr.is_some_and(|p| p.metallic() == 1.0)
            {
                look.name = finish.into();
                look.color = color;
                mesh.color = Some(look.rgb());
            }
        }
    }
    groups.insert("PCB".into(), substrate);
    let native_substrate = solidcraft_engine::io::step_import_feature(
        &std::fs::read(substrate_path)?,
        "substrate.step",
    )
    .map_err(|e| Error::message(e.to_string()))?;
    if !native_substrate.warnings.is_empty() || native_substrate.body_names.is_empty() {
        return Err(Error::message(format!(
            "invalid native substrate: {}",
            native_substrate.warnings.join("; ")
        )));
    }
    let mut doc = Document::new(&inventory.name);
    let parent = doc
        .add_component(Some(&inventory.name), 0)
        .map_err(|e| Error::message(e.to_string()))?;
    let mut names = BTreeSet::from([inventory.name.clone()]);
    let mut parts = Vec::new();
    let mut definitions = BTreeMap::new();
    let mut omissions = Vec::new();
    for reference in std::iter::once("PCB").chain(expected.iter().copied()) {
        let model = groups.remove(reference);
        if model
            .as_ref()
            .is_none_or(|m| m.meshes.is_empty() && reference != "PCB")
        {
            let message =
                format!("{reference}: omitted; KiCad exported no geometry for the assigned models");
            if reference == "PCB" || !allow_missing {
                return Err(Error::message(message));
            }
            omissions.push(message);
            continue;
        }
        let model = model.ok_or_else(|| Error::message("missing model geometry"))?;
        let mut meshes = model.meshes;
        let mut substrate_look = None;
        if reference == "PCB" {
            meshes.retain(|item| {
                let body = item
                    .look
                    .as_ref()
                    .and_then(|l| l.pbr)
                    .is_some_and(|p| p.metallic() == 0.0 && (p.roughness() - 0.8).abs() < 1e-5);
                if body {
                    substrate_look = item.look.clone();
                }
                !body
            });
            if substrate_look.is_none() {
                return Err(Error::message(
                    "KiCad board export has no substrate material",
                ));
            }
        }
        let name = unique_name(reference, &mut names);
        let placement = mat_mul(&TO_Z_UP, &mat_mul(&model.origin, &TO_Y_UP));
        let key = if reference == "PCB" {
            None
        } else {
            Some(model_key(&meshes)?)
        };
        let count = meshes.len()
            + if reference == "PCB" {
                native_substrate.body_names.len()
            } else {
                0
            };
        if let Some(&component) = key.as_ref().and_then(|key| definitions.get(key)) {
            let occurrence = doc
                .add_occurrence(component, parent, placement)
                .map_err(|e| Error::message(e.to_string()))?;
            if let Some(o) = doc.occurrences.iter_mut().find(|o| o.id == occurrence) {
                o.name = format!("{name}:1");
            }
            parts.push(Part {
                reference: reference.into(),
                bodies: count,
            });
            continue;
        }
        let component = doc
            .add_component(Some(&name), parent)
            .map_err(|e| Error::message(e.to_string()))?;
        if let Some(o) = doc
            .occurrences
            .iter_mut()
            .find(|o| o.component == component)
        {
            o.transform = placement;
        }
        if let Some(key) = key {
            definitions.insert(key, component);
        }
        if reference == "PCB" {
            if let Some(look) = substrate_look {
                doc.appearances.components.insert(component, look);
            }
            let label = unique_name("PCB substrate", &mut names);
            let feature = doc
                .add_feature(native_substrate.kind.clone(), Some(&label))
                .map_err(|e| Error::message(e.to_string()))?;
            if let Some(feature) = doc.feature_mut(feature) {
                feature.component = component;
                feature.body_names = (1..=native_substrate.body_names.len())
                    .map(|i| format!("{label}_{i}"))
                    .collect();
            }
        }
        let meshes: Vec<_> = meshes
            .into_iter()
            .enumerate()
            .map(|(i, ModelMesh { mut mesh, look })| {
                mesh.name = format!("{name}_{}", i + 1);
                if let Some(look) = look {
                    doc.appearances.bodies.insert(mesh.name.clone(), look);
                }
                mesh
            })
            .collect();
        if !meshes.is_empty() {
            let feature = doc
                .add_feature(
                    FeatureKind::MeshImport {
                        file: format!("{reference}.glb"),
                        meshes,
                    },
                    Some(&name),
                )
                .map_err(|e| Error::message(e.to_string()))?;
            if let Some(f) = doc.feature_mut(feature) {
                f.component = component;
            }
        }
        parts.push(Part {
            reference: reference.into(),
            bodies: count,
        });
    }
    if !groups.is_empty() {
        return Err(Error::message(
            "GLB contains geometry without a footprint assignment",
        ));
    }
    Ok((Prepared { doc, parent }, parts, omissions))
}

fn read_models(
    path: &Path,
    expected: &BTreeSet<&str>,
    allow_empty: bool,
    board: bool,
) -> Result<BTreeMap<String, PlacedModel>> {
    let bytes = std::fs::read(path)?;
    let imported = solidcraft_engine::io::read_gltf(&bytes, "assembly.glb", &mut |_| {
        Err("external glTF buffers are not allowed in a KiCad GLB".into())
    });
    let mut imported = match imported {
        Ok(imported) => imported,
        // KiCad writes an empty GLB when all assigned models are unavailable.
        Err(error)
            if allow_empty
                && ["glTF has no nodes to import", "glTF has no triangle meshes"]
                    .iter()
                    .any(|message| error.to_string().ends_with(message)) =>
        {
            return Ok(BTreeMap::new());
        }
        Err(error) => return Err(Error::message(error.to_string())),
    };
    if !imported.feature.warnings.is_empty() {
        return Err(Error::message(imported.feature.warnings.join("; ")));
    }
    if board {
        opaque_coatings(&mut imported, &bytes)?;
    }
    footprint_meshes(&imported, expected)
}

fn opaque_coatings(imported: &mut GltfImport, bytes: &[u8]) -> Result<()> {
    // The engine discards PBR parameters on blended materials. Recover the board
    // coating response when making those layers opaque, preserving their RGB.
    let json = if bytes.starts_with(b"glTF") {
        let length = bytes
            .get(12..16)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| Error::message("missing GLB JSON chunk"))? as usize;
        bytes
            .get(20..20 + length)
            .ok_or_else(|| Error::message("truncated GLB JSON chunk"))?
    } else {
        bytes
    };
    let document: serde_json::Value = serde_json::from_slice(json)?;
    let materials = document["materials"].as_array();
    for look in imported.looks.iter_mut().flatten() {
        look.opacity = 1.0;
        if look.pbr.is_none()
            && let Some(material) = materials.and_then(|ms| {
                ms.iter()
                    .find(|m| m["name"].as_str() == Some(look.name.as_str()))
            })
        {
            let pbr = &material["pbrMetallicRoughness"];
            look.pbr = PbrParameters::new(
                pbr["metallicFactor"].as_f64().unwrap_or(1.0),
                pbr["roughnessFactor"].as_f64().unwrap_or(1.0),
                1.5,
            )
            .ok();
        }
    }
    Ok(())
}

// KiCad repeats vertices at face boundaries. Share exact coordinates without
// changing any triangles, normals, dimensions, or material boundaries.
fn compact_vertices(mesh: &mut MeshData) -> Result<()> {
    let mut indices = HashMap::new();
    let mut positions = Vec::new();
    let mut remap = Vec::with_capacity(mesh.positions.len() / 3);
    for point in mesh.positions.as_chunks::<3>().0 {
        let key = point.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
        let next = u32::try_from(positions.len() / 3)
            .map_err(|_| Error::message("board mesh has too many vertices"))?;
        let index = *indices.entry(key).or_insert_with(|| {
            positions.extend_from_slice(point);
            next
        });
        remap.push(index);
    }
    for index in &mut mesh.triangles {
        *index = *remap
            .get(*index as usize)
            .ok_or_else(|| Error::message("invalid board mesh index"))?;
    }
    mesh.positions = positions;
    Ok(())
}

// KiCad 10's 3D viewer finish palette (BOARD_ADAPTER::GetLayerColors).
fn finish_color(finish: &str) -> Option<[u8; 3]> {
    let finish = finish.trim().to_ascii_lowercase();
    if finish.ends_with("osp") {
        Some([184, 115, 50])
    } else if finish.ends_with("ig") || finish.ends_with("gold") {
        Some([178, 156, 0])
    } else if finish.starts_with("hal")
        || finish.starts_with("hasl")
        || finish.ends_with("tin")
        || finish.ends_with("nickel")
    {
        Some([160, 160, 160])
    } else if finish.ends_with("silver") {
        Some([213, 213, 213])
    } else {
        None
    }
}

fn model_key(meshes: &[ModelMesh]) -> Result<[u8; 32]> {
    let mut pieces = Vec::with_capacity(meshes.len());
    for ModelMesh { mesh, look } in meshes {
        let mut hash = Sha256::new();
        hash.update((mesh.positions.len() as u64).to_le_bytes());
        for &coordinate in &mesh.positions {
            if !coordinate.is_finite() || coordinate.abs() > 1e9 {
                return Err(Error::message("invalid model coordinate"));
            }
            // Ignore roundoff from removing the placement (one nanometre in mm units).
            hash.update(((coordinate * 1e6).round() as i64).to_le_bytes());
        }
        hash.update((mesh.triangles.len() as u64).to_le_bytes());
        for &index in &mesh.triangles {
            hash.update(index.to_le_bytes());
        }
        hash.update([u8::from(mesh.color.is_some())]);
        if let Some(color) = mesh.color {
            for channel in color {
                hash.update(channel.to_bits().to_le_bytes());
            }
        }
        // Material labels vary between export nodes; physical appearance must match.
        hash.update(serde_json::to_vec(
            &look
                .as_ref()
                .map(|look| (look.color, look.opacity, look.pbr)),
        )?);
        pieces.push(<[u8; 32]>::from(hash.finalize()));
    }
    pieces.sort_unstable();
    let mut hash = Sha256::new();
    hash.update((pieces.len() as u64).to_le_bytes());
    for piece in pieces {
        hash.update(piece);
    }
    Ok(hash.finalize().into())
}

fn footprint_meshes(
    imported: &GltfImport,
    expected: &BTreeSet<&str>,
) -> Result<BTreeMap<String, PlacedModel>> {
    let FeatureKind::MeshImport { meshes, .. } = &imported.feature.kind else {
        return Err(Error::message("glTF did not produce mesh geometry"));
    };
    let mut children: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
    for (i, place) in imported.plan.places.iter().enumerate() {
        children.entry(place.parent).or_default().push(i);
    }
    let mut bodies: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
    for (i, slot) in imported.plan.bodies.iter().enumerate() {
        bodies.entry(*slot).or_default().push(i);
    }
    let mut walk = Walker {
        imported,
        meshes,
        children,
        bodies,
        expected,
        groups: BTreeMap::new(),
        visits: 0,
    };
    walk.visit(None, &IDENTITY, None, None, &mut Vec::new())?;
    Ok(walk.groups)
}

struct Walker<'a> {
    imported: &'a GltfImport,
    meshes: &'a [MeshData],
    children: BTreeMap<Option<usize>, Vec<usize>>,
    bodies: BTreeMap<Option<usize>, Vec<usize>>,
    expected: &'a BTreeSet<&'a str>,
    groups: BTreeMap<String, PlacedModel>,
    visits: usize,
}

impl Walker<'_> {
    fn visit(
        &mut self,
        slot: Option<usize>,
        world: &Mat,
        reference: Option<&str>,
        origin: Option<Mat>,
        stack: &mut Vec<usize>,
    ) -> Result<()> {
        self.visits += 1;
        if self.visits > 100_000 || stack.len() > 64 {
            return Err(Error::message("GLB assembly is too large or recursive"));
        }
        if let Some(indices) = self.bodies.get(&slot) {
            let reference = reference.unwrap_or("PCB");
            let destination = self
                .groups
                .entry(reference.into())
                .or_insert_with(|| PlacedModel {
                    meshes: Vec::new(),
                    origin: origin.unwrap_or(*world),
                });
            let inverse = mat_inverse(&destination.origin)
                .ok_or_else(|| Error::message("invalid model placement"))?;
            let relative = mat_mul(&inverse, world);
            for &i in indices {
                let mut mesh = self
                    .meshes
                    .get(i)
                    .ok_or_else(|| Error::message("invalid GLB mesh index"))?
                    .clone();
                for p in mesh.positions.as_chunks_mut::<3>().0 {
                    let q = apply_point(&relative, Vec3::new(p[0], p[1], p[2]));
                    // The reader scales metres to mm; rotate glTF Y-up to KiCad Z-up.
                    *p = [q.x, -q.z, q.y];
                }
                destination.meshes.push(ModelMesh {
                    mesh,
                    look: self.imported.looks.get(i).cloned().flatten(),
                });
            }
        }
        for i in self.children.get(&slot).cloned().unwrap_or_default() {
            let place = &self.imported.plan.places[i];
            if stack.contains(&place.slot) {
                return Err(Error::message("recursive GLB component placement"));
            }
            let name = self
                .imported
                .plan
                .slots
                .get(place.slot)
                .ok_or_else(|| Error::message("invalid GLB component slot"))?;
            let label = if let Some(reference) = reference {
                Some(reference)
            } else if self.expected.contains(name.as_str()) {
                Some(name.as_str())
            } else if name.starts_with("=>[") || name == "PCB" || name == "PCB:1" {
                Some("PCB")
            } else if let Some(reference) = name
                .strip_suffix(":1")
                .filter(|r| self.expected.contains(r))
            {
                Some(reference)
            } else {
                return Err(Error::message(format!("unexpected GLB component: {name}")));
            };
            let placed = mat_mul(world, &place.transform);
            let origin = if reference.is_some() {
                origin
            } else {
                Some(placed)
            };
            stack.push(place.slot);
            self.visit(Some(place.slot), &placed, label, origin, stack)?;
            stack.pop();
        }
        Ok(())
    }
}
