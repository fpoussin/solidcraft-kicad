//! Flat outer copper from KiCad plots: union once, partition by mask, batch by appearance.
use std::path::PathBuf;

use i_triangle::{
    float::triangulatable::Triangulatable,
    i_overlay::{
        core::{fill_rule::FillRule, overlay_rule::OverlayRule},
        float::{simplify::SimplifyShape, single::SingleFloatOverlay},
    },
};
use kurbo::{BezPath, PathEl};
use solidcraft_engine::doc::{MeshData, appearance::Look, pbr_appearance::PbrParameters};
use usvg::{Node, Paint, tiny_skia_path::PathSegment};

use crate::{
    assembly::ModelMesh,
    error::{Error, Result},
};

type Contours = Vec<Vec<[f64; 2]>>;
type Shapes = Vec<Contours>;
const CURVE_TOLERANCE_MM: f64 = 0.005;

pub(crate) fn apply(meshes: &mut Vec<ModelMesh>, paths: &[PathBuf; 2]) -> Result<()> {
    let substrate = meshes
        .iter()
        .find(|m| material(m, 0.8))
        .ok_or_else(|| Error::message("no substrate for copper clipping"))?;
    let low = substrate
        .mesh
        .positions
        .iter()
        .skip(2)
        .step_by(3)
        .copied()
        .fold(f64::INFINITY, f64::min);
    let high = substrate
        .mesh
        .positions
        .iter()
        .skip(2)
        .step_by(3)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let outlines = [
        plane_shapes(&substrate.mesh, high),
        plane_shapes(&substrate.mesh, low),
    ];
    let exposed_look = meshes
        .iter()
        .filter_map(|m| m.look.as_ref())
        .find(|l| l.pbr.is_some_and(|p| p.metallic() == 1.))
        .cloned();
    for (side, path) in paths.iter().enumerate() {
        let top = side == 0;
        let name = if top { "Top" } else { "Bottom" };
        let copper = read_plot(&std::fs::read(path)?)?.overlay(
            &outlines[side],
            OverlayRule::Intersect,
            FillRule::NonZero,
        );
        if copper.is_empty() {
            continue;
        }
        let mut exposed = copper.clone();
        if let Some(index) = meshes.iter().position(|m| {
            material(m, 0.6)
                && m.mesh
                    .positions
                    .get(2)
                    .is_some_and(|z| (*z > (low + high) * 0.5) == top)
        }) {
            let mask = &meshes[index];
            let z = mask.mesh.positions[2];
            let mask_shapes = plane_shapes(&mask.mesh, z);
            let covered = copper.overlay(&mask_shapes, OverlayRule::Intersect, FillRule::NonZero);
            exposed = copper.overlay(&mask_shapes, OverlayRule::Difference, FillRule::NonZero);
            let plain = mask_shapes.overlay(&copper, OverlayRule::Difference, FillRule::NonZero);
            let look = mask.look.clone();
            meshes[index] = surface(&plain, z, top, look.clone());
            if !covered.is_empty() {
                let mut tint =
                    look.ok_or_else(|| Error::message("soldermask has no appearance"))?;
                tint.name = format!("{name} copper under soldermask");
                // Bake the opaque coating's slight copper relief into its colour/roughness.
                // No transparency sorting, tiny layer offsets, or overlapping faces are needed.
                tint.color = tint
                    .color
                    .map(|v| (f64::from(v) * 1.22 + 2.).min(255.).round() as u8);
                tint.pbr = PbrParameters::new(0., 0.5, 1.5).ok();
                meshes.push(surface(&covered, z, top, Some(tint)));
            }
        }
        if !exposed.is_empty() {
            let mut look = exposed_look.clone().unwrap_or(Look {
                name: String::new(),
                color: [184, 115, 50],
                opacity: 1.,
                pbr: PbrParameters::new(1., 0.4, 1.5).ok(),
            });
            look.name = format!("{name} exposed copper");
            let z = if top { high + 0.035 } else { low - 0.035 };
            meshes.push(surface(&exposed, z, top, Some(look)));
        }
    }
    meshes.retain(|m| !m.mesh.triangles.is_empty());
    Ok(())
}

fn material(mesh: &ModelMesh, roughness: f64) -> bool {
    mesh.look
        .as_ref()
        .and_then(|l| l.pbr)
        .is_some_and(|p| p.metallic() == 0. && (p.roughness() - roughness).abs() < 1e-5)
}

// Recover a union of a horizontal mesh's triangles, preserving all cutouts.
fn plane_shapes(mesh: &MeshData, z: f64) -> Shapes {
    let mut contours = Contours::new();
    for triangle in mesh.triangles.as_chunks::<3>().0 {
        let points: Vec<_> = triangle
            .iter()
            .filter_map(|&i| mesh.positions.get(i as usize * 3..i as usize * 3 + 3))
            .collect();
        if points.len() != 3 || points.iter().any(|p| (p[2] - z).abs() > 1e-5) {
            continue;
        }
        let mut contour: Vec<_> = points.iter().map(|p| [p[0], p[1]]).collect();
        let a = contour[0];
        let b = contour[1];
        let c = contour[2];
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-12 {
            continue;
        }
        if area < 0. {
            contour.reverse();
        }
        contours.push(contour);
    }
    contours.simplify_shape(FillRule::NonZero)
}

