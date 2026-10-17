use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use figment::{
    Figment,
    providers::{Format, Toml},
    value::magic::RelativePathBuf,
};
use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    model_prefixes: BTreeMap<String, RelativePathBuf>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Prefix {
    pub path: PathBuf,
    pub source: String,
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Settings {
    pub model_prefixes: BTreeMap<String, Prefix>,
    pub project_variables: BTreeMap<String, String>,
}

pub fn pair(value: &str) -> std::result::Result<(String, String), String> {
    let (name, value) = value.split_once('=').ok_or("expected NAME=VALUE")?;
    let mut chars = name.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        || value.is_empty()
    {
        return Err("expected a variable name and nonempty value (NAME=VALUE)".into());
    }
    if name == "KIPRJMOD" {
        return Err("KIPRJMOD is defined by KiCad and cannot be overridden".into());
    }
    Ok((name.into(), value.into()))
}

pub fn load(
    board: &Path,
    explicit: Option<&Path>,
    prefixes: &[(String, String)],
    defines: &[(String, String)],
) -> Result<Settings> {
    let user =
        ProjectDirs::from("", "", "solidcraft-kicad").map(|d| d.config_dir().join("config.toml"));
    load_with_user(board, explicit, user.as_deref(), prefixes, defines)
}

fn project_config(board: &Path) -> Option<PathBuf> {
    board
        .parent()?
        .ancestors()
        .map(|p| p.join("solidcraft-kicad.toml"))
        .find(|p| p.is_file())
}

fn load_with_user(
    board: &Path,
    explicit: Option<&Path>,
    user: Option<&Path>,
    prefixes: &[(String, String)],
    defines: &[(String, String)],
) -> Result<Settings> {
    let mut figment = Figment::new();
    if let Some(user) = user.filter(|p| p.is_file()) {
        figment = figment.merge(Toml::file(user));
    }
    let project = match explicit {
        Some(path) => {
            if !path.is_file() {
                return Err(Error::message(format!(
                    "configuration does not exist: {}",
                    path.display()
                )));
            }
            Some(path.to_path_buf())
        }
        None => project_config(board),
    };
    if let Some(project) = project {
        figment = figment.merge(Toml::file(project));
    }
    let config: Config = figment.extract()?;
    let mut settings = Settings::default();
    for (name, value) in config.model_prefixes {
        pair(&format!("{name}=x")).map_err(Error::message)?;
        settings.model_prefixes.insert(
            name,
            Prefix {
                path: std::path::absolute(value.relative())?,
                source: value
                    .metadata_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "configuration".into()),
            },
        );
    }
    let mut cli = BTreeMap::new();
    for (name, value) in prefixes {
        if cli.insert(name, value).is_some_and(|old| old != value) {
            return Err(Error::message(format!(
                "conflicting --model-prefix values for {name}"
            )));
        }
        settings.model_prefixes.insert(
            name.clone(),
            Prefix {
                path: std::path::absolute(value)?,
                source: "command line".into(),
            },
        );
    }
    for (name, value) in defines {
        if let Some(prefix) = settings.model_prefixes.get(name)
            && std::path::absolute(value)? != prefix.path
        {
            return Err(Error::message(format!(
                "conflicting model-prefix and project-variable overrides for {name}"
            )));
        }
        if settings
            .project_variables
            .insert(name.clone(), value.clone())
            .is_some_and(|old| old != *value)
        {
            return Err(Error::message(format!(
                "conflicting --define-var values for {name}"
            )));
        }
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layered_paths_keep_their_own_config_origin() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let user = dir.path().join("user.toml");
        let project = dir.path().join("project");
        std::fs::create_dir_all(project.join("boards/gps"))?;
        std::fs::write(
            &user,
            "[model_prefixes]\nSHARED = 'user-models'\nCUSTOM = 'old'\n",
        )?;
        std::fs::write(
            project.join("solidcraft-kicad.toml"),
            "[model_prefixes]\nCUSTOM = 'project-models'\n",
        )?;
        let board = project.join("boards/gps/input");
        let s = load_with_user(&board, None, Some(&user), &[], &[])?;
        assert_eq!(
            s.model_prefixes["SHARED"].path,
            dir.path().join("user-models")
        );
        assert_eq!(
            s.model_prefixes["CUSTOM"].path,
            project.join("project-models")
        );
        let cli_path = dir.path().join("cli-models");
        let cli = vec![("CUSTOM".into(), cli_path.to_string_lossy().into_owned())];
        let s = load_with_user(&board, None, Some(&user), &cli, &[])?;
        assert_eq!(s.model_prefixes["CUSTOM"].path, cli_path);
        assert!(
            load_with_user(
                &board,
                None,
                Some(&user),
                &cli,
                &[("CUSTOM".into(), "/different".into())]
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn explicit_config_replaces_project_discovery_and_reserved_prefix_fails() -> Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(
            dir.path().join("solidcraft-kicad.toml"),
            "[model_prefixes]\nWRONG = '/wrong'\n",
        )?;
        let explicit = dir.path().join("explicit.toml");
        std::fs::write(&explicit, "[model_prefixes]\nRIGHT = '/right'\n")?;
        let s = load_with_user(&dir.path().join("board"), Some(&explicit), None, &[], &[])?;
        assert!(!s.model_prefixes.contains_key("WRONG"));
        assert!(s.model_prefixes.contains_key("RIGHT"));
        assert!(pair("KIPRJMOD=/tmp").is_err());
        std::fs::write(&explicit, "[model_prefixes]\nKIPRJMOD = '/wrong'\n")?;
        assert!(
            load_with_user(&dir.path().join("board"), Some(&explicit), None, &[], &[]).is_err()
        );
        Ok(())
    }
}
