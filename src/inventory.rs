use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
};

use kiutils_kicad::PcbFile;
use kiutils_sexpr::{Atom, Node};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

const MAX_BOARD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FOOTPRINTS: usize = 1000;

#[derive(Debug, Clone, Serialize)]
pub struct Footprint {
    pub reference: String,
    pub value: Option<String>,
    pub uuid: Option<String>,
    pub models: Vec<String>,
    pub dnp: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    pub source: PathBuf,
    pub sha256: String,
    pub name: String,
    pub thickness_mm: Option<f64>,
    pub footprints: Vec<Footprint>,
    pub warnings: Vec<String>,
}

pub fn hash(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_BOARD_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BOARD_BYTES {
        return Err(Error::message("PCB exceeds the 64 MiB input limit"));
    }
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

pub fn read(path: &Path) -> Result<Inventory> {
    let source = path.canonicalize()?;
    let sha256 = hash(&source)?;
    let document =
        PcbFile::read(&source).map_err(|e| Error::message(format!("{}: {e}", source.display())))?;
    let ast = document.ast();
    if ast.footprints.len() > MAX_FOOTPRINTS {
        return Err(Error::message(
            "first-version limit: 1000 footprints per PCB",
        ));
    }
    let mut references = BTreeSet::new();
    let mut footprints = Vec::new();
    let mut warnings = Vec::new();
    // Typed data is authoritative; CST supplies only optional fields newer than its schema.
    let nodes = document
        .cst()
        .nodes
        .first()
        .map(children)
        .unwrap_or_default();
    let raw_footprints: Vec<_> = nodes
        .iter()
        .filter(|n| head(n) == Some("footprint"))
        .collect();
    for (index, fp) in ast.footprints.iter().enumerate() {
        let reference = fp
            .reference
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Error::message(
                    "a footprint has no Reference property; KiCad 10 boards are required",
                )
            })?;
        if reference.len() > 96
            || reference.contains([',', '*', '?', '[', ']'])
            || reference.chars().any(char::is_control)
        {
            return Err(Error::message(format!(
                "reference {reference:?} cannot be used as an exact KiCad component filter"
            )));
        }
        if !references.insert(reference.to_string()) {
            return Err(Error::message(format!(
                "duplicate footprint reference {reference}"
            )));
        }
        let dnp = fp.attr.iter().any(|s| s == "dnp" || s == "do_not_populate")
            || raw_footprints.get(index).is_some_and(|n| {
                children(n).iter().any(|c| {
                    head(c) == Some("dnp") && children(c).get(1).and_then(atom) == Some("yes")
                })
            });
        let mut models = Vec::new();
        for model in fp.models.iter().filter(|m| !m.hide) {
            let path = model
                .path
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Error::message(format!("{reference} has an empty model path")))?;
            models.push(path.to_string());
        }
        if !dnp && models.is_empty() {
            warnings.push(format!("{reference}: no visible 3D model is assigned"));
        }
        footprints.push(Footprint {
            reference: reference.into(),
            value: fp.value.clone(),
            uuid: fp.uuid.clone(),
            models,
            dnp,
        });
    }
    footprints.sort_by(|a, b| a.reference.cmp(&b.reference));
    let name = source
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && s.len() <= 96)
        .ok_or_else(|| {
            Error::message("PCB filename must have a nonempty UTF-8 stem of at most 96 bytes")
        })?
        .to_string();
    let thickness_mm = ast.general.as_ref().and_then(|g| g.thickness);
    Ok(Inventory {
        source,
        sha256,
        name,
        thickness_mm,
        footprints,
        warnings,
    })
}

fn children(node: &Node) -> &[Node] {
    match node {
        Node::List { items, .. } => items,
        _ => &[],
    }
}

fn atom(node: &Node) -> Option<&str> {
    match node {
        Node::Atom {
            atom: Atom::Symbol(s) | Atom::Quoted(s),
            ..
        } => Some(s),
        _ => None,
    }
}

fn head(node: &Node) -> Option<&str> {
    children(node).first().and_then(atom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_handles_unknown_fields_dnp_hidden_models_and_unicode() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("metadata.txt");
        std::fs::write(
            &input,
            r#"(kicad_pcb (version 20260101) (generator test)
          (general (thickness 1.0)) (future_extension (nested "preserve me"))
          (footprint "GPS" (property "Reference" "U1") (property "Value" "GPS μ")
            (uuid "test-id") (model "${CUSTOM}/GPS.step") (future_setting yes))
          (footprint "R" (property "Reference" "R1") (attr smd dnp) (model "r.step"))
          (footprint "R" (property "Reference" "R2") (dnp yes) (model "r.step"))
          (footprint "X" (property "Reference" "X1") (model "hidden.step" hide)))"#,
        )?;
        let i = read(&input)?;
        assert_eq!(i.footprints.len(), 4);
        assert!(
            i.footprints
                .iter()
                .filter(|f| f.reference.starts_with('R'))
                .all(|f| f.dnp)
        );
        let u = i
            .footprints
            .iter()
            .find(|f| f.reference == "U1")
            .ok_or_else(|| Error::message("missing U1"))?;
        assert_eq!(u.models, ["${CUSTOM}/GPS.step"]);
        assert_eq!(u.value.as_deref(), Some("GPS μ"));
        assert_eq!(i.warnings.len(), 1);
        let doc = PcbFile::read(&input).map_err(|e| Error::message(e.to_string()))?;
        assert!(!doc.ast().unknown_nodes.is_empty());
        assert_eq!(
            doc.cst().to_lossless_string(),
            std::fs::read_to_string(&input)?
        );
        Ok(())
    }
}
