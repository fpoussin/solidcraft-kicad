//! Background release checks and opt-in replacement of the running executable.
use std::{
    fs,
    io::{self, BufRead, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use semver::Version;
use serde::{Deserialize, Serialize};

mod install;

const INTERVAL: u64 = 24 * 60 * 60;
const DOWNLOAD: &str = "https://github.com/fpoussin/solidcraft-kicad/releases/latest";
const API: &str = "https://api.github.com/repos/fpoussin/solidcraft-kicad/releases/latest";

#[derive(Deserialize, Serialize)]
struct Cache {
    checked_at: u64,
    latest: Option<String>,
}

#[derive(Serialize)]
pub struct Report {
    current: String,
    latest: String,
    update_available: bool,
    download_url: &'static str,
}

impl Report {
    pub fn offer_update(&self) -> Result<(), String> {
        if self.update_available {
            offer_update(&self.latest)?;
        }
        Ok(())
    }

    pub fn message(&self) -> String {
        if self.update_available {
            format!(
                "Update available: solidcraft-kicad {} (current {}).\nDownload: {}",
                self.latest, self.current, self.download_url
            )
        } else {
            format!(
                "solidcraft-kicad {} is up to date (latest stable: {}).",
                self.current, self.latest
            )
        }
    }
}

/// Unlike background notifications, an explicit check waits and reports errors.
pub fn force() -> Result<Report, String> {
    force_at(
        cache_path().as_deref(),
        now(),
        env!("CARGO_PKG_VERSION"),
        fetch_latest,
    )
}

fn force_at(
    path: Option<&Path>,
    checked_at: u64,
    current: &str,
    fetch: impl FnOnce() -> Result<Version, String>,
) -> Result<Report, String> {
    let latest = fetch()?;
    let current_version = Version::parse(current).map_err(|e| e.to_string())?;
    if let Some(path) = path {
        let _ = write_cache(
            path,
            &Cache {
                checked_at,
                latest: Some(latest.to_string()),
            },
        );
    }
    Ok(Report {
        current: current.into(),
        latest: latest.to_string(),
        update_available: latest.cmp_precedence(&current_version).is_gt(),
        download_url: DOWNLOAD,
    })
}

fn cache_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "solidcraft-kicad")
        .map(|dirs| dirs.cache_dir().join("release-check.json"))
}

pub struct Check {
    cached: Option<Version>,
    pending: Option<Receiver<Option<Version>>>,
}

impl Check {
    pub fn start(enabled: bool) -> Self {
        let path = enabled.then(cache_path).flatten();
        match path {
            Some(path) => Self::start_at(path, now(), || fetch_latest().ok()),
            None => Self {
                cached: None,
                pending: None,
            },
        }
    }

    fn start_at(
        path: PathBuf,
        now: u64,
        fetch: impl FnOnce() -> Option<Version> + Send + 'static,
    ) -> Self {
        let cache = read_cache(&path);
        let cached = cache
            .as_ref()
            .and_then(|cache| cache.latest.as_deref())
            .and_then(stable_version);
        if cache.is_some_and(|cache| cache.checked_at <= now && now - cache.checked_at < INTERVAL) {
            return Self {
                cached,
                pending: None,
            };
        }
        let previous = cached.clone();
        // Record the attempt before starting HTTP so short commands and failed
        // requests do not cause another check on the next invocation.
        let _ = write_cache(
            &path,
            &Cache {
                checked_at: now,
                latest: previous.as_ref().map(ToString::to_string),
            },
        );
        let (send, receive) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let latest = fetch().or(previous);
                // Remember failed attempts too, without losing a known newer version.
                // Offline operation must not trigger a request on every invocation.
                let _ = write_cache(
                    &path,
                    &Cache {
                        checked_at: now,
                        latest: latest.as_ref().map(ToString::to_string),
                    },
                );
                let _ = send.send(latest);
            });
        Self {
            cached,
            pending: worker.ok().map(|_| receive),
        }
    }

    fn available(&self, current: &str) -> Option<Version> {
        let latest = self
            .pending
            .as_ref()
            .and_then(|pending| pending.try_recv().ok())
            .flatten()
            .or_else(|| self.cached.clone())?;
        let current = Version::parse(current).ok()?;
        latest.cmp_precedence(&current).is_gt().then_some(latest)
    }

    pub fn notify(self) {
        if let Some(latest) = self.available(env!("CARGO_PKG_VERSION")) {
            eprintln!(
                "Update available: solidcraft-kicad {latest} (current {}).\nDownload: {DOWNLOAD}",
                env!("CARGO_PKG_VERSION")
            );
            if let Err(error) = offer_update(&latest.to_string()) {
                // A completed conversion remains successful if the optional update fails.
                eprintln!("warning: update failed: {error}");
            }
        }
    }
}

