use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use serde::Serialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use wait_timeout::ChildExt;

use crate::{
    assembly, cad,
    config::Settings,
    error::{Error, Result},
    inventory::{self, Inventory},
};

#[derive(Debug, Serialize)]
pub struct Asset {
    pub reference: String,
    pub path: PathBuf,
    pub measurement: Value,
}

pub struct Staged {
    dir: Option<TempDir>,
    pub id: String,
    pub inventory: Inventory,
    pub kicad_version: String,
    pub assets: Vec<Asset>,
    pub omissions: Vec<String>,
    pub warnings: Vec<String>,
    pub assembly: Option<assembly::Prepared>,
}

impl Staged {
    pub fn keep(&mut self) -> PathBuf {
        self.dir.take().map(TempDir::keep).unwrap_or_default()
    }
}

pub struct Exporter<'a> {
    pub program: &'a Path,
    pub timeout: Duration,
    pub settings: &'a Settings,
}

/// Board details are tessellated by KiCad, not reconstructed by SolidCraft.
#[derive(Clone, Copy)]
pub struct BoardLayers {
    pub silkscreen: bool,
    pub soldermask: bool,
    pub pads: bool,
    pub copper: bool,
}

impl Default for BoardLayers {
    fn default() -> Self {
        Self {
            silkscreen: true,
            soldermask: true,
            pads: true,
            copper: true,
        }
    }
}

impl BoardLayers {
    /// Read, component models, substrate, board surfaces, assembly preparation, write;
    /// plus each enabled surface layer. Batched exports report their layers together.
    pub fn progress_steps(self) -> usize {
        6 + usize::from(self.silkscreen) * 2
            + usize::from(self.soldermask) * 2
            + usize::from(self.pads)
            + usize::from(self.copper) * 2
    }
}