fn surface(shapes: &Shapes, z: f64, top: bool, look: Option<Look>) -> ModelMesh {
    let triangulation = shapes.triangulate().to_triangulation::<u32>();
    let mut triangles = triangulation.indices;
    if !top {
        for t in triangles.as_chunks_mut::<3>().0 {
            t.swap(1, 2);
        }
    }
    ModelMesh {
        mesh: MeshData {
            name: String::new(),
            positions: triangulation
                .points
                .into_iter()
                // Plots use four decimal places in mm. Quantise generated XY only
                // to 0.1 micrometre so vertex compaction merges boolean-operation roundoff.
                // Keep the original mask Z so adjacent surfaces remain coplanar.
                .flat_map(|[x, y]| [(x * 1e4).round() / 1e4, (y * 1e4).round() / 1e4, z])
                .collect(),
            triangles,
            color: look.as_ref().map(Look::rgb),
        },
        look,
    }
}

fn read_plot(bytes: &[u8]) -> Result<Shapes> {
    // At 25.4 DPI usvg resolves physical millimetres directly to user units.
    let options = usvg::Options {
        dpi: 25.4,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options)
        .map_err(|e| Error::message(format!("invalid KiCad copper plot: {e}")))?;
    let mut copper = Shapes::new();
    let mut holes = Shapes::new();
    collect(tree.root(), &mut copper, &mut holes)?;
    Ok(copper.overlay(&holes, OverlayRule::Difference, FillRule::NonZero))
}

fn collect(group: &usvg::Group, copper: &mut Shapes, holes: &mut Shapes) -> Result<()> {
    for node in group.children() {
        match node {
            Node::Group(group) => collect(group, copper, holes)?,
            Node::Path(path) if path.is_visible() => {
                if let Some(fill) = path.fill() {
                    let rule = match fill.rule() {
                        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
                        _ => FillRule::NonZero,
                    };
                    let contours = flatten(path.data(), path.abs_transform());
                    target(fill.paint(), copper, holes)?.extend(contours.simplify_shape(rule));
                }
                if let Some(stroke) = path.stroke()
                    && let Some(outline) = path.data().stroke(&stroke.to_tiny_skia(), 10.)
                {
                    let contours = flatten(&outline, path.abs_transform());
                    target(stroke.paint(), copper, holes)?
                        .extend(contours.simplify_shape(FillRule::NonZero));
                }
            }
            Node::Path(_) => (),
            _ => return Err(Error::message("copper plot contains non-vector content")),
        }
    }
    Ok(())
}

fn target<'a>(
    paint: &Paint,
    copper: &'a mut Shapes,
    holes: &'a mut Shapes,
) -> Result<&'a mut Shapes> {
    match paint {
        Paint::Color(c) if c.red == 0 && c.green == 0 && c.blue == 0 => Ok(copper),
        Paint::Color(c) if c.red == 255 && c.green == 255 && c.blue == 255 => Ok(holes),
        _ => Err(Error::message("expected a monochrome KiCad copper plot")),
    }
}