fn offer_update(latest: &str) -> Result<(), String> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Ok(());
    }
    let target = env!("BUILD_TARGET");
    if install::asset_name(latest, target).is_err() {
        return Ok(());
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let confirmed = confirm(
        &mut io::stdin().lock(),
        &mut io::stderr().lock(),
        &executable,
    )
    .map_err(|error| error.to_string())?;
    if confirmed {
        eprintln!("Downloading solidcraft-kicad {latest}...");
        install::install(latest, target)?;
        eprintln!("Updated to solidcraft-kicad {latest}. The next invocation will use it.");
    }
    Ok(())
}

fn confirm(input: &mut impl BufRead, output: &mut impl Write, path: &Path) -> io::Result<bool> {
    write!(output, "Replace {} with the update? [y/N] ", path.display())?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn stable_version(tag: &str) -> Option<Version> {
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    version.pre.is_empty().then_some(version)
}

fn parse_release(body: &str) -> Option<Version> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        draft: bool,
        prerelease: bool,
    }
    let release: Release = serde_json::from_str(body).ok()?;
    if release.draft || release.prerelease {
        return None;
    }
    stable_version(&release.tag_name)
}

fn fetch_latest() -> Result<Version, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .https_only(true)
        .user_agent(concat!("solidcraft-kicad/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let body = agent
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| format!("cannot check GitHub for updates: {error}"))?
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|error| format!("cannot read GitHub release response: {error}"))?;
    parse_release(&body).ok_or_else(|| "GitHub returned no valid stable release".into())
}

fn read_cache(path: &Path) -> Option<Cache> {
    let mut data = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(4096)
        .read_to_end(&mut data)
        .ok()?;
    serde_json::from_slice(&data).ok()
}

