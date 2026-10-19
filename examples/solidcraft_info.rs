//! Metadata from the actual linked engine and the manifest's compatibility policy.
fn main() {
    use solidcraft_kicad::cad;
    println!(
        "{}",
        serde_json::json!({
            "version": cad::SOLIDCRAFT_VERSION,
            "minimum_version": cad::MIN_SOLIDCRAFT_VERSION,
            "file_format": cad::SOLIDCRAFT_FORMAT,
        })
    );
}
