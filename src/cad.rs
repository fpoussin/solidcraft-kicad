use std::{collections::BTreeSet, path::Path};

use semver::Version;
use serde_json::{Value, json};
use solidcraft_mcp::{Backend, Headless};

use crate::{
    container,
    error::{Error, Result},
    staging::Staged,
};

pub use crate::container::Compression;

pub const METADATA_KEY: &str = "org.solidcraft-kicad.imports";
pub const SOLIDCRAFT_VERSION: &str = env!("SOLIDCRAFT_ENGINE_VERSION");
pub const MIN_SOLIDCRAFT_VERSION: &str = env!("SOLIDCRAFT_MINIMUM_VERSION");
pub const SOLIDCRAFT_FORMAT: u32 = solidcraft_engine::doc::format::FORMAT;

pub fn execute(b: &mut dyn Backend, command: &str, params: Value) -> Result<Value> {
    b.call(
        "engine.execute",
        json!({"command": command, "params": params}),
    )
    .map_err(Error::message)
}

pub fn inspect(b: &mut dyn Backend) -> Result<Value> {
    b.call(
        "document.inspect",
        json!({"measure": false, "sketches": false}),
    )
    .map_err(Error::message)
}

pub fn validate(b: &mut dyn Backend) -> Result<Value> {
    check(b)?;
    execute(b, "inspect.measure", json!({}))
}

fn check(b: &mut dyn Backend) -> Result<Value> {
    let i = inspect(b)?;
    for feature in i["timeline"].as_array().into_iter().flatten() {
        for key in ["error", "warning"] {
            if let Some(message) = feature[key].as_str().filter(|s| !s.is_empty()) {
                return Err(Error::message(format!(
                    "SolidCraft {}: {message}",
                    feature["name"]
                )));
            }
        }
    }
    if i["bodies"].as_array().is_none_or(Vec::is_empty) {
        return Err(Error::message("export contains no importable solid bodies"));
    }
    Ok(json!({"body_count": i["bodies"].as_array().map_or(0, Vec::len)}))
}

pub fn import_asset(step: &Path, native: &Path) -> Result<Value> {
    let mut b = Headless::default();
    let result = execute(
        &mut b,
        "file.insert_step",
        json!({"path": step, "up": "z", "names": "product"}),
    )?;
    if result["warnings"].as_array().is_some_and(|w| !w.is_empty()) {
        return Err(Error::message(format!(
            "SolidCraft import warnings: {}",
            result["warnings"]
        )));
    }
    let measure = check(&mut b)?;
    execute(&mut b, "file.save_as", json!({"path": native}))?;
    Ok(measure)
}

pub fn recipe(
    stage: &Staged,
    existing: &Value,
    components: &Value,
) -> Result<(Vec<Value>, String)> {
    let mut names: BTreeSet<String> = components["components"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["name"].as_str().map(str::to_string))
        .collect();
    // Reserve both component names and generated instance labels up front.
    for component in components["components"].as_array().into_iter().flatten() {
        for occurrence in component["occurrences"].as_array().into_iter().flatten() {
            if let Some(name) = occurrence["name"]
                .as_str()
                .and_then(|n| n.strip_suffix(":1"))
            {
                names.insert(name.to_string());
            }
        }
    }
    let name = unique_name(&stage.inventory.name, &mut names);
    let mut steps = vec![
        json!({"command": "component.create", "params": {"name": name, "parent": 0, "activate": false}}),
    ];
    let mut children = Vec::new();
    for asset in &stage.assets {
        let child = unique_name(&asset.reference, &mut names);
        steps.push(json!({"command": "component.insert_design", "params": {"path": asset.path, "name": child, "parent": name}}));
        steps.push(json!({"command": "component.break_link", "params": {"component": child}}));
        children.push(child);
    }
    if children.len() > 1 {
        steps.push(json!({"command": "joint.rigid_group", "params": {"occurrences": children.iter().map(|c| format!("{c}:1")).collect::<Vec<_>>()}}));
    }
    let mut imports = match existing.get(METADATA_KEY) {
        None => serde_json::Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => {
            return Err(Error::message(
                "existing solidcraft-kicad provenance is not an object",
            ));
        }
    };
    imports.insert(
        stage.id.clone(),
        json!({
            "source": stage.inventory.source, "sha256": stage.inventory.sha256,
            "component_name": name, "kicad_version": stage.kicad_version,
            "references": stage.assets.iter().map(|a| a.reference.as_str()).collect::<Vec<_>>(),
            "component_names": children,
            "omissions": stage.omissions,
        }),
    );
    if serde_json::to_vec(&imports)?.len() > 60 * 1024 {
        return Err(Error::message(
            "import provenance exceeds SolidCraft's metadata limit",
        ));
    }
    steps.push(json!({"command": "document.metadata.set", "params": {"key": METADATA_KEY, "value": imports}}));
    if steps.len() > 10_000 || serde_json::to_vec(&steps)?.len() > 15 * 1024 * 1024 {
        return Err(Error::message("assembly exceeds SolidCraft's batch limits"));
    }
    Ok((steps, name))
}

