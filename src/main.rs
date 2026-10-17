use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use solidcraft_kicad::{
    cad, config,
    error::{Error, Result},
    inventory,
    progress::Progress,
    staging::{BoardLayers, Exporter},
};
use solidcraft_mcp::Remote;

#[cfg(any(test, not(debug_assertions)))]
mod update;

#[derive(Parser)]
#[command(
    version,
    about = "Import KiCad PCBs as native SolidCraft component assemblies"
)]
struct Cli {
    /// Disable the release-build update check (interactive terminals only).
    #[arg(long, global = true)]
    no_update_check: bool,
    /// Print the detailed JSON report on success.
    #[arg(long, global = true)]
    json: bool,
    /// Include detailed geometry measurements in the JSON report (can be slow).
    #[arg(long, global = true, requires = "json")]
    measure: bool,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Create a self-contained native SolidCraft assembly.
    Build {
        #[command(flatten)]
        input: Input,
        /// Destination file (defaults to the PCB filename in the current directory).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Overwrite an existing output file without prompting.
        #[arg(short, long)]
        force: bool,
        #[command(flatten)]
        layers: LayerOptions,
    },
    /// Insert into the active running SolidCraft assembly, without saving it.
    Insert {
        #[command(flatten)]
        input: Input,
        #[arg(long, default_value_t = 7878, value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
    },
}

#[derive(Args)]
struct LayerOptions {
    /// Omit front and back silkscreen (included by default).
    #[arg(long)]
    no_silkscreen: bool,
    /// Omit soldermask surfaces and openings (included by default).
    #[arg(long)]
    no_soldermask: bool,
    /// Omit pads and plated holes (included by default).
    #[arg(long)]
    no_pads: bool,
    /// Omit outer copper tracks, vias and zones (included by default).
    #[arg(long)]
    no_copper: bool,
}

#[derive(Args)]
struct Input {
    /// Source PCB. It and its project settings stay in their original location.
    board: PathBuf,
    /// Replace automatic project-config discovery with this TOML file.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override a KiCad model-path prefix for this invocation only.
    #[arg(long, value_name = "NAME=PATH", value_parser = config::pair)]
    model_prefix: Vec<(String, String)>,
    /// Override a KiCad project variable.
    #[arg(long, value_name = "KEY=VALUE", value_parser = config::pair)]
    define_var: Vec<(String, String)>,
    /// Permit omitted/partial component geometry (the default).
    #[arg(long)]
    allow_missing_models: bool,
    /// Fail if an assigned component model is missing or unreadable.
    #[arg(long, conflicts_with = "allow_missing_models")]
    strict_models: bool,
    /// KiCad 10 executable (defaults to PATH).
    #[arg(long, default_value = "kicad-cli")]
    kicad_cli: PathBuf,
    /// Maximum seconds per KiCad subprocess.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=3600))]
    timeout: u64,
}

fn run(cli: Cli) -> Result<Value> {
    let measure = cli.measure;
    let (input, output, port, force, layers) = match cli.command {
        Action::Build {
            input,
            output,
            force,
            layers,
        } => {
            let output = match output {
                Some(path) => path,
                None => PathBuf::from(
                    input
                        .board
                        .file_name()
                        .ok_or_else(|| Error::message("source PCB has no filename"))?,
                )
                .with_extension("solidcraft"),
            };
            (
                input,
                Some(std::path::absolute(output)?),
                None,
                force,
                BoardLayers {
                    silkscreen: !layers.no_silkscreen,
                    soldermask: !layers.no_soldermask,
                    pads: !layers.no_pads,
                    copper: !layers.no_copper,
                },
            )
        }
        Action::Insert { input, port } => (input, None, Some(port), false, BoardLayers::default()),
    };
    let mut overwrite = false;
    if let Some(output) = &output {
        if output
            .extension()
            .and_then(|e| e.to_str())
            .is_none_or(|e| !e.eq_ignore_ascii_case("solidcraft"))
        {
            return Err(Error::message("--output must end in .solidcraft"));
        }
        overwrite = confirm_overwrite(output, force)?;
    }
    let progress = Progress::with_total(if output.is_some() {
        layers.progress_steps()
    } else {
        3
    });
    progress.stage(0, "Reading PCB");
    let inventory = inventory::read(&input.board)?;
    let settings = config::load(
        &inventory.source,
        input.config.as_deref(),
        &input.model_prefix,
        &input.define_var,
    )?;
    let mut remote = port
        .map(|port| Remote::connect(&format!("127.0.0.1:{port}")))
        .transpose()?;
    let before = remote.as_mut().map(|b| cad::destination(b)).transpose()?;
    let program = if input.kicad_cli.components().count() > 1 {
        std::path::absolute(&input.kicad_cli)?
    } else {
        input.kicad_cli
    };
    let exporter = Exporter {
        program: &program,
        timeout: Duration::from_secs(input.timeout),
        settings: &settings,
    };
    let allow_missing = input.allow_missing_models || !input.strict_models;
    progress.advance("PCB read");
    progress.label("Exporting layers and component models");
    let mut stage = if output.is_some() {
        exporter.stage_build_with_progress(inventory, allow_missing, layers, &|label| {
            progress.advance(label)
        })?
    } else {
        let stage = exporter.stage(inventory, allow_missing)?;
        progress.advance("Components exported");
        stage
    };
    let result = if let Some(output) = output {
        progress.label("Writing SolidCraft file");
        cad::build_with_options(&stage, &output, measure, overwrite)?
    } else {
        progress.label("Inserting components");
        let b = remote
            .as_mut()
            .ok_or_else(|| Error::message("missing SolidCraft connection"))?;
        cad::insert(
            b,
            &before.ok_or_else(|| Error::message("missing destination snapshot"))?,
            &mut stage,
        )?
    };
    progress.advance("Complete");
    drop(progress);
    for warning in stage.warnings.iter().chain(&stage.omissions) {
        eprintln!("Warning: {warning}");
    }
    Ok(json!({
        "result": result, "source": stage.inventory.source,
        "source_sha256": stage.inventory.sha256, "kicad_version": stage.kicad_version,
        "solidcraft_version": "0.14.0", "assigned_models_complete": stage.omissions.is_empty(),
        "parts": stage.assets.iter().map(|a| &a.reference).collect::<Vec<_>>(),
        "model_prefix_overrides": settings.model_prefixes, "warnings": stage.warnings, "omissions": stage.omissions,
    }))
}

fn confirm_overwrite(output: &Path, force: bool) -> Result<bool> {
    if force || !output.exists() {
        return Ok(force);
    }
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(Error::message(format!(
            "output already exists: {}; use --force to overwrite",
            output.display()
        )));
    }
    eprint!("Overwrite {}? [Y/n] ", output.display());
    io::stderr().flush()?;
    let mut answer = String::new();
    let read = io::stdin().read_line(&mut answer)?;
    let answer = answer.trim();
    if read > 0
        && (answer.is_empty()
            || answer.eq_ignore_ascii_case("y")
            || answer.eq_ignore_ascii_case("yes"))
    {
        Ok(true)
    } else {
        Err(Error::message("overwrite cancelled"))
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    #[cfg(not(debug_assertions))]
    let updates =
        update::Check::start(!cli.no_update_check && !cli.json && io::stderr().is_terminal());
    let print_json = cli.json;
    let status = match run(cli) {
        Ok(report) if print_json => match serde_json::to_string_pretty(&report) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        },
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    };
    #[cfg(not(debug_assertions))]
    updates.notify();
    status
}