type Completed<'a> = &'a (dyn Fn(&'static str) + Sync);

enum ExportKind<'a> {
    Models,
    Board(BoardLayers),
    Step(Option<&'a str>),
}

impl Exporter<'_> {
    fn run(
        &self,
        args: &[std::ffi::OsString],
        directory: &Path,
        log: &Path,
    ) -> Result<(bool, String)> {
        let out = File::create(log)?;
        let mut command = Command::new(self.program);
        command
            .args(args)
            .current_dir(directory)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out);
        for (name, prefix) in &self.settings.model_prefixes {
            command.env(name, &prefix.path);
        }
        let mut child = command
            .spawn()
            .map_err(|e| Error::message(format!("cannot start {}: {e}", self.program.display())))?;
        let status = match child.wait_timeout(self.timeout)? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::message(format!(
                    "KiCad export exceeded {} seconds",
                    self.timeout.as_secs()
                )));
            }
        };
        let mut text = String::new();
        File::open(log)?
            .take(4 * 1024 * 1024 + 1)
            .read_to_string(&mut text)?;
        if text.len() > 4 * 1024 * 1024 {
            return Err(Error::message("KiCad diagnostics exceeded 4 MiB"));
        }
        Ok((status.success(), text))
    }

    fn export(
        &self,
        input: &Inventory,
        directory: &Path,
        label: &str,
        kind: ExportKind<'_>,
    ) -> Result<(PathBuf, String, bool)> {
        let format = if matches!(kind, ExportKind::Step(_)) {
            "step"
        } else {
            "glb"
        };
        let step = directory.join(format!("{label}.{format}"));
        let mut args: Vec<std::ffi::OsString> = [
            "pcb",
            "export",
            format,
            "--subst-models",
            "--no-dnp",
            "--user-origin",
            "0x0mm",
            "--output",
        ]
        .iter()
        .map(Into::into)
        .collect();
        args.push(step.clone().into_os_string());
        match kind {
            ExportKind::Models => args.push("--no-board-body".into()),
            ExportKind::Board(layers) => {
                args.push("--board-only".into());
                for (enabled, flag) in [
                    (layers.silkscreen, "--include-silkscreen"),
                    (layers.soldermask, "--include-soldermask"),
                    (layers.pads, "--include-pads"),
                ] {
                    if enabled {
                        args.push(flag.into());
                    }
                }
            }
            ExportKind::Step(Some(reference)) => args.extend([
                "--no-board-body".into(),
                "--component-filter".into(),
                reference.into(),
            ]),
            ExportKind::Step(None) => args.push("--board-only".into()),
        }
        for (key, value) in &self.settings.project_variables {
            args.extend(["--define-var".into(), format!("{key}={value}").into()]);
        }
        args.push(input.source.clone().into_os_string());
        let (success, log) = self.run(
            &args,
            input
                .source
                .parent()
                .ok_or_else(|| Error::message("PCB has no parent"))?,
            &directory.join(format!("{label}.log")),
        )?;
        Ok((step, log, success))
    }

    fn export_copper(
        &self,
        input: &Inventory,
        directory: &Path,
        completed: Completed<'_>,
    ) -> Result<[PathBuf; 2]> {
        let paths = [directory.join("front.svg"), directory.join("back.svg")];
        for (layer, path) in ["F.Cu", "B.Cu"].into_iter().zip(&paths) {
            // Plotting preserves filled zones and drills without reconstructing copper solids.
            // Page mode 1 and scale 1 retain the source's absolute millimetre coordinates.
            let mut args: Vec<std::ffi::OsString> = [
                "pcb",
                "export",
                "svg",
                "--layers",
                layer,
                "--mode-single",
                "--exclude-drawing-sheet",
                "--page-size-mode",
                "1",
                "--scale",
                "1",
                "--black-and-white",
                "--drill-shape-opt",
                "2",
                "--output",
            ]
            .iter()
            .map(Into::into)
            .collect();
            args.push(path.clone().into_os_string());
            for (key, value) in &self.settings.project_variables {
                args.extend(["--define-var".into(), format!("{key}={value}").into()]);
            }
            args.push(input.source.clone().into_os_string());
            let (success, log) = self.run(
                &args,
                input
                    .source
                    .parent()
                    .ok_or_else(|| Error::message("PCB has no parent"))?,
                &path.with_extension("log"),
            )?;
            if !success || export_problem(&log) {
                return Err(Error::message(format!(
                    "KiCad could not plot {layer}\n{}",
                    log.trim()
                )));
            }
            completed(if layer == "F.Cu" {
                "Front copper plotted"
            } else {
                "Back copper plotted"
            });
        }
        Ok(paths)
    }

    pub fn stage(&self, inventory: Inventory, allow_missing: bool) -> Result<Staged> {
        self.stage_impl(inventory, allow_missing, None, &|_| {})
    }

    pub fn stage_build(&self, inventory: Inventory, allow_missing: bool) -> Result<Staged> {
        self.stage_build_with_layers(inventory, allow_missing, BoardLayers::default())
    }

    pub fn stage_build_with_layers(
        &self,
        inventory: Inventory,
        allow_missing: bool,
        layers: BoardLayers,
    ) -> Result<Staged> {
        self.stage_build_with_progress(inventory, allow_missing, layers, &|_| {})
    }

    pub fn stage_build_with_progress(
        &self,
        inventory: Inventory,
        allow_missing: bool,
        layers: BoardLayers,
        completed: Completed<'_>,
    ) -> Result<Staged> {
        self.stage_impl(inventory, allow_missing, Some(layers), completed)
    }

    fn stage_impl(
        &self,
        inventory: Inventory,
        allow_missing: bool,
        layers: Option<BoardLayers>,
        completed: Completed<'_>,
    ) -> Result<Staged> {
        let dir = tempfile::Builder::new()
            .prefix("solidcraft-kicad-")
            .tempdir()?;
        let id = dir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error::message("temporary path is not UTF-8"))?
            .to_string();
        let parent = inventory
            .source
            .parent()
            .ok_or_else(|| Error::message("PCB has no parent"))?;
        let (success, version) =
            self.run(&["version".into()], parent, &dir.path().join("version.log"))?;
        if !success || !version.trim().starts_with("10.") {
            return Err(Error::message(format!(
                "KiCad 10 is required, got: {}",
                version.trim()
            )));
        }
        let mut stage = Staged {
            id,
            kicad_version: version.trim().into(),
            assets: Vec::new(),
            omissions: Vec::new(),
            warnings: inventory.warnings.clone(),
            assembly: None,
            inventory,
            dir: Some(dir),
        };
        let directory = stage
            .dir
            .as_ref()
            .ok_or_else(|| Error::message("missing staging folder"))?
            .path()
            .to_path_buf();
        if let Some(layers) = layers {
            // These jobs are independent; keep board face construction off the model path.
            let (models, board, substrate, copper) = std::thread::scope(|scope| {
                let copper = layers.copper.then(|| {
                    scope.spawn(|| self.export_copper(&stage.inventory, &directory, completed))
                });
                let board = scope.spawn(|| {
                    let result = self.export(
                        &stage.inventory,
                        &directory,
                        "board",
                        ExportKind::Board(layers),
                    )?;
                    if result.2 {
                        completed("PCB surfaces exported");
                        if layers.silkscreen {
                            completed("Front silkscreen exported");
                            completed("Back silkscreen exported");
                        }
                        if layers.soldermask {
                            completed("Front soldermask exported");
                            completed("Back soldermask exported");
                        }
                        if layers.pads {
                            completed("Pads and plating exported");
                        }
                    }
                    Ok::<_, Error>(result)
                });
                let substrate = scope.spawn(|| {
                    let result = self.export(
                        &stage.inventory,
                        &directory,
                        "substrate",
                        ExportKind::Step(None),
                    )?;
                    if result.2 {
                        completed("CAD substrate exported");
                    }
                    Ok::<_, Error>(result)
                });
                let models = if stage
                    .inventory
                    .footprints
                    .iter()
                    .any(|f| !f.dnp && !f.models.is_empty())
                {
                    self.export(&stage.inventory, &directory, "models", ExportKind::Models)
                        .map(Some)
                } else {
                    Ok(None)
                };
                if models
                    .as_ref()
                    .is_ok_and(|m| m.as_ref().is_none_or(|m| m.2))
                {
                    completed("Component models exported");
                }
                let board = board
                    .join()
                    .map_err(|_| Error::message("board export thread failed"))?;
                let substrate = substrate
                    .join()
                    .map_err(|_| Error::message("substrate export thread failed"))?;
                let copper = copper
                    .map(|job| {
                        job.join()
                            .map_err(|_| Error::message("copper export thread failed"))?
                    })
                    .transpose()?;
                Ok::<_, Error>((models?, board?, substrate?, copper))
            })?;
            let (step, log, success) = models
                .map(|(path, log, success)| (Some(path), log, success))
                .unwrap_or((None, String::new(), true));
            let (board, board_log, board_success) = board;
            let (substrate, substrate_log, substrate_success) = substrate;
            if !substrate_success || export_problem(&substrate_log) {
                return Err(Error::message(format!(
                    "KiCad could not export the native substrate\n{}",
                    substrate_log.trim()
                )));
            }
            if !board_success || export_problem(&board_log) {
                return Err(Error::message(format!(
                    "KiCad could not export the board layers\n{}",
                    board_log.trim()
                )));
            }
            if !success || export_problem(&log) {
                if !success || !allow_missing {
                    return Err(Error::message(format!(
                        "KiCad could not export every assigned model\n{}\nOmit --strict-models to permit incomplete component geometry.",
                        log.trim()
                    )));
                }
                stage.omissions.push(
                    log.lines()
                        .filter(|line| export_problem(line))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            let (assembly, parts, omissions) = assembly::prepare_with_copper(
                step.as_deref(),
                &board,
                &substrate,
                &stage.inventory,
                allow_missing,
                copper.as_ref(),
            )?;
            completed("Linked assembly and layer meshes prepared");
            stage.omissions.extend(omissions);
            stage.assets = parts
                .into_iter()
                .map(|p| Asset {
                    path: if p.reference == "PCB" {
                        board.clone()
                    } else {
                        step.as_ref().unwrap_or(&board).clone()
                    },
                    reference: p.reference,
                    measurement: json!({"body_count": p.bodies}),
                })
                .collect();
            stage.assembly = Some(assembly);
            for line in log
                .lines()
                .chain(board_log.lines())
                .chain(substrate_log.lines())
                .filter(|l| l.to_ascii_lowercase().contains("warning"))
            {
                stage.warnings.push(line.into());
            }
            if inventory::hash(&stage.inventory.source)? != stage.inventory.sha256 {
                return Err(Error::message(
                    "PCB changed during staging; no assembly was built",
                ));
            }
            return Ok(stage);
        }
        let mut parts = vec![("PCB".to_string(), None, Vec::new())];
        parts.extend(
            stage
                .inventory
                .footprints
                .iter()
                .filter(|f| !f.dnp && !f.models.is_empty())
                .map(|f| {
                    (
                        f.reference.clone(),
                        Some(f.reference.clone()),
                        f.models.clone(),
                    )
                }),
        );
        for (index, (reference, filter, models)) in parts.into_iter().enumerate() {
            let label = format!("part-{index}");
            let (step, log, success) = self.export(
                &stage.inventory,
                &directory,
                &label,
                ExportKind::Step(filter.as_deref()),
            )?;
            let diagnostic_error = export_problem(&log);
            if !success || diagnostic_error {
                let message = format!(
                    "{reference}: KiCad could not export every assigned model\nmodels: {}\n{}",
                    models.join(", "),
                    log.trim()
                );
                if filter.is_none() || !allow_missing {
                    return Err(Error::message(format!(
                        "{message}\nOmit --strict-models to permit incomplete component geometry."
                    )));
                }
                stage.omissions.push(message);
            }
            let native = directory.join(format!("{label}.solidcraft"));
            match cad::import_asset(&step, &native) {
                Ok(measurement) => stage.assets.push(Asset {
                    reference: reference.clone(),
                    path: native,
                    measurement,
                }),
                Err(error) if filter.is_some() && allow_missing => stage.omissions.push(format!(
                    "{reference}: omitted; models: {}; {error}",
                    models.join(", ")
                )),
                Err(error) => {
                    return Err(Error::message(format!(
                        "{reference}: models {}; {error}\n{}",
                        models.join(", "),
                        log.trim()
                    )));
                }
            }
            if !diagnostic_error && !log.trim().is_empty() {
                for line in log
                    .lines()
                    .filter(|l| l.to_ascii_lowercase().contains("warning"))
                {
                    stage.warnings.push(format!("{reference}: {line}"));
                }
            }
        }
        if inventory::hash(&stage.inventory.source)? != stage.inventory.sha256 {
            return Err(Error::message(
                "PCB changed during staging; no assembly was inserted",
            ));
        }
        Ok(stage)
    }
}

fn export_problem(log: &str) -> bool {
    let log = log.to_ascii_lowercase();
    [
        "could not add 3d model",
        "file not found",
        "error:",
        "failed to",
        "unable to",
        "cannot load",
        "could not load",
        "cannot read",
        "could not read",
    ]
    .iter()
    .any(|s| log.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successful_exit_does_not_hide_missing_model_diagnostics() {
        assert!(export_problem(
            "Could not add 3D model for U1.\nFile not found: ${CUSTOM}/GPS.STEP\nSTEP file created."
        ));
        assert!(!export_problem(
            "Warning: Legacy zone fill strategy is not supported anymore.\nSTEP file created."
        ));
    }
}