pub(crate) fn unique_name(preferred: &str, names: &mut BTreeSet<String>) -> String {
    let mut name = preferred.to_string();
    let mut suffix = 2;
    while !names.insert(name.clone()) {
        name = format!("{preferred} ({suffix})");
        suffix += 1;
    }
    name
}

pub fn offline_batch(b: &mut Headless, steps: &[Value]) -> Result<Value> {
    let reply =
        solidcraft_mcp::call_tool(b, "batch", &json!({"commands": steps, "rollback": true}));
    let text = reply["content"][0]["text"]
        .as_str()
        .ok_or_else(|| Error::message("SolidCraft returned no batch result"))?;
    if reply["isError"] == true {
        return Err(Error::message(text));
    }
    Ok(serde_json::from_str(text)?)
}

pub fn build(stage: &Staged, output: &Path) -> Result<Value> {
    build_with_measurements(stage, output, true)
}

pub fn build_with_measurements(stage: &Staged, output: &Path, measure: bool) -> Result<Value> {
    build_with_options(stage, output, measure, false)
}

pub fn build_with_options(
    stage: &Staged,
    output: &Path,
    measure: bool,
    overwrite: bool,
) -> Result<Value> {
    build_with_compression(stage, output, measure, overwrite, Compression::default())
}

pub fn build_with_compression(
    stage: &Staged,
    output: &Path,
    measure: bool,
    overwrite: bool,
    compression: Compression,
) -> Result<Value> {
    build_with_progress(stage, output, measure, overwrite, compression, &|_| {})
}