fn flatten(path: &usvg::tiny_skia_path::Path, transform: usvg::Transform) -> Contours {
    let mut curve = BezPath::new();
    let at = |mut p: usvg::tiny_skia_path::Point| {
        transform.map_point(&mut p);
        (f64::from(p.x), -f64::from(p.y))
    };
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => curve.move_to(at(p)),
            PathSegment::LineTo(p) => curve.line_to(at(p)),
            PathSegment::QuadTo(a, b) => curve.quad_to(at(a), at(b)),
            PathSegment::CubicTo(a, b, c) => curve.curve_to(at(a), at(b), at(c)),
            PathSegment::Close => curve.close_path(),
        }
    }
    let mut contours = Contours::new();
    let mut current = Vec::new();
    kurbo::flatten(
        curve.elements().iter().copied(),
        CURVE_TOLERANCE_MM,
        |element| match element {
            PathEl::MoveTo(p) => {
                if current.len() >= 3 {
                    contours.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
                current.push([p.x, p.y]);
            }
            PathEl::LineTo(p) => current.push([p.x, p.y]),
            PathEl::ClosePath => {
                if current.len() >= 3 {
                    contours.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
            _ => (),
        },
    );
    if current.len() >= 3 {
        contours.push(current);
    }
    contours
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot(content: &str) -> Vec<u8> {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="40mm" viewBox="0 0 40 40">{content}</svg>"#).into_bytes()
    }

    fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Shapes {
        vec![vec![vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]]]]
    }

    fn area(mesh: &MeshData) -> f64 {
        mesh.triangles
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let p: Vec<_> = t
                    .iter()
                    .map(|&i| &mesh.positions[i as usize * 3..i as usize * 3 + 3])
                    .collect();
                ((p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                    - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]))
                    .abs()
                    * 0.5
            })
            .sum()
    }

    #[test]
    fn plots_union_overlaps_and_subtract_round_and_slotted_drills() -> Result<()> {
        let shapes = read_plot(&plot(
            r#"<g fill="black"><rect x="10" y="20" width="10" height="10"/><rect x="15" y="20" width="10" height="10"/></g><g fill="white"><circle cx="12" cy="23" r="1"/><path d="M 20,22 L 20,24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"/></g>"#,
        ))?;
        let mesh = surface(&shapes, 0., true, None).mesh;
        let expected = 150. - 4. - 2. * std::f64::consts::PI;
        assert!(
            (area(&mesh) - expected).abs() < 0.08,
            "{} != {expected}",
            area(&mesh)
        );
        // Plot Y is inverted, but X retains the source's absolute coordinate.
        assert!(
            mesh.positions
                .as_chunks::<3>()
                .0
                .iter()
                .all(|p| p[0] >= 10. - 1e-5
                    && p[0] <= 25. + 1e-5
                    && p[1] >= -30. - 1e-5
                    && p[1] <= -20. + 1e-5)
        );
        Ok(())
    }

    #[test]
    fn plot_arcs_transforms_and_nested_zone_holes_are_preserved() -> Result<()> {
        let shapes = read_plot(&plot(
            r#"<g transform="translate(10 20)"><path fill="black" fill-rule="evenodd" d="M0,0 H10 V10 H0 Z M2,2 H8 V8 H2 Z"/><path fill="none" stroke="black" stroke-width="1" stroke-linecap="round" d="M15,0 A5,5 0 0 1 20,5"/></g>"#,
        ))?;
        let mesh = surface(&shapes, 0., true, None).mesh;
        let expected = 64. + std::f64::consts::PI * 2.5 + std::f64::consts::PI * 0.25;
        assert!((area(&mesh) - expected).abs() < 0.1);
        Ok(())
    }

    #[test]
    fn copper_partitions_each_mask_without_overlaps_or_per_track_bodies() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = [dir.path().join("front.svg"), dir.path().join("back.svg")];
        let mut substrate = surface(&rectangle(10., -30., 10., 10.), 0., false, None);
        let top = surface(&rectangle(10., -30., 10., 10.), 1.6, true, None);
        let offset = substrate.mesh.positions.len() as u32 / 3;
        substrate.mesh.positions.extend(top.mesh.positions);
        substrate
            .mesh
            .triangles
            .extend(top.mesh.triangles.into_iter().map(|i| i + offset));
        substrate.look = Some(Look {
            name: "core".into(),
            color: [100, 100, 0],
            opacity: 1.,
            pbr: PbrParameters::new(0., 0.8, 1.5).ok(),
        });
        let mut meshes = vec![substrate];
        for (side, color) in [[10, 60, 30], [30, 10, 60]].into_iter().enumerate() {
            std::fs::write(
                &paths[side],
                plot(r#"<rect x="5" y="20" width="20" height="5" fill="black"/>"#),
            )?;
            let look = Look {
                name: format!("mask-{side}"),
                color,
                opacity: 1.,
                pbr: PbrParameters::new(0., 0.6, 1.5).ok(),
            };
            meshes.push(surface(
                &rectangle(10., -30., 5., 10.),
                if side == 0 { 1.65 } else { -0.05 },
                side == 0,
                Some(look),
            ));
        }
        apply(&mut meshes, &paths)?;
        // One core, two plain mask areas, two covered copper areas, two exposed areas.
        assert_eq!(meshes.len(), 7);
        for (side, z) in [("Top", 1.65), ("Bottom", -0.05)] {
            let coated = meshes
                .iter()
                .find(|m| {
                    m.look
                        .as_ref()
                        .is_some_and(|l| l.name == format!("{side} copper under soldermask"))
                })
                .ok_or_else(|| Error::message("covered copper missing"))?;
            assert!((area(&coated.mesh) - 25.).abs() < 1e-4);
            assert!(
                coated
                    .mesh
                    .positions
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .all(|p| (p[2] - z).abs() < 1e-8)
            );
            let bare = meshes
                .iter()
                .find(|m| {
                    m.look
                        .as_ref()
                        .is_some_and(|l| l.name == format!("{side} exposed copper"))
                })
                .ok_or_else(|| Error::message("bare copper missing"))?;
            assert!((area(&bare.mesh) - 25.).abs() < 1e-4);
            assert!(
                bare.look
                    .as_ref()
                    .and_then(|l| l.pbr)
                    .is_some_and(|p| p.metallic() == 1.)
            );
        }
        let covered_colors: Vec<_> = meshes
            .iter()
            .filter_map(|m| m.look.as_ref())
            .filter(|l| l.name.ends_with("under soldermask"))
            .map(|l| l.color)
            .collect();
        assert_ne!(covered_colors[0], covered_colors[1]);
        Ok(())
    }
}
