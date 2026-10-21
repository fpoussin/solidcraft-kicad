//! Download and validate everything before touching the installed executable.
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::Path,
    time::Duration,
};

use sha2::{Digest, Sha256};

const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
const MAX_BINARY: u64 = 256 * 1024 * 1024;
const RELEASES: &str = "https://github.com/fpoussin/solidcraft-kicad/releases/download";

pub(super) fn asset_name(version: &str, target: &str) -> Result<String, String> {
    let extension = match target {
        "x86_64-unknown-linux-gnu" | "aarch64-apple-darwin" => "tar.gz",
        "x86_64-pc-windows-msvc" => "zip",
        _ => return Err(format!("no release binary for {target}")),
    };
    // Also ensure cache contents cannot become a URL/path fragment.
    if super::stable_version(version).is_none() {
        return Err("invalid stable release version".into());
    }
    Ok(format!("solidcraft-kicad-v{version}-{target}.{extension}"))
}

pub(super) fn install(version: &str, target: &str) -> Result<(), String> {
    install_with(version, target, download, replace_binary)
}

fn replace_binary(binary: &Path) -> io::Result<()> {
    // Windows replacement first moves the locked executable aside. Keep a
    // recovery copy so a later failure cannot leave its installation path empty.
    #[cfg(windows)]
    let backup = {
        let executable = std::env::current_exe()?.canonicalize()?;
        let parent = executable
            .parent()
            .ok_or_else(|| io::Error::other("no installation directory"))?;
        let backup = tempfile::NamedTempFile::new_in(parent)?;
        std::fs::copy(&executable, backup.path())?;
        (executable, backup)
    };
    let result = self_replace::self_replace(binary);
    #[cfg(windows)]
    if result.is_err()
        && !backup.0.exists()
        && let Err(error) = backup.1.persist_noclobber(&backup.0)
    {
        let restore_error = error.error.to_string();
        let (_, recovery) = error.file.keep().map_err(|error| error.error)?;
        return Err(io::Error::other(format!(
            "update and recovery failed: {restore_error}; previous executable saved at {}",
            recovery.display()
        )));
    }
    result.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{error}; check installation permissions"),
        )
    })
}

fn install_with(
    version: &str,
    target: &str,
    mut download: impl FnMut(&str, &mut File, u64) -> Result<(), String>,
    replace: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), String> {
    let name = asset_name(version, target)?;
    let base = format!("{RELEASES}/v{version}");
    let temporary = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut sums = tempfile::tempfile().map_err(|error| error.to_string())?;
    download(&format!("{base}/SHA256SUMS"), &mut sums, 64 * 1024)?;
    sums.rewind().map_err(|error| error.to_string())?;
    let mut manifest = String::new();
    sums.read_to_string(&mut manifest)
        .map_err(|error| error.to_string())?;
    let expected = checksum(&manifest, &name)?;

    let mut archive = tempfile::tempfile().map_err(|error| error.to_string())?;
    download(&format!("{base}/{name}"), &mut archive, MAX_ARCHIVE)?;
    archive.rewind().map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    io::copy(&mut archive, &mut digest).map_err(|error| error.to_string())?;
    if format!("{:x}", digest.finalize()) != expected {
        return Err(format!("SHA-256 checksum mismatch for {name}"));
    }
    archive.rewind().map_err(|error| error.to_string())?;

    let binary = temporary.path().join(if target.ends_with("windows-msvc") {
        "solidcraft-kicad.exe"
    } else {
        "solidcraft-kicad"
    });
    extract(archive, &name, &binary)
        .map_err(|error| format!("invalid release archive: {error}"))?;
    // self-replace stages beside the installed executable, then swaps it in;
    // it also handles the running executable lock and cleanup on Windows.
    replace(&binary).map_err(|error| format!("cannot replace running executable: {error}"))
}