fn write_cache(path: &Path, cache: &Cache) -> Option<()> {
    let parent = path.parent()?;
    fs::create_dir_all(parent).ok()?;
    let mut file = tempfile::NamedTempFile::new_in(parent).ok()?;
    serde_json::to_writer(&mut file, cache).ok()?;
    file.persist(path).ok()?;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn installation_requires_explicit_yes() -> io::Result<()> {
        for answer in ["", "\n", "n\n", "no\n", "sure\n", "yesterday\n"] {
            let mut output = Vec::new();
            assert!(!confirm(
                &mut answer.as_bytes(),
                &mut output,
                Path::new("client")
            )?);
            assert!(String::from_utf8_lossy(&output).ends_with("[y/N] "));
        }
        for answer in ["y\n", "Y\n", "yes\n", " YES \r\n"] {
            assert!(confirm(
                &mut answer.as_bytes(),
                &mut Vec::new(),
                Path::new("client")
            )?);
        }
        Ok(())
    }

    #[test]
    fn only_newer_stable_published_releases_are_notified() {
        for (tag, draft, prerelease, expected) in [
            ("v0.10.0", false, false, Some("0.10.0")),
            ("v0.9.0", false, false, None),
            ("v0.8.0", false, false, None),
            ("v0.9.0+build2", false, false, None),
            ("v1.0.0-beta.1", false, false, None),
            ("v1.0.0", true, false, None),
            ("v1.0.0", false, true, None),
            ("invalid", false, false, None),
        ] {
            let latest = parse_release(
                &serde_json::json!({
                    "tag_name": tag, "draft": draft, "prerelease": prerelease,
                })
                .to_string(),
            );
            let check = Check {
                cached: latest,
                pending: None,
            };
            assert_eq!(
                check.available("0.9.0").map(|v| v.to_string()).as_deref(),
                expected
            );
        }
        assert!(parse_release("not JSON").is_none());
        assert!(parse_release("{}").is_none());
    }

    #[test]
    fn forced_check_bypasses_fresh_cache_and_refreshes_it() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("release-check.json");
        write_cache(
            &path,
            &Cache {
                checked_at: 100,
                latest: Some("99.0.0".into()),
            },
        )
        .ok_or("cannot seed cache")?;
        let mut calls = 0;
        let report = force_at(Some(&path), 101, "0.2.0", || {
            calls += 1;
            Ok(Version::new(0, 3, 0))
        })?;
        assert_eq!(calls, 1);
        assert_eq!(report.latest, "0.3.0");
        assert!(report.update_available);
        let cache = read_cache(&path).ok_or("missing refreshed cache")?;
        assert_eq!(cache.checked_at, 101);
        assert_eq!(cache.latest.as_deref(), Some("0.3.0"));
        Ok(())
    }

    #[test]
    fn forced_check_reports_failure_instead_of_using_a_cached_release()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("release-check.json");
        write_cache(
            &path,
            &Cache {
                checked_at: 100,
                latest: Some("99.0.0".into()),
            },
        )
        .ok_or("cannot seed cache")?;
        let result = force_at(Some(&path), 101, "0.2.0", || Err("offline".into()));
        assert_eq!(result.err().as_deref(), Some("offline"));
        let cache = read_cache(&path).ok_or("missing previous cache")?;
        assert_eq!(cache.checked_at, 100);
        assert_eq!(cache.latest.as_deref(), Some("99.0.0"));
        Ok(())
    }

    #[test]
    fn forced_check_explains_current_status_and_ignores_build_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        for (latest, expected) in [
            ("0.3.0", true),
            ("0.2.0", false),
            ("0.2.0+build.1", false),
            ("0.1.0", false),
        ] {
            let report = force_at(None, 100, "0.2.0", || {
                Version::parse(latest).map_err(|e| e.to_string())
            })?;
            assert_eq!(report.update_available, expected);
            assert!(report.message().contains(if expected {
                "Update available"
            } else {
                "up to date"
            }));
        }
        Ok(())
    }

    #[test]
    fn fresh_cache_skips_the_network() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("release-check.json");
        assert!(
            write_cache(
                &path,
                &Cache {
                    checked_at: 100,
                    latest: Some("0.2.0".into())
                }
            )
            .is_some()
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let fetched = calls.clone();
        let check = Check::start_at(path, 101, move || {
            fetched.fetch_add(1, Ordering::SeqCst);
            None
        });
        assert!(check.pending.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            check.available("0.1.0").map(|v| v.to_string()).as_deref(),
            Some("0.2.0")
        );
        Ok(())
    }

    #[test]
    fn slow_check_never_blocks_completion_and_caches_the_result()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("release-check.json");
        let (unblock, blocked) = mpsc::channel();
        let check = Check::start_at(path.clone(), 100, move || {
            blocked.recv().ok()?;
            stable_version("0.2.0")
        });
        assert!(check.available("0.1.0").is_none());
        assert_eq!(read_cache(&path).ok_or("missing attempt")?.checked_at, 100);
        unblock.send(())?;
        let latest = check
            .pending
            .as_ref()
            .ok_or("missing worker")?
            .recv_timeout(Duration::from_secs(2))?;
        assert_eq!(latest.map(|v| v.to_string()).as_deref(), Some("0.2.0"));
        assert_eq!(
            read_cache(&path).ok_or("missing cache")?.latest.as_deref(),
            Some("0.2.0")
        );
        Ok(())
    }

    #[test]
    fn failed_refresh_keeps_known_release_and_throttles_retries()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("release-check.json");
        assert!(
            write_cache(
                &path,
                &Cache {
                    checked_at: 1,
                    latest: Some("0.2.0".into())
                }
            )
            .is_some()
        );
        let now = INTERVAL + 1;
        let (unblock, blocked) = mpsc::channel();
        let check = Check::start_at(path.clone(), now, move || {
            blocked.recv().ok()?;
            None
        });
        assert_eq!(
            check.available("0.1.0").map(|v| v.to_string()).as_deref(),
            Some("0.2.0")
        );
        unblock.send(())?;
        let _ = check
            .pending
            .as_ref()
            .ok_or("missing worker")?
            .recv_timeout(Duration::from_secs(2))?;
        let cache = read_cache(&path).ok_or("missing cache")?;
        assert_eq!(cache.checked_at, now);
        assert_eq!(cache.latest.as_deref(), Some("0.2.0"));
        Ok(())
    }

    #[test]
    fn disabled_checks_do_nothing() {
        let check = Check::start(false);
        assert!(check.cached.is_none() && check.pending.is_none());
        check.notify();
    }
}