/// Reports the geometry-storage stage after serialization and then the final write stage.
/// The caller advances once more after this function returns to account for writing.
pub fn build_with_progress(
    stage: &Staged,
    output: &Path,
    measure: bool,
    overwrite: bool,
    compression: Compression,
    next_stage: &dyn Fn(&'static str),
) -> Result<Value> {
    if !overwrite && output.exists() {
        return Err(Error::message(format!(
            "output already exists: {}",
            output.display()
        )));
    }
    let (mut b, name) = if let Some(prepared) = &stage.assembly {
        let mut doc = prepared.doc.clone();
        let children: Vec<_> = prepared
            .doc
            .components
            .iter()
            .filter(|c| c.parent == prepared.parent)
            .map(|c| c.name.clone())
            .collect();
        let imports = json!({stage.id.clone(): {
            "source": stage.inventory.source, "sha256": stage.inventory.sha256,
            "component_name": stage.inventory.name, "kicad_version": stage.kicad_version,
            "references": stage.assets.iter().map(|a| a.reference.as_str()).collect::<Vec<_>>(),
            "component_names": children, "omissions": stage.omissions,
        }});
        if serde_json::to_vec(&imports)?.len() > 60 * 1024 {
            return Err(Error::message(
                "import provenance exceeds SolidCraft's metadata limit",
            ));
        }
        doc.metadata = serde_json::from_value(json!({ METADATA_KEY: imports }))?;
        let measurement = if measure {
            let mut b = Headless::new(solidcraft_engine::Session::new(doc.clone()));
            validate(&mut b)?
        } else {
            Value::Null
        };
        let bytes = solidcraft_engine::io::write_design_container_with_compression(
            &doc,
            solidcraft_engine::io::DesignCompression::Fast,
        )
        .map_err(|e| Error::message(e.to_string()))?;
        let bytes = repack(bytes, compression, next_stage)?;
        let parent = output
            .parent()
            .ok_or_else(|| Error::message("output has no parent folder"))?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        std::io::Write::write_all(&mut file, &bytes)?;
        if overwrite {
            file.persist(output).map_err(|e| e.error)?;
        } else {
            file.persist_noclobber(output).map_err(|e| e.error)?;
        }
        return Ok(
            json!({"component_name": stage.inventory.name, "output": output, "measurement": measurement}),
        );
    } else {
        let mut b = Headless::default();
        let components = execute(&mut b, "component.list", json!({"compact": true}))?;
        let (steps, name) = recipe(stage, &json!({}), &components)?;
        offline_batch(&mut b, &steps)?;
        (b, name)
    };
    let measure = if measure {
        validate(&mut b)?
    } else {
        check(&mut b)?
    };
    std::fs::create_dir_all(
        output
            .parent()
            .ok_or_else(|| Error::message("output has no parent folder"))?,
    )?;
    // Publish only after a complete save, replacing atomically when authorized.
    let directory = tempfile::tempdir_in(
        output
            .parent()
            .ok_or_else(|| Error::message("output has no parent"))?,
    )?;
    let native = directory.path().join(
        output
            .file_name()
            .ok_or_else(|| Error::message("output has no filename"))?,
    );
    execute(
        &mut b,
        "file.save_as",
        json!({"path": native, "format": "container", "compression": "fast"}),
    )?;
    let bytes = repack(std::fs::read(&native)?, compression, next_stage)?;
    std::fs::write(&native, bytes)?;
    if overwrite {
        std::fs::rename(&native, output)?;
    } else {
        std::fs::hard_link(&native, output)?;
    }
    Ok(json!({"component_name": name, "output": output, "measurement": measure}))
}

fn repack(
    bytes: Vec<u8>,
    compression: Compression,
    next_stage: &dyn Fn(&'static str),
) -> Result<Vec<u8>> {
    match compression {
        Compression::Compact => next_stage("Compressing SolidCraft geometry"),
        Compression::Fast => next_stage("Storing SolidCraft geometry"),
    };
    let bytes = container::repack(&bytes, compression)?;
    next_stage("Writing SolidCraft file");
    Ok(bytes)
}

#[derive(Debug, PartialEq)]
pub struct Destination {
    identity: Value,
    pub version: String,
    pub inspect: Value,
    pub components: Value,
}

pub fn destination(b: &mut dyn Backend) -> Result<Destination> {
    let about = b.call("ui.help", json!({})).map_err(Error::message)?;
    let version = about["version"].as_str().unwrap_or_default();
    let minimum = Version::parse(MIN_SOLIDCRAFT_VERSION)
        .map_err(|e| Error::message(format!("invalid minimum desktop version: {e}")))?;
    if !Version::parse(version).is_ok_and(|v| v >= minimum) {
        return Err(Error::message(format!(
            "SolidCraft {MIN_SOLIDCRAFT_VERSION} or later is required; app reported {version:?}"
        )));
    }
    let ui = b.call("ui.inspect", json!({})).map_err(Error::message)?;
    if ui["tool"].is_string()
        || ui["dialog"].is_string()
        || ui["file_busy"] == true
        || ui["file_picker_busy"] == true
    {
        return Err(Error::message(
            "finish the current SolidCraft tool/dialog or file operation before inserting",
        ));
    }
    if ui["ui"]["agent_cursor"]["show"] == true {
        return Err(Error::message(
            "SolidCraft's agent cursor is enabled; disable it before using atomic insertion",
        ));
    }
    let docs = b.call("ui.documents", json!({})).map_err(Error::message)?;
    let i = inspect(b)?;
    if !i["active_sketch"].is_null()
        || i["timeline"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|f| f["rolled_back"] == true)
    {
        return Err(Error::message(
            "finish the active sketch and move SolidCraft's timeline to its end before inserting",
        ));
    }
    let components = execute(b, "component.list", json!({"compact": true}))?;
    let identity = json!({"name": i["name"], "path": i["path"], "revision": i["revision"], "documents": docs, "components": components});
    Ok(Destination {
        identity,
        version: version.to_string(),
        inspect: i,
        components,
    })
}

pub fn insert(b: &mut dyn Backend, before: &Destination, stage: &mut Staged) -> Result<Value> {
    let after = destination(b)?;
    if before.identity != after.identity {
        return Err(Error::message(
            "the destination changed during staging; insertion was not sent",
        ));
    }
    let (steps, name) = recipe(stage, &after.inspect["metadata"], &after.components)?;
    let reply = match b.call("engine.batch", json!({"commands": steps, "rollback": true})) {
        Ok(value) => value,
        Err(error) => {
            let kept = stage.keep();
            return Err(Error::message(format!(
                "insertion did not return a definitive result: {error}. Do not repeat it automatically; inspect SolidCraft first. Staging files retained at {}",
                kept.display()
            )));
        }
    };
    if !reply["failed"].is_null() {
        return Err(Error::message(format!(
            "SolidCraft rejected the insertion (completed {}, undone {}): {}",
            reply["completed"], reply["undone"], reply["failed"]
        )));
    }
    if reply["completed"].as_u64() != Some(steps.len() as u64) {
        let kept = stage.keep();
        return Err(Error::message(format!(
            "unexpected batch reply; inspect the app before retrying. Staging retained at {}: {reply}",
            kept.display()
        )));
    }
    let mut warnings = Vec::new();
    for result in reply["results"].as_array().into_iter().flatten() {
        for warning in result["warnings"].as_array().into_iter().flatten() {
            warnings.push(warning.clone());
        }
    }
    Ok(
        json!({"component_name": name, "destination": after.inspect["name"], "component": reply["results"][0]["component"], "occurrence": reply["results"][0]["occurrence"], "warnings": warnings, "completed": reply["completed"]}),
    )
}