fn download(url: &str, output: &mut File, limit: u64) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .https_only(true)
        .user_agent(concat!("solidcraft-kicad/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| format!("cannot download {url}: {error}"))?;
    copy_limited(response.body_mut().as_reader(), output, limit)
        .map_err(|error| format!("cannot download {url}: {error}"))
}

fn checksum(manifest: &str, name: &str) -> Result<String, String> {
    let mut found = None;
    for line in manifest.lines() {
        let mut fields = line.split_whitespace();
        let (Some(hash), Some(file)) = (fields.next(), fields.next()) else {
            continue;
        };
        if file.trim_start_matches('*') != name {
            continue;
        }
        if found.is_some()
            || fields.next().is_some()
            || hash.len() != 64
            || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!("invalid SHA256SUMS entry for {name}"));
        }
        found = Some(hash.to_ascii_lowercase());
    }
    found.ok_or_else(|| format!("SHA256SUMS has no checksum for {name}"))
}

fn copy_limited(input: impl Read, output: &mut impl Write, limit: u64) -> io::Result<()> {
    let copied = io::copy(&mut input.take(limit + 1), output)?;
    if copied == 0 || copied > limit {
        return Err(io::Error::other("empty file or size limit exceeded"));
    }
    Ok(())
}

fn extract(archive: File, name: &str, binary: &Path) -> io::Result<()> {
    let (base, is_zip) = if let Some(base) = name.strip_suffix(".zip") {
        (base, true)
    } else {
        (
            name.strip_suffix(".tar.gz")
                .ok_or_else(|| io::Error::other("unsupported archive format"))?,
            false,
        )
    };
    let member = format!(
        "{base}/{}",
        if is_zip {
            "solidcraft-kicad.exe"
        } else {
            "solidcraft-kicad"
        }
    );
    let mut output = File::create(binary)?;
    // Read only the exact regular-file member; never unpack archive paths.
    if is_zip {
        let mut archive = zip::ZipArchive::new(archive)?;
        let mut entry = archive.by_name(&member)?;
        if !entry.is_file() || entry.is_symlink() {
            return Err(io::Error::other("executable is not a regular file"));
        }
        copy_limited(&mut entry, &mut output, MAX_BINARY)?;
    } else {
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        let mut found = false;
        for entry in archive.entries()? {
            let mut entry = entry?;
            if entry.path()? != Path::new(&member) {
                continue;
            }
            if found || !entry.header().entry_type().is_file() {
                return Err(io::Error::other("duplicate or non-regular executable"));
            }
            copy_limited(&mut entry, &mut output, MAX_BINARY)?;
            found = true;
        }
        if !found {
            return Err(io::Error::other("executable missing from archive"));
        }
    }
    output.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Cursor, process::Command};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn release_archive(
        target: &str,
        payload: &[u8],
        regular: bool,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let name = asset_name("9.0.0", target)?;
        if let Some(base) = name.strip_suffix(".zip") {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default();
            if regular {
                zip.start_file(format!("{base}/solidcraft-kicad.exe"), options)?;
                zip.write_all(payload)?;
            } else {
                zip.add_symlink(format!("{base}/solidcraft-kicad.exe"), "elsewhere", options)?;
            }
            return Ok(zip.finish()?.into_inner());
        }
        let base = name
            .strip_suffix(".tar.gz")
            .ok_or("unexpected archive type")?;
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gzip);
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o755);
        if regular {
            header.set_size(payload.len() as u64);
        } else {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name("elsewhere")?;
            header.set_size(0);
        }
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("{base}/solidcraft-kicad"),
            if regular { payload } else { &[] },
        )?;
        Ok(tar.into_inner()?.finish()?)
    }

    fn fixture_download<'a>(
        name: &'a str,
        archive: &'a [u8],
    ) -> impl FnMut(&str, &mut File, u64) -> Result<(), String> + 'a {
        move |url, output, limit| {
            assert!(url.starts_with(&format!("{RELEASES}/v9.0.0/")));
            if url.ends_with("/SHA256SUMS") {
                writeln!(output, "{:x}  {name}", Sha256::digest(archive))
            } else {
                assert!(url.ends_with(name));
                assert_eq!(limit, MAX_ARCHIVE);
                output.write_all(archive)
            }
            .map_err(|error| error.to_string())
        }
    }

    #[test]
    fn verifies_and_extracts_each_release_platform_before_replacement() -> TestResult {
        for target in [
            "x86_64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-pc-windows-msvc",
        ] {
            let name = asset_name("9.0.0", target)?;
            let archive = release_archive(target, b"new executable", true)?;
            let mut replaced = false;
            install_with(
                "9.0.0",
                target,
                fixture_download(&name, &archive),
                |binary| {
                    assert_eq!(fs::read(binary)?, b"new executable");
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        assert_ne!(fs::metadata(binary)?.permissions().mode() & 0o111, 0);
                    }
                    replaced = true;
                    Ok(())
                },
            )?;
            assert!(replaced);
        }
        Ok(())
    }

    #[test]
    fn failed_download_checksum_or_archive_never_reaches_replacement() -> TestResult {
        let target = "x86_64-unknown-linux-gnu";
        let name = asset_name("9.0.0", target)?;
        for failure in ["download", "checksum", "archive", "missing", "symlink"] {
            let archive = match failure {
                "archive" => b"not a gzip archive".to_vec(),
                "symlink" => release_archive(target, b"", false)?,
                "missing" => {
                    let gzip =
                        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                    tar::Builder::new(gzip).into_inner()?.finish()?
                }
                _ => release_archive(target, b"new executable", true)?,
            };
            let mut replaced = false;
            let result = install_with(
                "9.0.0",
                target,
                |url, output, limit| {
                    if url.ends_with("/SHA256SUMS") {
                        fixture_download(&name, &archive)(url, output, limit)
                    } else if failure == "download" {
                        Err("interrupted download".into())
                    } else if failure == "checksum" {
                        output.write_all(b"damaged").map_err(|e| e.to_string())
                    } else {
                        fixture_download(&name, &archive)(url, output, limit)
                    }
                },
                |_| {
                    replaced = true;
                    Ok(())
                },
            );
            assert!(result.is_err(), "{failure}");
            assert!(!replaced, "{failure}");
        }
        let archive = release_archive("x86_64-pc-windows-msvc", b"", false)?;
        let mut archive_file = tempfile::tempfile()?;
        archive_file.write_all(&archive)?;
        archive_file.rewind()?;
        let dir = tempfile::tempdir()?;
        assert!(
            extract(
                archive_file,
                &asset_name("9.0.0", "x86_64-pc-windows-msvc")?,
                &dir.path().join("client.exe")
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn validates_checksum_manifest_limits_and_platform() -> TestResult {
        let hash = "a".repeat(64);
        assert_eq!(
            checksum(&format!("{hash} *client.zip\n"), "client.zip")?,
            hash
        );
        for manifest in [
            String::new(),
            "bad  client.zip".into(),
            format!("{hash}  other.zip"),
            format!("{hash}  client.zip\n{hash}  client.zip"),
        ] {
            assert!(checksum(&manifest, "client.zip").is_err());
        }
        assert!(asset_name("9.0.0", "aarch64-unknown-linux-gnu").is_err());
        assert!(asset_name("../bad", "x86_64-unknown-linux-gnu").is_err());
        assert!(copy_limited(&b""[..], &mut Vec::new(), 4).is_err());
        assert!(copy_limited(&b"12345"[..], &mut Vec::new(), 4).is_err());
        copy_limited(&b"1234"[..], &mut Vec::new(), 4)?;
        Ok(())
    }

    // Run the real replacement from a disposable copy of this test executable,
    // exercising the running-file lock without touching Cargo's test binary.
    #[test]
    fn replaces_its_own_running_binary() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let current = std::env::current_exe()?;
        let old = temporary.path().join(if cfg!(windows) {
            "client.exe"
        } else {
            "client"
        });
        fs::copy(&current, &old)?;
        #[cfg(not(target_os = "macos"))]
        File::options()
            .append(true)
            .open(&old)?
            .write_all(b"old binary marker")?;
        let status = Command::new(&old)
            .args([
                "--exact",
                "update::install::tests::replacement_child",
                "--nocapture",
            ])
            .env("SOLIDCRAFT_KICAD_TEST_REPLACEMENT", &current)
            .status()?;
        assert!(status.success());
        assert_eq!(fs::read(&old)?, fs::read(&current)?);
        assert!(Command::new(&old).arg("--list").output()?.status.success());
        Ok(())
    }

    #[test]
    fn replacement_child() -> TestResult {
        if let Some(replacement) = std::env::var_os("SOLIDCRAFT_KICAD_TEST_REPLACEMENT") {
            replace_binary(Path::new(&replacement))?;
        }
        Ok(())
    }
}
