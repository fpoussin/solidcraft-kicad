//! Share coloured model definitions while retaining named footprint placements.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use sha2::{Digest, Sha256};
use solidcraft_engine::{
    doc::{Document, FeatureKind, IDENTITY, Mat, MeshData, apply_point, mat_inverse, mat_mul},
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
    meshes: Vec<MeshData>,
    origin: Mat,
}

pub fn prepare(
    path: &Path,
    inventory: &Inventory,
    allow_missing: bool,
) -> Result<(Prepared, Vec<Part>, Vec<String>)> {
    let bytes = std::fs::read(path)?;
    let imported = solidcraft_engine::io::read_gltf(&bytes, "assembly.glb", &mut |_| {
        Err("external glTF buffers are not allowed in a KiCad GLB".into())
    })
    .map_err(|e| Error::message(e.to_string()))?;
    if !imported.feature.warnings.is_empty() {
        return Err(Error::message(imported.feature.warnings.join("; ")));
    }
    let expected: BTreeSet<&str> = inventory
        .footprints
        .iter()
        .filter(|f| !f.dnp && !f.models.is_empty())
        .map(|f| f.reference.as_str())
        .collect();
    let groups = footprint_meshes(&imported, &expected)?;
    let mut doc = Document::new(&inventory.name);
    let parent = doc
        .add_component(Some(&inventory.name), 0)
        .map_err(|e| Error::message(e.to_string()))?;
    let mut names = BTreeSet::from([inventory.name.clone()]);
    let mut parts = Vec::new();
    let mut definitions = BTreeMap::new();
    let mut omissions = Vec::new();
    let mut groups = groups;
    for reference in std::iter::once("PCB").chain(expected.iter().copied()) {
        let model = groups.remove(reference);
        if model.as_ref().is_none_or(|m| m.meshes.is_empty()) {
            let message =
                format!("{reference}: omitted; KiCad exported no geometry for the assigned models");
            if reference == "PCB" || !allow_missing {
                return Err(Error::message(message));
            }
            omissions.push(message);
            continue;
        }
        let model = model.ok_or_else(|| Error::message("missing model geometry"))?;
        let meshes = model.meshes;
        let name = unique_name(reference, &mut names);
        let placement = mat_mul(&TO_Z_UP, &mat_mul(&model.origin, &TO_Y_UP));
        let key = model_key(&meshes)?;
        let count = meshes.len();
        if let Some(&component) = definitions.get(&key) {
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
        definitions.insert(key, component);
        let meshes: Vec<_> = meshes
            .into_iter()
            .enumerate()
            .map(|(i, mut mesh)| {
                mesh.name = format!("{name}_{}", i + 1);
                mesh
            })
            .collect();
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

fn model_key(meshes: &[MeshData]) -> Result<[u8; 32]> {
    let mut pieces = Vec::with_capacity(meshes.len());
    for mesh in meshes {
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
                destination.meshes.push(mesh);
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
