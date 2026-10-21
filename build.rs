use std::{env, error::Error, fs};

use semver::Version;
use toml::Value;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rustc-env=BUILD_TARGET={}", env::var("TARGET")?);
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    let root = std::path::PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let manifest: Value = toml::from_str(&fs::read_to_string(root.join("Cargo.toml"))?)?;
    let lock: Value = toml::from_str(&fs::read_to_string(root.join("Cargo.lock"))?)?;
    let engine = &manifest["dependencies"]["solidcraft-engine"];
    let git = engine["git"].as_str().ok_or("engine needs a Git source")?;
    let tag = engine["tag"].as_str().ok_or("engine needs a release tag")?;
    let source = format!("git+{git}?tag={tag}#");
    let minimum = manifest["package"]["metadata"]["solidcraft"]["minimum-version"]
        .as_str()
        .ok_or("missing minimum SolidCraft desktop version")?;
    let minimum_version = Version::parse(minimum)?;
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag))?;
    if !minimum_version.pre.is_empty() || minimum_version > version {
        return Err(
            "minimum desktop version must be a stable release at or below the engine".into(),
        );
    }
    for (name, dependency) in manifest["dependencies"]
        .as_table()
        .ok_or("missing dependencies")?
        .iter()
        .chain(
            manifest["patch"]["crates-io"]
                .as_table()
                .ok_or("missing upstream kernel patches")?
                .iter(),
        )
    {
        if dependency.get("git").and_then(Value::as_str) == Some(git)
            && dependency.get("tag").and_then(Value::as_str) != Some(tag)
        {
            return Err(
                format!("{name} must use the same SolidCraft release tag as the engine").into(),
            );
        }
    }
    let packages = lock["package"]
        .as_array()
        .ok_or("missing locked packages")?;
    let locked_engine = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("solidcraft-engine"))
        .ok_or("engine is missing from Cargo.lock")?;
    let locked_source = locked_engine["source"]
        .as_str()
        .ok_or("engine is not Git locked")?;
    if !locked_source.starts_with(&source)
        || locked_engine["version"].as_str() != Some(version.to_string().as_str())
        || packages.iter().any(|package| {
            package
                .get("source")
                .and_then(Value::as_str)
                .is_some_and(|s| s.starts_with(&format!("git+{git}?")) && s != locked_source)
        })
    {
        return Err("SolidCraft lockfile sources must match the manifest tag and revision".into());
    }
    println!("cargo:rustc-env=SOLIDCRAFT_ENGINE_VERSION={version}");
    println!("cargo:rustc-env=SOLIDCRAFT_MINIMUM_VERSION={minimum}");
    Ok(())
}
