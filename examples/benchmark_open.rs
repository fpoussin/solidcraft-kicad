//! Measure actual native-file loading/evaluation, independently of conversion.
use std::{path::PathBuf, time::Instant};

use serde_json::json;
use solidcraft_kicad::{
    cad,
    error::{Error, Result},
};
use solidcraft_mcp::{Backend, Headless};

fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| Error::message("usage: benchmark_open FILE.solidcraft [PREVIEW.png]"))?;
    let mut backend = Headless::default();
    let start = Instant::now();
    cad::execute(
        &mut backend,
        "doc.open",
        json!({"path": std::path::absolute(path)?}),
    )?;
    let open_seconds = start.elapsed().as_secs_f64();
    let world = backend.session.world_state();
    let total_seconds = start.elapsed().as_secs_f64();
    let errors: Vec<_> = backend
        .session
        .model
        .results
        .iter()
        .filter_map(|r| r.error.as_deref())
        .collect();
    if !errors.is_empty() {
        return Err(Error::message(errors.join("; ")));
    }
    let render_seconds = if let Some(output) = std::env::args_os().nth(2) {
        solidcraft_engine::capture::install(std::sync::Arc::new(|scene, camera, width, height| {
            Ok(solidcraft_engine::capture::Picture {
                rgba: solidcraft_engine::render::render(
                    scene,
                    camera,
                    width as usize,
                    height as usize,
                )
                .rgba,
                renderer: "cpu",
            })
        }));
        let render_start = Instant::now();
        backend.call("ui.render", json!({"path": std::path::absolute(PathBuf::from(output))?, "width": 1280, "height": 900, "grid": false, "axes": false})).map_err(Error::message)?;
        Some(render_start.elapsed().as_secs_f64())
    } else {
        None
    };
    println!(
        "{}",
        json!({"open_seconds": open_seconds, "world_ready_seconds": total_seconds, "render_seconds": render_seconds, "components": backend.session.doc.components.len(), "occurrences": backend.session.doc.occurrences.len(), "bodies": world.bodies.len(), "mesh_bodies": world.bodies.iter().filter(|b| b.body.is_mesh()).count(), "coloured_bodies": world.bodies.iter().filter(|b| b.body.color().is_some()).count()})
    );
    Ok(())
}
